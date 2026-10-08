//! Moving accounts in and out: building backups, reading CSV files (v2 and
//! v1 layouts) and applying an import. v1 data migration reuses `apply`.

use std::collections::BTreeMap;

use chrono::Utc;
use serde::Serialize;

use super::accounts::{AccountError, AccountInput, Accounts};
use super::backup::{BackupAccount, BackupData, BackupError, BACKUP_VERSION};
use super::secrets::AccountKey;
use super::validate::Completeness;
use crate::broker::registry::{normalize_alias, ManifestBundle};

const CLIENT_ID: &str = "client_id";
/// CSV cell key for the account tag (not a broker field).
const TAG: &str = "tag";

/// CSV header (normalized: lowercase letters/digits only) -> field key.
/// Covers v2 exports and v1's friendly names ("Client ID", "TOTP Key", ...).
const COLUMN_ALIASES: &[(&str, &str)] = &[
    ("tenant", "tenant"),
    ("workspace", "tenant"),
    ("broker", "broker"),
    ("clientid", CLIENT_ID),
    ("accounttag", TAG),
    ("tag", TAG),
    ("name", TAG),
    ("userid", CLIENT_ID),
    ("mobilenumber", "mobile_number"),
    ("mobile", "mobile_number"),
    ("password", "password"),
    ("mpin", "mpin"),
    ("pin", "mpin"),
    ("totpkey", "totp_key"),
    ("totp", "totp_key"),
    ("totpsecret", "totp_key"),
    ("apikey", "api_key"),
    ("dob", "dob"),
    ("dateofbirth", "dob"),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportProblem {
    /// 1-based row (CSV: data row, excluding the header) or entry number.
    pub row: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub added: usize,
    pub updated: usize,
    /// Accounts saved without some secrets ("needs setup").
    pub needs_setup: usize,
    pub problems: Vec<ImportProblem>,
}

/// Every account with its secrets, for an encrypted or plain backup.
pub fn collect_backup(accounts: &Accounts<'_>, app_version: &str) -> Result<BackupData, AccountError> {
    let entries = accounts
        .list()?
        .into_iter()
        .map(|account| {
            let secrets = accounts.secrets_of(&account.key())?;
            Ok(BackupAccount {
                tenant_id: account.tenant_id,
                broker_id: account.broker_id,
                client_id: account.client_id,
                tag: account.tag,
                fields: account.fields,
                secrets,
            })
        })
        .collect::<Result<_, AccountError>>()?;
    Ok(BackupData { version: BACKUP_VERSION, exported_at: Utc::now(), app_version: app_version.into(), accounts: entries })
}

/// Parse a CSV into accounts. Secret columns (v1 exported them) are split
/// out using the broker manifest; unknown columns are ignored.
pub fn parse_csv(text: &str, bundle: &ManifestBundle) -> Result<(Vec<BackupAccount>, Vec<ImportProblem>), BackupError> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).trim(csv::Trim::All).from_reader(text.as_bytes());
    let headers: Vec<Option<&str>> = reader
        .headers()
        .map_err(|e| BackupError::Corrupt(e.to_string()))?
        .iter()
        .map(|h| {
            let wanted = normalize_alias(h);
            COLUMN_ALIASES.iter().find(|(alias, _)| *alias == wanted).map(|(_, key)| *key)
        })
        .collect();
    if !headers.contains(&Some("broker")) || !headers.contains(&Some(CLIENT_ID)) {
        return Err(BackupError::Corrupt("the CSV needs 'Broker' and 'Client ID' columns".into()));
    }

    let mut accounts = Vec::new();
    let mut problems = Vec::new();
    for (index, record) in reader.records().enumerate() {
        let row = index + 1;
        let Ok(record) = record else {
            problems.push(ImportProblem { row, reason: "unreadable row".into() });
            continue;
        };
        let cells: BTreeMap<&str, String> = headers
            .iter()
            .zip(record.iter())
            .filter_map(|(key, value)| key.map(|k| (k, value.to_string())))
            .filter(|(_, v)| !v.is_empty())
            .collect();
        match csv_row_to_account(&cells, bundle) {
            Ok(account) => accounts.push(account),
            Err(reason) => problems.push(ImportProblem { row, reason }),
        }
    }
    Ok((accounts, problems))
}

