//! Account secrets encrypted in the database with one key from the keychain.
//!
//! macOS ties "Always Allow" to the exact app binary, so every update (and
//! every dev rebuild) asks again for each keychain item. With one item per
//! account that meant many prompts; here there is exactly one item (a random
//! 256-bit key), read once per launch and cached in memory. Secrets are
//! encrypted with XChaCha20-Poly1305, bound to their account by using the
//! account's entry name as associated data.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroizing;

use super::secrets::{AccountKey, SecretError, SecretStore, Secrets};
#[cfg(not(target_os = "android"))]
use super::secrets::KEYCHAIN_SERVICE;

#[cfg(not(target_os = "android"))]
const MASTER_KEY_ENTRY: &str = "vault-key";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

pub type MasterKeyBytes = Zeroizing<[u8; KEY_LEN]>;

/// Where the vault's master key lives.
pub trait MasterKey: Send + Sync {
    fn get_or_create(&self) -> Result<MasterKeyBytes, SecretError>;
}

/// The master key as a single OS keychain item (hex encoded).
#[cfg(not(target_os = "android"))]
#[derive(Debug, Default)]
pub struct KeychainMasterKey;

#[cfg(not(target_os = "android"))]
impl MasterKey for KeychainMasterKey {
    fn get_or_create(&self) -> Result<MasterKeyBytes, SecretError> {
        let keychain_error = |e: keyring::Error| SecretError::Keychain(e.to_string());
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, MASTER_KEY_ENTRY).map_err(keychain_error)?;
        match entry.get_password() {
            Ok(hex_key) => decode_key(&Zeroizing::new(hex_key)),
            Err(keyring::Error::NoEntry) => {
                let key = random_key();
                entry.set_password(&Zeroizing::new(hex::encode(key.as_ref()))).map_err(keychain_error)?;
                tracing::info!("created the AutoLogin vault key in the keychain");
                Ok(key)
            }
            Err(error) => Err(keychain_error(error)),
        }
    }
}

/// The master key in a file only this app can read. Used on Android until a
/// Keystore-backed implementation replaces it.
#[derive(Debug)]
pub struct FileMasterKey {
    path: std::path::PathBuf,
}

impl FileMasterKey {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }
}

impl MasterKey for FileMasterKey {
    fn get_or_create(&self) -> Result<MasterKeyBytes, SecretError> {
        let storage = |e: std::io::Error| SecretError::Storage(e.to_string());
        match std::fs::read_to_string(&self.path) {
            Ok(hex_key) => decode_key(&Zeroizing::new(hex_key)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let key = random_key();
                write_private(&self.path, &hex::encode(key.as_ref())).map_err(storage)?;
                Ok(key)
            }
            Err(error) => Err(storage(error)),
        }
    }
}

fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

fn decode_key(hex_key: &str) -> Result<MasterKeyBytes, SecretError> {
    let bytes = Zeroizing::new(hex::decode(hex_key.trim()).map_err(|_| SecretError::Corrupt)?);
    let array: [u8; KEY_LEN] = bytes.as_slice().try_into().map_err(|_| SecretError::Corrupt)?;
    Ok(Zeroizing::new(array))
}

fn random_key() -> MasterKeyBytes {
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    getrandom::getrandom(key.as_mut()).expect("OS random number generator is available");
    key
}

pub struct VaultStore {
    master: Box<dyn MasterKey>,
    cached: Mutex<Option<MasterKeyBytes>>,
    key_reads: AtomicUsize,
}

impl VaultStore {
    pub fn new(master: Box<dyn MasterKey>) -> Self {
        Self { master, cached: Mutex::new(None), key_reads: AtomicUsize::new(0) }
    }

    fn cipher(&self) -> Result<XChaCha20Poly1305, SecretError> {
        let mut cached = self.cached.lock().map_err(|_| SecretError::Storage("vault lock poisoned".into()))?;
        if cached.is_none() {
            self.key_reads.fetch_add(1, Ordering::Relaxed);
            *cached = Some(self.master.get_or_create()?);
        }
        let key = cached.as_ref().expect("set above");
        Ok(XChaCha20Poly1305::new(key.as_ref().into()))
    }

    /// How many times the keychain was asked for the master key (tests).
    pub fn key_reads(&self) -> usize {
        self.key_reads.load(Ordering::Relaxed)
    }

    fn read_vault(&self, conn: &Connection, key: &AccountKey) -> Result<Option<Secrets>, SecretError> {
        let entry = key.entry_name();
        let row: Option<(Vec<u8>, Vec<u8>)> = conn
            .query_row("SELECT nonce, ciphertext FROM secrets WHERE entry = ?1", [&entry], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(|e| SecretError::Storage(e.to_string()))?;
        let Some((nonce, ciphertext)) = row else { return Ok(None) };
        if nonce.len() != NONCE_LEN {
            return Err(SecretError::Corrupt);
        }
        let plaintext = Zeroizing::new(
            self.cipher()?
                .decrypt(XNonce::from_slice(&nonce), Payload { msg: &ciphertext, aad: entry.as_bytes() })
                .map_err(|_| SecretError::Corrupt)?,
        );
        serde_json::from_slice(&plaintext).map(Some).map_err(|_| SecretError::Corrupt)
    }
}

impl SecretStore for VaultStore {
    fn load(&self, conn: &Connection, key: &AccountKey) -> Result<Secrets, SecretError> {
        Ok(self.read_vault(conn, key)?.unwrap_or_default())
    }

