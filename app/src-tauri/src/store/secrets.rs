//! Account secrets. The app uses `vault::VaultStore` (encrypted in the
//! database with one key held in the OS keychain). `KeyringStore` (one
//! keychain item per account) is kept for tests/tools only: it prompts once
//! per account on macOS after every update.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use rusqlite::Connection;
use thiserror::Error;
#[cfg(not(target_os = "android"))]
use zeroize::Zeroizing;

pub const KEYCHAIN_SERVICE: &str = "trade.autologin.autologin";

pub type Secrets = BTreeMap<String, String>;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("AutoLogin couldn't unlock its saved passwords ({0}). When your computer asks, enter your login password and choose Always Allow.")]
    Keychain(String),
    #[error("saved secrets for this account are unreadable; please re-enter them")]
    Corrupt,
    #[error("could not save secrets: {0}")]
    Storage(String),
}

/// Identity of an account's keychain item.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountKey {
    pub tenant_id: String,
    pub broker_id: String,
    pub client_id: String,
}

impl AccountKey {
    pub fn entry_name(&self) -> String {
        format!("{}:{}:{}", self.tenant_id, self.broker_id, self.client_id)
    }
}

/// Stores that keep data in the database get the caller's connection, so a
/// secret write joins the same transaction as the account row.
pub trait SecretStore: Send + Sync {
    /// All secrets for the account; empty if none are saved.
    fn load(&self, conn: &Connection, key: &AccountKey) -> Result<Secrets, SecretError>;
    /// Replace the account's secrets. An empty map deletes them.
    fn save(&self, conn: &Connection, key: &AccountKey, secrets: &Secrets) -> Result<(), SecretError>;
    fn delete(&self, conn: &Connection, key: &AccountKey) -> Result<(), SecretError>;

    /// Make secrets readable now (may show an OS permission prompt).
    fn unlock(&self) -> Result<(), SecretError> {
        Ok(())
    }

    /// Whether reading secrets will work without a permission prompt.
    fn is_unlocked(&self) -> bool {
        true
    }
}

/// One keychain item per account holding a JSON object, so the OS asks for
/// access once per account rather than once per field.
#[cfg(not(target_os = "android"))]
#[derive(Debug, Default)]
pub struct KeyringStore;

#[cfg(not(target_os = "android"))]
impl KeyringStore {
    fn entry(key: &AccountKey) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, &key.entry_name()).map_err(|e| SecretError::Keychain(e.to_string()))
    }
}

#[cfg(not(target_os = "android"))]
impl SecretStore for KeyringStore {
    fn load(&self, _conn: &Connection, key: &AccountKey) -> Result<Secrets, SecretError> {
        match Self::entry(key)?.get_password() {
            Ok(json) => {
                let json = Zeroizing::new(json);
                serde_json::from_str(&json).map_err(|_| SecretError::Corrupt)
            }
            Err(keyring::Error::NoEntry) => Ok(Secrets::new()),
            Err(e) => Err(SecretError::Keychain(e.to_string())),
        }
    }

    fn save(&self, conn: &Connection, key: &AccountKey, secrets: &Secrets) -> Result<(), SecretError> {
        if secrets.is_empty() {
            return self.delete(conn, key);
        }
        let json = Zeroizing::new(serde_json::to_string(secrets).expect("string map always serializes"));
        Self::entry(key)?.set_password(&json).map_err(|e| SecretError::Keychain(e.to_string()))
    }

    fn delete(&self, _conn: &Connection, key: &AccountKey) -> Result<(), SecretError> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Keychain(e.to_string())),
        }
    }
}

/// In-memory store for tests and for previews without keychain access.
#[derive(Debug, Default)]
pub struct MemoryStore {
    items: Mutex<HashMap<AccountKey, Secrets>>,
}

impl SecretStore for MemoryStore {
    fn load(&self, _conn: &Connection, key: &AccountKey) -> Result<Secrets, SecretError> {
        Ok(self.items.lock().expect("lock").get(key).cloned().unwrap_or_default())
    }

    fn save(&self, _conn: &Connection, key: &AccountKey, secrets: &Secrets) -> Result<(), SecretError> {
        let mut items = self.items.lock().expect("lock");
        if secrets.is_empty() {
            items.remove(key);
        } else {
            items.insert(key.clone(), secrets.clone());
        }
        Ok(())
    }

    fn delete(&self, _conn: &Connection, key: &AccountKey) -> Result<(), SecretError> {
        self.items.lock().expect("lock").remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> AccountKey {
        AccountKey { tenant_id: "cirrus".into(), broker_id: "zerodha".into(), client_id: "AB1".into() }
    }

    #[test]
    fn entry_name_includes_tenant_so_tenants_do_not_collide() {
        let other = AccountKey { tenant_id: "pocketful".into(), ..key() };
        assert_ne!(key().entry_name(), other.entry_name());
    }

    #[test]
    fn memory_store_round_trips_and_empty_save_deletes() {
        let conn = Connection::open_in_memory().unwrap();
        let store = MemoryStore::default();
        let secrets: Secrets = [("password".to_string(), "pw".to_string())].into();
        store.save(&conn, &key(), &secrets).unwrap();
        assert_eq!(store.load(&conn, &key()).unwrap(), secrets);
        store.save(&conn, &key(), &Secrets::new()).unwrap();
        assert!(store.load(&conn, &key()).unwrap().is_empty());
    }

    /// Touches the real OS keychain; run manually.
    #[test]
    #[cfg(not(target_os = "android"))]
    #[ignore = "uses the real OS keychain"]
    fn keyring_store_round_trips() {
        let conn = Connection::open_in_memory().unwrap();
        let store = KeyringStore;
        let key = AccountKey { client_id: "AUTOLOGIN-TEST".into(), ..key() };
        let secrets: Secrets = [("mpin".to_string(), "1234".to_string())].into();
        store.save(&conn, &key, &secrets).unwrap();
        assert_eq!(store.load(&conn, &key).unwrap(), secrets);
        store.delete(&conn, &key).unwrap();
        assert!(store.load(&conn, &key).unwrap().is_empty());
    }
}