fn csv_row_to_account(cells: &BTreeMap<&str, String>, bundle: &ManifestBundle) -> Result<BackupAccount, String> {
    let broker_name = cells.get("broker").ok_or("missing broker")?;
    let manifest = bundle.resolve_alias(broker_name).ok_or_else(|| format!("unsupported broker {broker_name:?}"))?;
    let client_id = cells.get(CLIENT_ID).ok_or("missing client ID")?.clone();
    let tenant_id = cells.get("tenant").cloned().unwrap_or_else(|| bundle.default_tenant().to_string());
    let tag = cells.get(TAG).cloned();

    let mut fields = BTreeMap::new();
    let mut secrets = BTreeMap::new();
    for (key, value) in cells {
        let Some(field) = manifest.field(key).filter(|f| f.key != CLIENT_ID) else { continue };
        let target = if field.secret { &mut secrets } else { &mut fields };
        target.insert(key.to_string(), value.clone());
    }
    Ok(BackupAccount { tenant_id, broker_id: manifest.id.clone(), client_id, tag, fields, secrets })
}

/// Add or update each account (matched by tenant + broker + client ID).
/// Missing secrets are allowed; those accounts are reported as needing setup.
pub fn apply(accounts: &Accounts<'_>, bundle: &ManifestBundle, entries: &[BackupAccount]) -> ImportReport {
    let mut report = ImportReport::default();
    for (index, entry) in entries.iter().enumerate() {
        let row = index + 1;
        match apply_one(accounts, bundle, entry) {
            Ok((created, needs_setup)) => {
                if created {
                    report.added += 1;
                } else {
                    report.updated += 1;
                }
                report.needs_setup += usize::from(needs_setup);
            }
            Err(error) => report.problems.push(ImportProblem { row, reason: error.to_string() }),
        }
    }
    tracing::info!(added = report.added, updated = report.updated, problems = report.problems.len(), "import applied");
    report
}

/// Returns (created, needs_setup).
fn apply_one(accounts: &Accounts<'_>, bundle: &ManifestBundle, entry: &BackupAccount) -> Result<(bool, bool), AccountError> {
    let broker = bundle.resolve_alias(&entry.broker_id).ok_or_else(|| AccountError::UnknownBroker(entry.broker_id.clone()))?;
    // Drop keys the broker doesn't define (e.g. v1's api_secret).
    let values: BTreeMap<String, String> = entry
        .fields
        .iter()
        .chain(&entry.secrets)
        .filter(|(key, _)| broker.field(key).is_some())
        .map(|(k, v)| (k.clone(), v.clone()))
        .chain([(CLIENT_ID.to_string(), entry.client_id.clone())])
        .collect();
    // A file without a tag (older backup, CSV without the column) keeps the saved one.
    let input = AccountInput { tenant_id: entry.tenant_id.clone(), broker_id: broker.id.clone(), values, tag: entry.tag.clone() };
    let key = AccountKey { tenant_id: entry.tenant_id.clone(), broker_id: broker.id.clone(), client_id: entry.client_id.trim().to_string() };

    let (account, created) = match accounts.find(&key)? {
        Some(existing) => (accounts.update(existing.id, &input, Completeness::AllowMissing)?, false),
        None => (accounts.create(&input, Completeness::AllowMissing)?, true),
    };
    let needs_setup = !super::validate::missing_fields(broker, &account.fields, &account.secret_keys).is_empty();
    Ok((created, needs_setup))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db;
    use crate::store::secrets::{MemoryStore, SecretStore};

    struct Fixture {
        conn: rusqlite::Connection,
        secrets: MemoryStore,
        bundle: ManifestBundle,
    }

    impl Fixture {
        fn new() -> Self {
            Self { conn: db::open_in_memory().unwrap(), secrets: MemoryStore::default(), bundle: ManifestBundle::bundled().unwrap() }
        }
        fn accounts(&self) -> Accounts<'_> {
            Accounts::new(&self.conn, &self.secrets, &self.bundle)
        }
    }

    const V1_CSV: &str = "Broker,Client ID,Mobile Number,Password,MPIN,TOTP Key,API Key,API Secret,Added On,Last Login,Status
