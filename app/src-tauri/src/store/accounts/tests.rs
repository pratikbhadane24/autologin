use chrono::TimeZone;

use super::*;
use crate::store::db;
use crate::store::secrets::MemoryStore;

use crate::store::validate::Completeness;

struct Fixture {
    conn: Connection,
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

fn zerodha(tenant: &str, client: &str) -> AccountInput {
    AccountInput {
        tenant_id: tenant.into(),
        broker_id: "zerodha".into(),
        values: [
            ("client_id", client),
            ("api_key", "kite_key"),
            ("password", "pw-123"),
            ("totp_key", "JBSWY3DPEHPK3PXP"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect(),
    }
}

#[test]
fn create_keeps_secrets_out_of_the_database() {
    let fx = Fixture::new();
    let account = fx.accounts().create(&zerodha("cirrus", "AB1"), Completeness::Strict).unwrap();

    assert_eq!(account.fields, BTreeMap::from([("api_key".into(), "kite_key".into())]));
    assert_eq!(account.secret_keys, vec!["password".to_string(), "totp_key".to_string()]);
    let row: String = fx.conn.query_row("SELECT fields || secret_keys FROM accounts", [], |r| r.get(0)).unwrap();
    assert!(!row.contains("pw-123") && !row.contains("JBSWY3DP"), "{row}");
    assert_eq!(fx.secrets.load(&fx.conn, &account.key()).unwrap()["password"], "pw-123");
}

#[test]
fn same_client_can_be_added_once_per_tenant() {
    let fx = Fixture::new();
    fx.accounts().create(&zerodha("cirrus", "AB1"), Completeness::Strict).unwrap();
    fx.accounts().create(&zerodha("pocketful", "AB1"), Completeness::Strict).unwrap();
    let err = fx.accounts().create(&zerodha("cirrus", "AB1"), Completeness::Strict).unwrap_err();
    assert!(matches!(err, AccountError::Duplicate { .. }));
    assert_eq!(fx.accounts().list().unwrap().len(), 2);
}

#[test]
fn rejects_unknown_tenant_broker_and_invalid_values() {
    let fx = Fixture::new();
    assert!(matches!(fx.accounts().create(&zerodha("evil", "AB1"), Completeness::Strict), Err(AccountError::UnknownTenant(_))));
    let unknown = AccountInput { broker_id: "nope".into(), ..zerodha("cirrus", "AB1") };
    assert!(matches!(fx.accounts().create(&unknown, Completeness::Strict), Err(AccountError::UnknownBroker(_))));
    let mut bad = zerodha("cirrus", "AB1");
    bad.values.insert("totp_key".into(), "!!".into());
    assert!(matches!(fx.accounts().create(&bad, Completeness::Strict), Err(AccountError::Invalid(_))));
}

#[test]
fn update_keeps_blank_secrets_and_moves_keychain_item_on_rename() {
    let fx = Fixture::new();
    let account = fx.accounts().create(&zerodha("cirrus", "AB1"), Completeness::Strict).unwrap();
    let mut edit = zerodha("cirrus", "AB2");
    edit.values.insert("password".into(), "  ".into()); // blank = keep saved
    edit.values.remove("totp_key");

    let updated = fx.accounts().update(account.id, &edit, Completeness::Strict).unwrap();

    assert_eq!(updated.client_id, "AB2");
    let moved = fx.secrets.load(&fx.conn, &updated.key()).unwrap();
    assert_eq!(moved["password"], "pw-123");
    assert_eq!(moved["totp_key"], "JBSWY3DPEHPK3PXP");
    assert!(fx.secrets.load(&fx.conn, &account.key()).unwrap().is_empty());
}

#[test]
fn delete_requires_a_selection_and_removes_secrets() {
    let fx = Fixture::new();
    let account = fx.accounts().create(&zerodha("cirrus", "AB1"), Completeness::Strict).unwrap();
    assert!(matches!(fx.accounts().delete(&[]), Err(AccountError::NothingSelected)));
    assert_eq!(fx.accounts().list().unwrap().len(), 1);

    assert_eq!(fx.accounts().delete(&[account.id]).unwrap(), 1);
    assert!(fx.accounts().list().unwrap().is_empty());
    assert!(fx.secrets.load(&fx.conn, &account.key()).unwrap().is_empty());
}

#[test]
fn record_result_persists_immediately_and_login_values_merge_everything() {
    let fx = Fixture::new();
    let account = fx.accounts().create(&zerodha("cirrus", "AB1"), Completeness::Strict).unwrap();
    let at = Utc.with_ymd_and_hms(2026, 10, 5, 3, 30, 0).unwrap();

    fx.accounts().record_result(account.id, &LoginResult::Success { at }).unwrap();
    let saved = fx.accounts().get(account.id).unwrap();
    assert_eq!((saved.status, saved.last_login), (LoginStatus::LoggedIn, Some(at)));

    fx.accounts().record_result(account.id, &LoginResult::Failure { message: "Invalid TOTP".into() }).unwrap();
    let saved = fx.accounts().get(account.id).unwrap();
    assert_eq!(saved.status, LoginStatus::Failed);
    assert_eq!(saved.last_error.as_deref(), Some("Invalid TOTP"));

    let values = fx.accounts().login_values(account.id).unwrap();
    assert_eq!(values["client_id"], "AB1");
    assert_eq!(values["api_key"], "kite_key");
    assert_eq!(values["password"], "pw-123");
}

#[test]
fn effective_status_expires_after_broker_reset() {
    let fx = Fixture::new();
    let account = fx.accounts().create(&zerodha("cirrus", "AB1"), Completeness::Strict).unwrap();
    let manifest = fx.bundle.get("zerodha");
    // Zerodha resets at 06:00 IST = 00:30 UTC.
    let login = Utc.with_ymd_and_hms(2026, 10, 5, 3, 30, 0).unwrap();
    fx.accounts().record_result(account.id, &LoginResult::Success { at: login }).unwrap();
    let saved = fx.accounts().get(account.id).unwrap();

    let before_reset = Utc.with_ymd_and_hms(2026, 10, 6, 0, 29, 0).unwrap();
    let after_reset = Utc.with_ymd_and_hms(2026, 10, 6, 0, 30, 0).unwrap();
    assert_eq!(saved.effective_status(manifest, before_reset), LoginStatus::LoggedIn);
    assert_eq!(saved.effective_status(manifest, after_reset), LoginStatus::LoggedOut);
}

#[test]
fn login_values_reports_lost_secrets_clearly() {
    let fx = Fixture::new();
    let account = fx.accounts().create(&zerodha("cirrus", "AB1"), Completeness::Strict).unwrap();
    fx.secrets.delete(&fx.conn, &account.key()).unwrap();
    assert!(matches!(fx.accounts().login_values(account.id), Err(AccountError::SecretsMissing)));
    assert!(fx.accounts().get(account.id).unwrap().secret_keys.is_empty(), "now shows as needs setup");
}
