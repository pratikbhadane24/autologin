//! Automatic import of AutoLogin 1.x (Python) data on first launch.
//!
//! v1 kept plaintext `accounts.json` + `preferences.json` in
//! `platformdirs.user_data_dir("AutoLogin")/data`. We import what v2
//! supports, move secrets into the keychain, and rename the file to
//! `accounts.json.v1-migrated.bak` (never deleted). Brokers v2 doesn't
//! support yet are imported automatically on a later launch, once a broker
//! update adds them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::broker::registry::ManifestBundle;
use crate::store::accounts::Accounts;
use crate::store::backup::BackupAccount;
use crate::store::settings;
use crate::store::transfer::{self, ImportReport};

pub const ACCOUNTS_FILE: &str = "accounts.json";
pub const BACKUP_FILE: &str = "accounts.json.v1-migrated.bak";
const PREFERENCES_FILE: &str = "preferences.json";
const STATE_KEY: &str = "migration.v1";
/// Display names for v1 broker keys (v1 is frozen, so this list is too).
const V1_BROKER_NAMES: &[(&str, &str)] = &[
    ("angel_one", "Angel One"),
    ("zerodha", "Zerodha"),
    ("upstox", "Upstox"),
    ("sharekhan", "Sharekhan"),
    ("motilal", "Motilal Oswal"),
    ("nuvama", "Nuvama"),
    ("jainamlite", "Jainam Lite"),
    ("kotakneo", "Kotak Neo"),
    ("fyers", "Fyers"),
    ("fivepaisa", "5Paisa"),
    ("dhan", "Dhan"),
    ("firstock", "Firstock"),
    ("pocketful", "Pocketful"),
];

fn v1_display_name(key: &str) -> String {
    V1_BROKER_NAMES.iter().find(|(k, _)| *k == key).map_or_else(|| key.to_string(), |(_, name)| name.to_string())
}

/// v1's per-account bookkeeping, not account fields.
const V1_META_KEYS: &[&str] = &["added_on", "last_login", "status", "client_id"];

