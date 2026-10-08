//! Broker accounts: non-secret data in SQLite, secrets in the keychain.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::secrets::{AccountKey, SecretError, SecretStore, Secrets};
use super::validate::{self, Completeness, FieldErrors};
use crate::broker::context::AccountValues;
use crate::broker::manifest::BrokerManifest;
use crate::broker::registry::ManifestBundle;
use crate::session;

const CLIENT_ID: &str = "client_id";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginStatus {
    LoggedOut,
    LoggedIn,
    Failed,
}

impl LoginStatus {
    fn as_db(self) -> &'static str {
        match self {
            Self::LoggedOut => "logged_out",
            Self::LoggedIn => "logged_in",
            Self::Failed => "failed",
        }
    }

    fn from_db(text: &str) -> Self {
        match text {
            "logged_in" => Self::LoggedIn,
            "failed" => Self::Failed,
            _ => Self::LoggedOut,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Account {
    pub id: i64,
    pub tenant_id: String,
    pub broker_id: String,
    pub client_id: String,
    /// The user's own name for the account (Cirrus's "Account Tag"), e.g.
    /// "Pratik D". Not secret; shown wherever the account is named.
    pub tag: Option<String>,
    /// Non-secret field values (never includes secrets).
    pub fields: BTreeMap<String, String>,
    /// Names of secret fields that have a saved value.
    pub secret_keys: Vec<String>,
    pub status: LoginStatus,
    pub last_login: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub added_on: DateTime<Utc>,
}

impl Account {
    pub fn key(&self) -> AccountKey {
        AccountKey {
            tenant_id: self.tenant_id.clone(),
            broker_id: self.broker_id.clone(),
            client_id: self.client_id.clone(),
        }
    }

    /// How the account is named to the user: "Zerodha AB1 (Pratik D)".
    pub fn display_name(&self, broker_name: &str) -> String {
        match &self.tag {
            Some(tag) => format!("{broker_name} {} ({tag})", self.client_id),
            None => format!("{broker_name} {}", self.client_id),
        }
    }

    /// Status as the user should see it: a stored "logged in" whose broker
    /// session has since been reset shows as logged out.
    pub fn effective_status(&self, manifest: Option<&BrokerManifest>, now: DateTime<Utc>) -> LoginStatus {
        match (self.status, self.last_login, manifest) {
            (LoginStatus::LoggedIn, Some(at), Some(m)) if session::is_expired(at, now, &m.session_policy()) => {
                LoginStatus::LoggedOut
            }
            (LoginStatus::LoggedIn, None, _) => LoginStatus::LoggedOut,
            (status, _, _) => status,
        }
    }
}

/// Input for creating or editing an account: every manifest field by key,
/// secrets included. On edit, an omitted/empty secret keeps the saved one.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccountInput {
    pub tenant_id: String,
    pub broker_id: String,
    pub values: BTreeMap<String, String>,
    /// The account's name. On edit, `None` keeps the saved one and an empty
    /// text clears it.
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginResult {
    Success { at: DateTime<Utc> },
    Failure { message: String },
}

#[derive(Debug, Error)]
pub enum AccountError {
    #[error("unknown Cirrus workspace {0:?}")]
    UnknownTenant(String),
    #[error("unknown broker {0:?}")]
    UnknownBroker(String),
    #[error("{0}")]
    Invalid(FieldErrors),
    #[error("this {broker} account ({client_id}) is already added for this workspace")]
    Duplicate { broker: String, client_id: String },
    #[error("account not found")]
    NotFound,
    #[error("select at least one account first")]
    NothingSelected,
    #[error("this account's saved password/PIN is missing from this computer; edit the account and enter it again")]
    SecretsMissing,
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Secret(#[from] SecretError),
}

pub struct Accounts<'a> {
    conn: &'a Connection,
    secrets: &'a dyn SecretStore,
    bundle: &'a ManifestBundle,
}

impl<'a> Accounts<'a> {
    pub fn new(conn: &'a Connection, secrets: &'a dyn SecretStore, bundle: &'a ManifestBundle) -> Self {
        Self { conn, secrets, bundle }
    }

    pub fn list(&self) -> Result<Vec<Account>, AccountError> {
        let mut stmt = self.conn.prepare(&format!("{SELECT} ORDER BY broker_id, client_id, tenant_id"))?;
        let rows = stmt.query_map([], from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Account, AccountError> {
        self.conn
            .query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row)
            .optional()?
            .ok_or(AccountError::NotFound)
    }

    pub fn find(&self, key: &AccountKey) -> Result<Option<Account>, AccountError> {
        Ok(self
            .conn
            .query_row(
                &format!("{SELECT} WHERE tenant_id = ?1 AND broker_id = ?2 AND client_id = ?3"),
                params![key.tenant_id, key.broker_id, key.client_id],
                from_row,
            )
            .optional()?)
    }

    fn manifest(&self, input: &AccountInput) -> Result<&'a BrokerManifest, AccountError> {
        if !self.bundle.has_tenant(&input.tenant_id) {
            return Err(AccountError::UnknownTenant(input.tenant_id.clone()));
        }
        self.bundle.get(&input.broker_id).ok_or_else(|| AccountError::UnknownBroker(input.broker_id.clone()))
    }

    /// Add a pasted account, or refresh it if it's already here. A refresh
    /// only fills values the account doesn't have yet (e.g. the API key a
    /// 1.x account never had). It never replaces a saved value: the API key
    /// decides which app the broker sends the login to, so changing it takes
    /// a deliberate edit, not a paste. The same goes for the tag: a pasted
    /// one is used only if the account has none. Saved secrets stay. Returns the
    /// account and whether it is new.
    pub fn add_or_refresh(&self, input: &AccountInput) -> Result<(Account, bool), AccountError> {
        let manifest = self.manifest(input)?;
        let values = validate::normalize(&input.values);
        let client_id = values.get(CLIENT_ID).cloned().unwrap_or_default();
        let key = AccountKey { tenant_id: input.tenant_id.clone(), broker_id: manifest.id.clone(), client_id };
        let Some(existing) = self.find(&key)? else {
            return Ok((self.create(input, Completeness::AllowMissing)?, true));
        };
        let mut merged = existing.fields.clone();
        merged.insert(CLIENT_ID.to_string(), existing.client_id.clone());
        for (field, value) in values {
            let saved_is_empty = merged.get(&field).is_none_or(|saved| saved.trim().is_empty());
            if saved_is_empty && !value.trim().is_empty() {
                merged.insert(field, value);
            }
        }
        let tag = existing.tag.clone().or_else(|| input.tag.clone());
        let refreshed = self.update(existing.id, &AccountInput { values: merged, tag, ..input.clone() }, Completeness::AllowMissing)?;
        Ok((refreshed, false))
    }

    pub fn create(&self, input: &AccountInput, completeness: Completeness) -> Result<Account, AccountError> {
        let manifest = self.manifest(input)?;
        let values = validate::normalize(&input.values);
        let tag = check(manifest, &values, &[], completeness, input.tag.as_deref())?;
        let (fields, secrets) = split(manifest, &values);
        let client_id = values[CLIENT_ID].clone();
        let key = AccountKey { tenant_id: input.tenant_id.clone(), broker_id: manifest.id.clone(), client_id };
        if self.find(&key)?.is_some() {
            return Err(AccountError::Duplicate { broker: manifest.name.clone(), client_id: key.client_id });
        }

        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO accounts (tenant_id, broker_id, client_id, fields, secret_keys, added_on, tag)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                key.tenant_id,
                key.broker_id,
                key.client_id,
                to_json(&fields),
                to_json(&secrets.keys().collect::<Vec<_>>()),
                Utc::now().to_rfc3339(),
                tag,
            ],
        )?;
        let id = tx.last_insert_rowid();
        // Keychain write happens inside the transaction: if it fails, the row
        // is rolled back and nothing half-saved remains.
        self.secrets.save(&tx, &key, &secrets)?;
        tx.commit()?;
        tracing::info!(account = id, broker = %key.broker_id, tenant = %key.tenant_id, "account added");
        self.get(id)
    }

    pub fn update(&self, id: i64, input: &AccountInput, completeness: Completeness) -> Result<Account, AccountError> {
        let existing = self.get(id)?;
        let manifest = self.manifest(&AccountInput { broker_id: existing.broker_id.clone(), ..input.clone() })?;
        let values = validate::normalize(&input.values);
        let checked_tag = check(manifest, &values, &existing.secret_keys, completeness, input.tag.as_deref())?;
        let tag = if input.tag.is_some() { checked_tag } else { existing.tag.clone() };
        let (fields, new_secrets) = split(manifest, &values);

        let old_key = existing.key();
        let new_key = AccountKey {
            tenant_id: input.tenant_id.clone(),
            broker_id: existing.broker_id.clone(),
            client_id: values[CLIENT_ID].clone(),
        };
        if new_key != old_key && self.find(&new_key)?.is_some() {
            return Err(AccountError::Duplicate { broker: manifest.name.clone(), client_id: new_key.client_id });
        }
        let merged: Secrets = self.secrets.load(self.conn, &old_key)?.into_iter().chain(new_secrets).collect();

        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE accounts SET tenant_id = ?1, client_id = ?2, fields = ?3, secret_keys = ?4, tag = ?5 WHERE id = ?6",
            params![
                new_key.tenant_id,
                new_key.client_id,
                to_json(&fields),
                to_json(&merged.keys().collect::<Vec<_>>()),
                tag,
                id
            ],
        )?;
        if new_key != old_key {
            self.secrets.delete(&tx, &old_key)?;
        }
        self.secrets.save(&tx, &new_key, &merged)?;
        tx.commit()?;
        tracing::info!(account = id, "account updated");
        self.get(id)
    }

    /// Delete the given accounts and their keychain items. Deleting with
    /// nothing selected is an error (v1 deleted *every* account).
    pub fn delete(&self, ids: &[i64]) -> Result<usize, AccountError> {
        if ids.is_empty() {
            return Err(AccountError::NothingSelected);
        }
        let mut deleted = 0;
        for id in ids {
            let account = self.get(*id)?;
            self.secrets.delete(self.conn, &account.key())?;
            deleted += self.conn.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        }
        tracing::info!(count = deleted, "accounts deleted");
        Ok(deleted)
    }

    /// Persist one login's outcome immediately (v1 only saved at batch end).
    pub fn record_result(&self, id: i64, result: &LoginResult) -> Result<(), AccountError> {
        let changed = match result {
            LoginResult::Success { at } => self.conn.execute(
                "UPDATE accounts SET status = ?1, last_login = ?2, last_error = NULL WHERE id = ?3",
                params![LoginStatus::LoggedIn.as_db(), at.to_rfc3339(), id],
            )?,
            LoginResult::Failure { message } => self.conn.execute(
                "UPDATE accounts SET status = ?1, last_error = ?2 WHERE id = ?3",
                params![LoginStatus::Failed.as_db(), message, id],
            )?,
        };
        if changed == 0 {
            return Err(AccountError::NotFound);
        }
        Ok(())
    }

    /// The saved secrets for an account (for backups and logins only).
    pub fn secrets_of(&self, key: &AccountKey) -> Result<Secrets, AccountError> {
        Ok(self.secrets.load(self.conn, key)?)
    }

    /// Everything a login needs: client id, non-secret fields and secrets.
    pub fn login_values(&self, id: i64) -> Result<AccountValues, AccountError> {
        let account = self.get(id)?;
        let secrets = self.secrets.load(self.conn, &account.key())?;
        // The row says secrets were saved but none can be found (e.g. the
        // keychain key was reset): say so instead of failing mid-login.
        if secrets.is_empty() && !account.secret_keys.is_empty() {
            // Self-heal: forget the stale "saved" marker so the account shows
            // "Needs setup" and the form asks for the secrets again.
            self.conn.execute("UPDATE accounts SET secret_keys = '[]' WHERE id = ?1", [id])?;
            return Err(AccountError::SecretsMissing);
        }
        Ok(account
            .fields
            .into_iter()
            .chain(secrets)
            .chain([(CLIENT_ID.to_string(), account.client_id)])
            .collect())
    }
}

