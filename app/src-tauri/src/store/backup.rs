//! Export/import, so users can move to a new computer. Three formats; the
//! user picks one:
//!
//! 1. Encrypted backup (`.autologin`): everything including secrets, locked
//!    with a password (Argon2id + XChaCha20-Poly1305). Recommended.
//! 2. Plain export (`.json`): everything including secrets, unencrypted. The
//!    UI warns and asks for confirmation.
//! 3. CSV: account details without any secrets.
//!
//! Import detects the format and asks for the password when needed.

use std::collections::BTreeMap;

use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroizing;

pub const ENCRYPTED_FORMAT: &str = "autologin-backup-encrypted";
pub const PLAIN_FORMAT: &str = "autologin-backup-plain";
pub const BACKUP_VERSION: u32 = 1;
pub const MIN_PASSWORD_CHARS: usize = 8;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
/// OWASP-recommended Argon2id settings: 64 MiB, 3 passes, 1 lane.
const KDF_MEMORY_KIB: u32 = 64 * 1024;
const KDF_ITERATIONS: u32 = 3;
const KDF_PARALLELISM: u32 = 1;
/// Upper bounds accepted when *reading* a backup. The file states its own
/// key-derivation settings; without limits a crafted file could make import
/// allocate gigabytes or run for hours. Generous headroom over our defaults.
const MAX_KDF_MEMORY_KIB: u32 = 256 * 1024;
const MAX_KDF_ITERATIONS: u32 = 10;
const MAX_KDF_PARALLELISM: u32 = 8;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BackupError {
    #[error("Choose a password of at least {MIN_PASSWORD_CHARS} characters.")]
    WeakPassword,
    #[error("Wrong password, or the backup file is damaged.")]
    WrongPassword,
    #[error("This backup needs a password.")]
    PasswordRequired,
    #[error("This isn't an AutoLogin backup or CSV file.")]
    UnknownFormat,
    #[error("This backup was made by a newer AutoLogin (format {0}); update the app first.")]
    NewerVersion(u32),
    #[error("The backup file is damaged: {0}")]
    Corrupt(String),
}

/// One account as it travels between computers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupAccount {
    pub tenant_id: String,
    pub broker_id: String,
    pub client_id: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupData {
    pub version: u32,
    pub exported_at: DateTime<Utc>,
    pub app_version: String,
    pub accounts: Vec<BackupAccount>,
}

#[derive(Serialize, Deserialize)]
struct KdfParams {
    algorithm: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: String,
}

#[derive(Serialize, Deserialize)]
struct EncryptedFile {
    format: String,
    version: u32,
    kdf: KdfParams,
    cipher: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
struct PlainFile {
    format: String,
    #[serde(flatten)]
    data: BackupData,
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::getrandom(&mut bytes).expect("OS random number generator is available");
    bytes
}

fn derive_key(password: &str, salt: &[u8], memory_kib: u32, iterations: u32, parallelism: u32) -> Result<Zeroizing<[u8; KEY_LEN]>, BackupError> {
    let params = Params::new(memory_kib, iterations, parallelism, Some(KEY_LEN))
        .map_err(|e| BackupError::Corrupt(format!("key settings: {e}")))?;
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|e| BackupError::Corrupt(format!("key derivation: {e}")))?;
    Ok(key)
}

pub fn export_encrypted(data: &BackupData, password: &str) -> Result<String, BackupError> {
    export_encrypted_with(data, password, KDF_MEMORY_KIB, KDF_ITERATIONS)
}

fn export_encrypted_with(data: &BackupData, password: &str, memory_kib: u32, iterations: u32) -> Result<String, BackupError> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(BackupError::WeakPassword);
    }
    let salt = random_bytes::<SALT_LEN>();
    let nonce = random_bytes::<NONCE_LEN>();
    let key = derive_key(password, &salt, memory_kib, iterations, KDF_PARALLELISM)?;
    let plaintext = Zeroizing::new(serde_json::to_vec(data).expect("backup data always serializes"));
    let ciphertext = XChaCha20Poly1305::new(key.as_ref().into())
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_slice())
        .map_err(|_| BackupError::Corrupt("encryption failed".into()))?;

    let file = EncryptedFile {
        format: ENCRYPTED_FORMAT.into(),
        version: BACKUP_VERSION,
        kdf: KdfParams {
            algorithm: "argon2id".into(),
            memory_kib,
            iterations,
            parallelism: KDF_PARALLELISM,
            salt: B64.encode(salt),
        },
        cipher: "xchacha20poly1305".into(),
        nonce: B64.encode(nonce),
        ciphertext: B64.encode(ciphertext),
    };
    Ok(serde_json::to_string_pretty(&file).expect("backup file always serializes"))
}

