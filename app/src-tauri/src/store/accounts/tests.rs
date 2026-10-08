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
        tag: None,
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

#[test]
fn add_or_refresh_fills_only_missing_values_and_never_replaces_saved_ones() {
    let fx = Fixture::new();
    // Saved without an API key (like an AutoLogin 1.x account).
    let mut no_key = zerodha("cirrus", "AB1");
    no_key.values.remove("api_key");
    let original = fx.accounts().create(&no_key, Completeness::AllowMissing).unwrap();

    let paste = |api_key: &str| AccountInput {
        tenant_id: "cirrus".into(),
        broker_id: "zerodha".into(),
        values: [("client_id", "AB1"), ("api_key", api_key), ("password", "")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        tag: None,
    };

    // The missing key is filled in; saved secrets stay.
    let (filled, created) = fx.accounts().add_or_refresh(&paste("cirrus_key")).unwrap();
    assert!(!created);
    assert_eq!(filled.id, original.id);
    assert_eq!(filled.fields["api_key"], "cirrus_key");
    let secrets = fx.secrets.load(&fx.conn, &filled.key()).unwrap();
    assert_eq!(secrets["password"], "pw-123");
    assert_eq!(secrets["totp_key"], "JBSWY3DPEHPK3PXP");

    // A different key later (e.g. from someone else's copy) never replaces it:
    // the API key decides which app the broker sends the login to.
    let (kept, _) = fx.accounts().add_or_refresh(&paste("other_key")).unwrap();
    assert_eq!(kept.fields["api_key"], "cirrus_key");
    assert_eq!(fx.accounts().list().unwrap().len(), 1);
}

#[test]
fn values_differing_from_a_paste_are_reported() {
    let fx = Fixture::new();
    let saved = fx.accounts().create(&zerodha("cirrus", "AB1"), Completeness::Strict).unwrap();
    let pasted: BTreeMap<String, String> =
        [("client_id", "AB1"), ("api_key", "other_key")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    assert_eq!(super::differing_values(&saved, &pasted), vec!["api_key".to_string()]);
    let same: BTreeMap<String, String> = [("api_key".to_string(), "kite_key".to_string())].into();
    assert!(super::differing_values(&saved, &same).is_empty());
}

#[test]
fn add_or_refresh_creates_new_accounts_even_with_values_missing() {
    let fx = Fixture::new();
    let only_client = AccountInput {
        tenant_id: "pocketful".into(),
        broker_id: "zerodha".into(),
        values: [("client_id".to_string(), "AB1".to_string())].into(),
        tag: None,
    };
    let (account, created) = fx.accounts().add_or_refresh(&only_client).unwrap();
    assert!(created);
    assert_eq!(account.tenant_id, "pocketful");
}

fn tagged(input: AccountInput, tag: &str) -> AccountInput {
    AccountInput { tag: Some(tag.into()), ..input }
}

#[test]
fn tag_is_saved_trimmed_kept_on_edit_and_cleared_when_emptied() {
    let fx = Fixture::new();
    let account = fx.accounts().create(&tagged(zerodha("cirrus", "AB1"), "  Pratik D "), Completeness::Strict).unwrap();
    assert_eq!(account.tag.as_deref(), Some("Pratik D"));
    assert_eq!(account.display_name("Zerodha"), "Zerodha AB1 (Pratik D)");

    // No tag in the input (e.g. an import without one) keeps the saved tag.
    let kept = fx.accounts().update(account.id, &zerodha("cirrus", "AB1"), Completeness::Strict).unwrap();
    assert_eq!(kept.tag.as_deref(), Some("Pratik D"));

    let renamed = fx.accounts().update(account.id, &tagged(zerodha("cirrus", "AB1"), "Vinit ant"), Completeness::Strict).unwrap();
    assert_eq!(renamed.tag.as_deref(), Some("Vinit ant"));

    let cleared = fx.accounts().update(account.id, &tagged(zerodha("cirrus", "AB1"), "  "), Completeness::Strict).unwrap();
    assert_eq!(cleared.tag, None);
    assert_eq!(cleared.display_name("Zerodha"), "Zerodha AB1");
}

#[test]
fn invalid_tags_are_reported_next_to_the_name_field() {
    let fx = Fixture::new();
    let err = fx.accounts().create(&tagged(zerodha("cirrus", "AB1"), &"x".repeat(65)), Completeness::Strict).unwrap_err();
    let AccountError::Invalid(fields) = err else { panic!("expected a field error, got {err:?}") };
    assert!(fields.0["tag"].contains("64"));

    // Reported together with the other fields' problems.
    let mut bad = tagged(zerodha("cirrus", "AB1"), "two\nlines");
    bad.values.insert("totp_key".into(), "!!".into());
    let AccountError::Invalid(fields) = fx.accounts().create(&bad, Completeness::Strict).unwrap_err() else { panic!() };
    assert_eq!(fields.0.keys().collect::<Vec<_>>(), vec!["tag", "totp_key"]);
    assert!(fx.accounts().list().unwrap().is_empty());
}

#[test]
fn add_or_refresh_fills_a_missing_tag_but_never_replaces_one() {
    let fx = Fixture::new();
    let original = fx.accounts().create(&zerodha("cirrus", "AB1"), Completeness::Strict).unwrap();
    assert_eq!(original.tag, None);
    let paste = |tag: Option<&str>| AccountInput {
        tenant_id: "cirrus".into(),
        broker_id: "zerodha".into(),
        values: [("client_id".to_string(), "AB1".to_string())].into(),
        tag: tag.map(str::to_string),
    };

    let (filled, created) = fx.accounts().add_or_refresh(&paste(Some("Pratik D"))).unwrap();
    assert!(!created);
    assert_eq!(filled.tag.as_deref(), Some("Pratik D"));

    let (kept, _) = fx.accounts().add_or_refresh(&paste(Some("Someone else"))).unwrap();
    assert_eq!(kept.tag.as_deref(), Some("Pratik D"));
    let (still, _) = fx.accounts().add_or_refresh(&paste(None)).unwrap();
    assert_eq!(still.tag.as_deref(), Some("Pratik D"));

    // A new account takes the pasted tag.
    let new_account = AccountInput { tenant_id: "pocketful".into(), ..paste(Some("Vinit ant")) };
    let (added, created) = fx.accounts().add_or_refresh(&new_account).unwrap();
    assert!(created);
    assert_eq!(added.tag.as_deref(), Some("Vinit ant"));
}

#[test]
fn a_differently_tagged_paste_is_reported() {
    let fx = Fixture::new();
    let untagged = fx.accounts().create(&zerodha("cirrus", "AB1"), Completeness::Strict).unwrap();
    assert!(!super::tag_differs(&untagged, Some("Pratik D")), "an empty tag is filled, not kept");
    let saved = fx.accounts().create(&tagged(zerodha("cirrus", "AB2"), "Pratik D"), Completeness::Strict).unwrap();
    assert!(super::tag_differs(&saved, Some("Vinit ant")));
    assert!(!super::tag_differs(&saved, Some(" Pratik D ")));
    assert!(!super::tag_differs(&saved, None));
    assert!(!super::tag_differs(&saved, Some("")));
}