/// Check the values and the tag together, so the form shows every problem at
/// once. Returns the tidied tag.
fn check(
    manifest: &BrokerManifest,
    values: &BTreeMap<String, String>,
    stored_secrets: &[String],
    completeness: Completeness,
    tag: Option<&str>,
) -> Result<Option<String>, AccountError> {
    match (validate::validate(manifest, values, stored_secrets, completeness), validate::normalize_tag(tag)) {
        (Ok(()), Ok(tag)) => Ok(tag),
        (fields, tag) => {
            let mut errors = fields.err().unwrap_or_default();
            if let Err(message) = tag {
                errors.0.entry(validate::TAG_FIELD.to_string()).or_insert(message);
            }
            Err(AccountError::Invalid(errors))
        }
    }
}

/// True when the account has a tag and a paste names it differently (the
/// saved tag is kept; the dialog says so).
pub fn tag_differs(saved: &Account, pasted: Option<&str>) -> bool {
    match (saved.tag.as_deref(), pasted.map(str::trim)) {
        (Some(old), Some(new)) => !new.is_empty() && old != new,
        _ => false,
    }
}

/// Saved plain values that a paste would set differently (a paste never
/// replaces them; the dialog tells the user so they can edit on purpose).
pub fn differing_values(saved: &Account, pasted: &BTreeMap<String, String>) -> Vec<String> {
    pasted
        .iter()
        .filter(|(key, value)| key.as_str() != CLIENT_ID && !value.trim().is_empty())
        .filter(|(key, value)| saved.fields.get(*key).is_some_and(|old| !old.trim().is_empty() && old.trim() != value.trim()))
        .map(|(key, _)| key.clone())
        .collect()
}