    fn save(&self, conn: &Connection, key: &AccountKey, secrets: &Secrets) -> Result<(), SecretError> {
        if secrets.is_empty() {
            return self.delete(conn, key);
        }
        let entry = key.entry_name();
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::getrandom(&mut nonce).expect("OS random number generator is available");
        let plaintext = Zeroizing::new(serde_json::to_vec(secrets).expect("string map always serializes"));
        let ciphertext = self
            .cipher()?
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: &plaintext, aad: entry.as_bytes() })
            .map_err(|_| SecretError::Storage("encryption failed".into()))?;
        conn.execute(
            "INSERT INTO secrets (entry, nonce, ciphertext) VALUES (?1, ?2, ?3)
             ON CONFLICT(entry) DO UPDATE SET nonce = excluded.nonce, ciphertext = excluded.ciphertext",
            params![entry, nonce.to_vec(), ciphertext],
        )
        .map_err(|e| SecretError::Storage(e.to_string()))?;
        Ok(())
    }

    fn delete(&self, conn: &Connection, key: &AccountKey) -> Result<(), SecretError> {
        conn.execute("DELETE FROM secrets WHERE entry = ?1", [key.entry_name()])
            .map_err(|e| SecretError::Storage(e.to_string()))?;
        Ok(())
    }

    fn unlock(&self) -> Result<(), SecretError> {
        self.cipher().map(|_| ())
    }

    fn is_unlocked(&self) -> bool {
        self.cached.lock().is_ok_and(|key| key.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db;

    struct FixedKey([u8; KEY_LEN]);

    impl MasterKey for FixedKey {
        fn get_or_create(&self) -> Result<MasterKeyBytes, SecretError> {
            Ok(Zeroizing::new(self.0))
        }
    }

    fn vault(key: u8) -> VaultStore {
        VaultStore::new(Box::new(FixedKey([key; KEY_LEN])))
    }

    fn account(client: &str) -> AccountKey {
        AccountKey { tenant_id: "cirrus".into(), broker_id: "zerodha".into(), client_id: client.into() }
    }

    fn secrets() -> Secrets {
        [("password".to_string(), "pw-SECRET".to_string())].into()
    }

    #[test]
    fn file_master_key_is_created_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.key");
        let first = FileMasterKey::new(path.clone()).get_or_create().unwrap();
        let second = FileMasterKey::new(path.clone()).get_or_create().unwrap();
        assert_eq!(first.as_ref(), second.as_ref());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn round_trips_and_stores_only_ciphertext() {
        let conn = db::open_in_memory().unwrap();
        let store = vault(1);
        store.save(&conn, &account("A"), &secrets()).unwrap();
        assert_eq!(store.load(&conn, &account("A")).unwrap(), secrets());
        let raw: Vec<u8> = conn.query_row("SELECT ciphertext FROM secrets", [], |r| r.get(0)).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("pw-SECRET"));
    }

    #[test]
    fn reads_the_master_key_once_for_many_accounts() {
        let conn = db::open_in_memory().unwrap();
        let store = vault(1);
        for client in ["A", "B", "C", "D"] {
            store.save(&conn, &account(client), &secrets()).unwrap();
            store.load(&conn, &account(client)).unwrap();
        }
        assert_eq!(store.key_reads(), 1);
    }

    #[test]
    fn unlock_reads_the_key_once_up_front() {
        let store = vault(1);
        assert!(!store.is_unlocked());
        store.unlock().unwrap();
        assert!(store.is_unlocked());
        store.unlock().unwrap();
        assert_eq!(store.key_reads(), 1);
    }

    #[test]
    fn wrong_key_or_moved_ciphertext_is_rejected() {
        let conn = db::open_in_memory().unwrap();
        vault(1).save(&conn, &account("A"), &secrets()).unwrap();
        assert!(matches!(vault(2).load(&conn, &account("A")), Err(SecretError::Corrupt)));

        // Copying A's ciphertext onto B's entry must not decrypt (bound by AAD).
        conn.execute("INSERT INTO secrets (entry, nonce, ciphertext) SELECT 'cirrus:zerodha:B', nonce, ciphertext FROM secrets", [])
            .unwrap();
        assert!(matches!(vault(1).load(&conn, &account("B")), Err(SecretError::Corrupt)));
    }

    #[test]
    fn empty_save_deletes() {
        let conn = db::open_in_memory().unwrap();
        let store = vault(1);
        store.save(&conn, &account("A"), &secrets()).unwrap();
        store.save(&conn, &account("A"), &Secrets::new()).unwrap();
        assert!(store.load(&conn, &account("A")).unwrap().is_empty());
    }
}