pub fn export_plain(data: &BackupData) -> String {
    let file = PlainFile { format: PLAIN_FORMAT.into(), data: data.clone() };
    serde_json::to_string_pretty(&file).expect("backup file always serializes")
}

/// CSV of non-secret details: tenant, broker, client_id, then one column per
/// non-secret field used by any exported account.
pub fn export_csv(accounts: &[BackupAccount]) -> String {
    let field_columns: Vec<String> = accounts
        .iter()
        .flat_map(|a| a.fields.keys().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut writer = csv::Writer::from_writer(Vec::new());
    let header = ["tenant", "broker", "client_id"].into_iter().map(str::to_string).chain(field_columns.clone());
    writer.write_record(header).expect("in-memory write");
    for account in accounts {
        let row = [account.tenant_id.clone(), account.broker_id.clone(), account.client_id.clone()]
            .into_iter()
            .chain(field_columns.iter().map(|c| account.fields.get(c).cloned().unwrap_or_default()));
        writer.write_record(row).expect("in-memory write");
    }
    String::from_utf8(writer.into_inner().expect("in-memory flush")).expect("CSV of UTF-8 strings is UTF-8")
}

/// What an import file turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Encrypted,
    Plain,
    Csv,
}

pub fn detect(text: &str) -> Result<FileKind, BackupError> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        return match value.get("format").and_then(|f| f.as_str()) {
            Some(ENCRYPTED_FORMAT) => Ok(FileKind::Encrypted),
            Some(PLAIN_FORMAT) => Ok(FileKind::Plain),
            _ => Err(BackupError::UnknownFormat),
        };
    }
    let first_line = text.lines().next().unwrap_or_default().to_ascii_lowercase();
    if first_line.contains("client") && first_line.contains(',') {
        Ok(FileKind::Csv)
    } else {
        Err(BackupError::UnknownFormat)
    }
}

/// Read an encrypted or plain backup. CSV goes through `csv_import`.
pub fn read_backup(text: &str, password: Option<&str>) -> Result<BackupData, BackupError> {
    let data = match detect(text)? {
        FileKind::Plain => {
            serde_json::from_str::<PlainFile>(text).map_err(|e| BackupError::Corrupt(e.to_string()))?.data
        }
        FileKind::Encrypted => decrypt(text, password.ok_or(BackupError::PasswordRequired)?)?,
        FileKind::Csv => return Err(BackupError::UnknownFormat),
    };
    if data.version > BACKUP_VERSION {
        return Err(BackupError::NewerVersion(data.version));
    }
    Ok(data)
}