/// Split values into (non-secret fields without client_id, secrets).
fn split(manifest: &BrokerManifest, values: &BTreeMap<String, String>) -> (BTreeMap<String, String>, Secrets) {
    let is_secret = |key: &str| manifest.field(key).is_some_and(|f| f.secret);
    let fields = values.iter().filter(|(k, _)| *k != CLIENT_ID && !is_secret(k)).map(clone_pair).collect();
    let secrets = values.iter().filter(|(k, _)| is_secret(k)).map(clone_pair).collect();
    (fields, secrets)
}

fn clone_pair((k, v): (&String, &String)) -> (String, String) {
    (k.clone(), v.clone())
}

fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("plain data always serializes")
}

const SELECT: &str = "SELECT id, tenant_id, broker_id, client_id, fields, secret_keys, status, last_login, \
                      last_error, added_on, tag FROM accounts";

fn parse_time(text: Option<String>) -> Option<DateTime<Utc>> {
    text.and_then(|t| DateTime::parse_from_rfc3339(&t).ok()).map(|t| t.with_timezone(&Utc))
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<Account> {
    let fields: String = row.get(4)?;
    let secret_keys: String = row.get(5)?;
    let status: String = row.get(6)?;
    Ok(Account {
        id: row.get(0)?,
        tenant_id: row.get(1)?,
        broker_id: row.get(2)?,
        client_id: row.get(3)?,
        fields: serde_json::from_str(&fields).unwrap_or_default(),
        secret_keys: serde_json::from_str(&secret_keys).unwrap_or_default(),
        status: LoginStatus::from_db(&status),
        last_login: parse_time(row.get(7)?),
        last_error: row.get(8)?,
        added_on: parse_time(row.get(9)?).unwrap_or_else(Utc::now),
        tag: row.get(10)?,
    })
}

#[cfg(test)]
mod tests;