#[derive(Debug, Default, Serialize, Deserialize)]
struct MigrationState {
    /// v1 broker keys already imported.
    migrated_brokers: BTreeSet<String>,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct MigrationOutcome {
    pub report: ImportReport,
    /// Names of v1 brokers whose accounts are waiting for v2 support.
    pub waiting_brokers: Vec<String>,
    /// v1 "Background Login" preference, to seed the show/hide browser default.
    pub background_login: Option<bool>,
}

/// Overrides the v1 folder, for development and testing against fake data.
pub const V1_DIR_ENV: &str = "AUTOLOGIN_V1_DIR";

/// v1's data folder, resolved the way Python's platformdirs does.
pub fn v1_data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(V1_DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    let base = if cfg!(target_os = "windows") {
        // platformdirs on Windows: %LOCALAPPDATA%\<author>\<app>, author = app name.
        dirs::data_local_dir()?.join("AutoLogin").join("AutoLogin")
    } else {
        dirs::data_dir()?.join("AutoLogin")
    };
    Some(base.join("data"))
}

/// Run (or resume) the migration from `v1_dir`. Returns `None` when there is
/// nothing to do.
pub fn run(
    v1_dir: &Path,
    conn: &Connection,
    accounts: &Accounts<'_>,
    bundle: &ManifestBundle,
) -> Result<Option<MigrationOutcome>, rusqlite::Error> {
    let original = v1_dir.join(ACCOUNTS_FILE);
    let backup = v1_dir.join(BACKUP_FILE);
    let source = if original.exists() { &original } else { &backup };
    let Some(v1) = read_v1_accounts(source) else { return Ok(None) };

    let mut state: MigrationState = settings::get(conn, STATE_KEY)?.unwrap_or_default();
    let mut entries = Vec::new();
    let mut waiting = Vec::new();
    let mut newly_migrated = Vec::new();
    for (v1_broker, records) in &v1 {
        if state.migrated_brokers.contains(v1_broker) {
            continue;
        }
        match bundle.resolve_alias(v1_broker) {
            Some(manifest) => {
                entries.extend(records.iter().filter_map(|r| to_backup_account(r, &manifest.id, bundle)));
                newly_migrated.push(v1_broker.clone());
            }
            // Only brokers that actually have accounts are worth mentioning.
            None if !records.is_empty() => waiting.push(v1_display_name(v1_broker)),
            None => {}
        }
    }
    if newly_migrated.is_empty() && !original.exists() {
        return Ok(None);
    }

    // v2's copy of an account is newer than v1's: never overwrite it.
    let entries: Vec<BackupAccount> = entries
        .into_iter()
        .filter(|entry| {
            let key = crate::store::secrets::AccountKey {
                tenant_id: entry.tenant_id.clone(),
                broker_id: entry.broker_id.clone(),
                client_id: entry.client_id.clone(),
            };
            !matches!(accounts.find(&key), Ok(Some(_)))
        })
        .collect();
    let report = transfer::apply(accounts, bundle, &entries);
    state.migrated_brokers.extend(newly_migrated);
    settings::set(conn, STATE_KEY, &state)?;

    if original.exists() {
        if let Err(error) = std::fs::rename(&original, &backup) {
            tracing::warn!(%error, "could not rename v1 accounts.json; it will be re-read next launch");
        }
    }
    tracing::info!(
        added = report.added,
        updated = report.updated,
        waiting = waiting.len(),
        "v1 migration step finished"
    );
    Ok(Some(MigrationOutcome { report, waiting_brokers: waiting, background_login: read_background_pref(v1_dir) }))
}

type V1Accounts = BTreeMap<String, Vec<BTreeMap<String, Value>>>;

fn read_v1_accounts(path: &Path) -> Option<V1Accounts> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(accounts) => Some(accounts),
        Err(error) => {
            tracing::warn!(%error, "v1 accounts.json is unreadable; skipping migration");
            None
        }
    }
}

fn to_backup_account(record: &BTreeMap<String, Value>, broker_id: &str, bundle: &ManifestBundle) -> Option<BackupAccount> {
    let manifest = bundle.get(broker_id)?;
    let text = |v: &Value| match v {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    };
    let client_id = record.get("client_id").and_then(text).filter(|c| !c.is_empty())?;
    let mut fields = BTreeMap::new();
    let mut secrets = BTreeMap::new();
    for (key, value) in record {
        if V1_META_KEYS.contains(&key.as_str()) {
            continue;
        }
        let (Some(field), Some(value)) = (manifest.field(key), text(value)) else { continue };
        if value.is_empty() {
            continue;
        }
        let target = if field.secret { &mut secrets } else { &mut fields };
        target.insert(key.clone(), value);
    }
    Some(BackupAccount {
        tenant_id: bundle.default_tenant().to_string(),
        broker_id: broker_id.to_string(),
        client_id,
        fields,
        secrets,
    })
}