fn decrypt(text: &str, password: &str) -> Result<BackupData, BackupError> {
    let file: EncryptedFile = serde_json::from_str(text).map_err(|e| BackupError::Corrupt(e.to_string()))?;
    if file.version > BACKUP_VERSION {
        return Err(BackupError::NewerVersion(file.version));
    }
    if file.kdf.algorithm != "argon2id" || file.cipher != "xchacha20poly1305" {
        return Err(BackupError::Corrupt("unsupported encryption".into()));
    }
    let kdf = &file.kdf;
    if kdf.memory_kib > MAX_KDF_MEMORY_KIB
        || kdf.iterations > MAX_KDF_ITERATIONS
        || kdf.parallelism == 0
        || kdf.parallelism > MAX_KDF_PARALLELISM
    {
        return Err(BackupError::Corrupt("unsupported key settings".into()));
    }
    let decode = |field: &str| B64.decode(field).map_err(|_| BackupError::Corrupt("bad encoding".into()));
    let salt = decode(&file.kdf.salt)?;
    let nonce = decode(&file.nonce)?;
    if nonce.len() != NONCE_LEN {
        return Err(BackupError::Corrupt("bad nonce".into()));
    }
    let key = derive_key(password, &salt, file.kdf.memory_kib, file.kdf.iterations, file.kdf.parallelism)?;
    let plaintext = Zeroizing::new(
        XChaCha20Poly1305::new(key.as_ref().into())
            .decrypt(XNonce::from_slice(&nonce), decode(&file.ciphertext)?.as_slice())
            .map_err(|_| BackupError::WrongPassword)?,
    );
    serde_json::from_slice(&plaintext).map_err(|e| BackupError::Corrupt(e.to_string()))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    /// Light KDF settings so tests run fast; production uses the constants.
    const TEST_MEMORY_KIB: u32 = 1024;
    const TEST_ITERATIONS: u32 = 1;

    fn data() -> BackupData {
        BackupData {
            version: BACKUP_VERSION,
            exported_at: Utc.with_ymd_and_hms(2026, 10, 5, 9, 0, 0).unwrap(),
            app_version: "2.0.0".into(),
            accounts: vec![BackupAccount {
                tenant_id: "cirrus".into(),
                broker_id: "zerodha".into(),
                client_id: "AB1".into(),
                fields: [("api_key".to_string(), "kite".to_string())].into(),
                secrets: [("password".to_string(), "s3cret-pw".to_string())].into(),
            }],
        }
    }

    fn encrypted(password: &str) -> String {
        export_encrypted_with(&data(), password, TEST_MEMORY_KIB, TEST_ITERATIONS).unwrap()
    }

    #[test]
    fn encrypted_round_trip_and_secret_not_visible() {
        let file = encrypted("correct horse");
        assert!(!file.contains("s3cret-pw") && !file.contains("AB1"));
        assert_eq!(detect(&file), Ok(FileKind::Encrypted));
        assert_eq!(read_backup(&file, Some("correct horse")).unwrap(), data());
    }

    #[test]
    fn wrong_or_missing_password_is_rejected() {
        let file = encrypted("correct horse");
        assert_eq!(read_backup(&file, Some("wrong horse")), Err(BackupError::WrongPassword));
        assert_eq!(read_backup(&file, None), Err(BackupError::PasswordRequired));
    }

    #[test]
    fn tampered_ciphertext_fails_authentication() {
        let mut file: serde_json::Value = serde_json::from_str(&encrypted("correct horse")).unwrap();
        let mut bytes = B64.decode(file["ciphertext"].as_str().unwrap()).unwrap();
        bytes[0] ^= 1;
        file["ciphertext"] = B64.encode(bytes).into();
        assert_eq!(read_backup(&file.to_string(), Some("correct horse")), Err(BackupError::WrongPassword));
    }

    #[test]
    fn each_export_uses_fresh_salt_and_nonce() {
        assert_ne!(encrypted("correct horse"), encrypted("correct horse"));
    }

    #[test]
    fn rejects_backups_asking_for_excessive_key_derivation_work() {
        // A crafted file could otherwise make import allocate gigabytes or spin.
        let base: serde_json::Value = serde_json::from_str(&encrypted("correct horse")).unwrap();
        for (field, value) in [("memory_kib", 64 * 1024 * 1024), ("iterations", 1_000_000), ("parallelism", 64)] {
            let mut file = base.clone();
            file["kdf"][field] = value.into();
            let result = read_backup(&file.to_string(), Some("correct horse"));
            assert!(matches!(result, Err(BackupError::Corrupt(_))), "{field}: {result:?}");
        }
    }

    #[test]
    fn short_password_is_refused() {
        assert_eq!(export_encrypted(&data(), "short"), Err(BackupError::WeakPassword));
    }

    #[test]
    fn plain_round_trip() {
        let file = export_plain(&data());
        assert_eq!(detect(&file), Ok(FileKind::Plain));
        assert_eq!(read_backup(&file, None).unwrap(), data());
    }

    #[test]
    fn csv_has_no_secrets() {
        let csv = export_csv(&data().accounts);
        assert_eq!(csv, "tenant,broker,client_id,api_key\ncirrus,zerodha,AB1,kite\n");
        assert_eq!(detect(&csv), Ok(FileKind::Csv));
    }

    #[test]
    fn newer_backups_and_junk_are_rejected() {
        let mut newer = data();
        newer.version = BACKUP_VERSION + 1;
        assert_eq!(read_backup(&export_plain(&newer), None), Err(BackupError::NewerVersion(2)));
        assert_eq!(detect("hello world"), Err(BackupError::UnknownFormat));
        assert_eq!(detect(r#"{"format":"other"}"#), Err(BackupError::UnknownFormat));
    }
}