Zerodha,AB1,,pw-1,,JBSWY3DPEHPK3PXP,kite,SHOULD-DROP,13-07-2026 09:29,,Logged In
upstox,UP1,9876543210,,123456,JBSWY3DPEHPK3PXP,ukey,,,,
Pocketful,P1,,,,,,,,,
Angel Two,X1,,,,,,,,,
";

    #[test]
    fn imports_v1_csv_splitting_secrets_and_dropping_unknown_columns() {
        let fx = Fixture::new();
        let (entries, problems) = parse_csv(V1_CSV, &fx.bundle).unwrap();
        assert_eq!(problems, vec![ImportProblem { row: 4, reason: "unsupported broker \"Angel Two\"".into() }]);

        let report = apply(&fx.accounts(), &fx.bundle, &entries);
        assert_eq!((report.added, report.updated), (3, 0));
        // Pocketful row has no password -> saved, but needs setup.
        assert_eq!(report.needs_setup, 1);

        let zerodha = fx.accounts().find(&AccountKey {
            tenant_id: "cirrus".into(),
            broker_id: "zerodha".into(),
            client_id: "AB1".into(),
        });
        let zerodha = zerodha.unwrap().unwrap();
        assert_eq!(zerodha.fields, BTreeMap::from([("api_key".into(), "kite".into())]));
        let secrets = fx.secrets.load(&fx.conn, &zerodha.key()).unwrap();
        assert_eq!(secrets.get("password").map(String::as_str), Some("pw-1"));
        assert!(!secrets.values().any(|v| v == "SHOULD-DROP"));
    }

    #[test]
    fn reimport_updates_instead_of_duplicating() {
        let fx = Fixture::new();
        let (entries, _) = parse_csv(V1_CSV, &fx.bundle).unwrap();
        apply(&fx.accounts(), &fx.bundle, &entries);
        let report = apply(&fx.accounts(), &fx.bundle, &entries);
        assert_eq!((report.added, report.updated), (0, 3));
        assert_eq!(fx.accounts().list().unwrap().len(), 3);
    }

    #[test]
    fn backup_round_trip_restores_secrets_on_a_new_machine() {
        let old = Fixture::new();
        let (entries, _) = parse_csv(V1_CSV, &old.bundle).unwrap();
        apply(&old.accounts(), &old.bundle, &entries);
        let backup = collect_backup(&old.accounts(), "2.0.0").unwrap();

        let new = Fixture::new();
        let report = apply(&new.accounts(), &new.bundle, &backup.accounts);
        assert_eq!(report.added, 3);
        let restored = new.accounts().list().unwrap();
        let zerodha = restored.iter().find(|a| a.client_id == "AB1").unwrap();
        assert_eq!(new.secrets.load(&new.conn, &zerodha.key()).unwrap()["password"], "pw-1");
    }

    #[test]
    fn account_tags_survive_backup_and_csv_round_trips() {
        let old = Fixture::new();
        let (entries, _) = parse_csv(V1_CSV, &old.bundle).unwrap();
        apply(&old.accounts(), &old.bundle, &entries);
        let zerodha = old.accounts().list().unwrap().into_iter().find(|a| a.client_id == "AB1").unwrap();
        let tagged = AccountInput {
            tenant_id: "cirrus".into(),
            broker_id: "zerodha".into(),
            values: [("client_id".to_string(), "AB1".to_string()), ("api_key".to_string(), "kite".to_string())].into(),
            tag: Some("Pratik D".into()),
        };
        old.accounts().update(zerodha.id, &tagged, Completeness::AllowMissing).unwrap();
        let backup = collect_backup(&old.accounts(), "2.0.0").unwrap();
        let tag_of = |fx: &Fixture| fx.accounts().list().unwrap().into_iter().find(|a| a.client_id == "AB1").unwrap().tag;

        // Encrypted and plain backups carry the tag (serde round trip).
        let plain = crate::store::backup::export_plain(&backup);
        let restored = crate::store::backup::read_backup(&plain, None).unwrap();
        let new = Fixture::new();
        apply(&new.accounts(), &new.bundle, &restored.accounts);
        assert_eq!(tag_of(&new).as_deref(), Some("Pratik D"));

        // CSV has an "Account Tag" column that is read back.
        let csv = crate::store::backup::export_csv(&backup.accounts);
        assert!(csv.starts_with("tenant,broker,client_id,Account Tag,"), "{csv}");
        let (from_csv, problems) = parse_csv(&csv, &new.bundle).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let fresh = Fixture::new();
        apply(&fresh.accounts(), &fresh.bundle, &from_csv);
        assert_eq!(tag_of(&fresh).as_deref(), Some("Pratik D"));

        // An older CSV without the column keeps the tag already saved.
        apply(&fresh.accounts(), &fresh.bundle, &parse_csv(V1_CSV, &fresh.bundle).unwrap().0);
        assert_eq!(tag_of(&fresh).as_deref(), Some("Pratik D"));
    }

    #[test]
    fn invalid_tags_in_a_file_are_reported_per_row() {
        let fx = Fixture::new();
        let long = "x".repeat(65);
        let (entries, _) = parse_csv(&format!("broker,client_id,Account Tag\nzerodha,AB1,{long}\n"), &fx.bundle).unwrap();
        let report = apply(&fx.accounts(), &fx.bundle, &entries);
        assert_eq!(report.added, 0);
        assert_eq!(report.problems[0].row, 1);
    }

    #[test]
    fn csv_without_required_columns_is_rejected() {
        let fx = Fixture::new();
        assert!(parse_csv("name,phone\nx,y\n", &fx.bundle).is_err());
    }

    #[test]
    fn tenant_column_is_respected() {
        let fx = Fixture::new();
        let (entries, _) = parse_csv("tenant,broker,client_id\npocketful,zerodha,AB1\n", &fx.bundle).unwrap();
        assert_eq!(entries[0].tenant_id, "pocketful");
    }
}