fn read_background_pref(v1_dir: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(v1_dir.join(PREFERENCES_FILE)).ok()?;
    serde_json::from_str::<Value>(&text).ok()?.get("background_login")?.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db;
    use crate::store::secrets::{MemoryStore, SecretStore};

    const V1_ACCOUNTS: &str = r#"{
      "zerodha": [{"client_id": "AB1", "api_key": "kite", "password": "pw", "totp_key": "JBSWY3DPEHPK3PXP",
                   "api_secret": "", "added_on": "13-07-2026 09:29", "last_login": "", "status": "Logged In"}],
      "kotakneo": [{"client_id": "K1", "mpin": "1234", "totp_key": "JBSWY3DPEHPK3PXP", "mobile_number": "9999999999"}],
      "pocketful": [{"client_id": "P1", "password": "pw2", "mpin": ""}],
      "nuvama": []
    }"#;

    fn setup(dir: &Path) {
        std::fs::write(dir.join(ACCOUNTS_FILE), V1_ACCOUNTS).unwrap();
        std::fs::write(dir.join(PREFERENCES_FILE), r#"{"background_login": true}"#).unwrap();
    }

    #[test]
    fn imports_supported_brokers_keeps_backup_and_remembers_progress() {
        let dir = tempfile::tempdir().unwrap();
        setup(dir.path());
        let conn = db::open_in_memory().unwrap();
        let secrets = MemoryStore::default();
        let bundle = ManifestBundle::bundled().unwrap();
        let accounts = Accounts::new(&conn, &secrets, &bundle);

        let outcome = run(dir.path(), &conn, &accounts, &bundle).unwrap().unwrap();

        assert_eq!(outcome.report.added, 2);
        assert_eq!(outcome.waiting_brokers, vec!["Kotak Neo".to_string()]);
        assert_eq!(outcome.background_login, Some(true));
        assert!(!dir.path().join(ACCOUNTS_FILE).exists());
        assert!(dir.path().join(BACKUP_FILE).exists(), "v1 data must never be deleted");

        let zerodha = accounts.list().unwrap().into_iter().find(|a| a.broker_id == "zerodha").unwrap();
        assert_eq!(secrets.load(&conn, &zerodha.key()).unwrap()["password"], "pw");

        // Second launch: nothing new to import, no duplicates.
        assert_eq!(run(dir.path(), &conn, &accounts, &bundle).unwrap(), None);
        assert_eq!(accounts.list().unwrap().len(), 2);
    }

    #[test]
    fn brokers_supported_later_are_imported_from_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        setup(dir.path());
        let conn = db::open_in_memory().unwrap();
        let secrets = MemoryStore::default();
        let bundle = ManifestBundle::bundled().unwrap();
        run(dir.path(), &conn, &Accounts::new(&conn, &secrets, &bundle), &bundle).unwrap();

        // Simulate a broker update that adds Kotak Neo.
        let mut newer = bundle.clone();
        let mut kotak = newer.get("pocketful").unwrap().clone();
        kotak.id = "kotakneo".into();
        kotak.name = "Kotak Neo".into();
        kotak.aliases = vec!["kotakneo".into()];
        newer.brokers.insert("kotakneo".into(), kotak);

        let accounts = Accounts::new(&conn, &secrets, &newer);
        let outcome = run(dir.path(), &conn, &accounts, &newer).unwrap().unwrap();
        assert_eq!(outcome.report.added, 1);
        assert!(outcome.waiting_brokers.is_empty());
    }

    #[test]
    fn never_overwrites_an_account_already_in_v2() {
        use crate::store::accounts::AccountInput;
        use crate::store::validate::Completeness;
        let dir = tempfile::tempdir().unwrap();
        setup(dir.path());
        let conn = db::open_in_memory().unwrap();
        let secrets = MemoryStore::default();
        let bundle = ManifestBundle::bundled().unwrap();
        let accounts = Accounts::new(&conn, &secrets, &bundle);
        // The user already added P1 in v2 with a newer password.
        let input = AccountInput {
            tenant_id: "cirrus".into(),
            broker_id: "pocketful".into(),
            values: [("client_id", "P1"), ("password", "new-pw"), ("mpin", "1234")]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        let existing = accounts.create(&input, Completeness::Strict).unwrap();

        let outcome = run(dir.path(), &conn, &accounts, &bundle).unwrap().unwrap();

        assert_eq!(outcome.report.updated, 0);
        assert_eq!(secrets.load(&conn, &existing.key()).unwrap()["password"], "new-pw");
    }

    #[test]
    fn no_v1_data_means_nothing_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open_in_memory().unwrap();
        let secrets = MemoryStore::default();
        let bundle = ManifestBundle::bundled().unwrap();
        let accounts = Accounts::new(&conn, &secrets, &bundle);
        assert_eq!(run(dir.path(), &conn, &accounts, &bundle).unwrap(), None);
    }
}
