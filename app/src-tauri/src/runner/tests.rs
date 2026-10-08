use std::collections::BTreeMap;

use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::store::accounts::{AccountInput, LoginStatus};
use crate::store::db;
use crate::store::secrets::MemoryStore;
use crate::store::validate::Completeness;

const TOTP: &str = "JBSWY3DPEHPK3PXP";

/// Bundle whose Motilal API and Cirrus API both point at `server`.
fn bundle_for(server: &str) -> ManifestBundle {
    let mut bundle = ManifestBundle::bundled().unwrap();
    bundle.brokers.get_mut("motilal").unwrap().consts.insert("login_url".into(), format!("{server}/motilal/login"));
    bundle.index.tenants.get_mut("cirrus").unwrap().insert("broker_auth_api".into(), server.into());
    bundle
}

fn deps(bundle: ManifestBundle, data_dir: &std::path::Path) -> RunnerDeps {
    RunnerDeps {
        conn: Arc::new(Mutex::new(db::open_in_memory().unwrap())),
        secrets: Arc::new(MemoryStore::default()),
        bundle: Arc::new(bundle),
        data_dir: data_dir.to_path_buf(),
    }
}

fn add(deps: &RunnerDeps, broker: &str, values: &[(&str, &str)]) -> i64 {
    let input = AccountInput {
        tenant_id: "cirrus".into(),
        broker_id: broker.into(),
        values: values.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>(),
    };
    deps.with_accounts(|a| a.create(&input, Completeness::AllowMissing)).unwrap().id
}

fn motilal(deps: &RunnerDeps) -> i64 {
    add(deps, "motilal", &[("client_id", "EMUM1"), ("api_key", "KEY"), ("password", "pw"), ("dob", "01/01/1990"), ("totp_key", TOTP)])
}

fn options() -> RunOptions {
    RunOptions { retries: 2, ..RunOptions::new(Trigger::Manual, true) }
}

fn collect_events() -> (Arc<Mutex<Vec<RunEvent>>>, impl Fn(RunEvent) + Send + Sync) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    (events, move |event| sink.lock().unwrap().push(event))
}

#[tokio::test]
async fn http_broker_logs_in_and_hands_token_to_cirrus() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/motilal/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "SUCCESS", "AuthToken": "tok" })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/sessions/motilal-oswal/callback"))
        .and(body_partial_json(json!({ "authtoken": "tok", "client_id": "EMUM1" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "message": "Account saved" })))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let deps = deps(bundle_for(&server.uri()), dir.path());
    let id = motilal(&deps);
    let (events, emit) = collect_events();

    let summary = run(&deps, &[id], &options(), &CancellationToken::new(), emit).await;

    assert_eq!((summary.succeeded, summary.failed), (1, 0));
    assert_eq!(deps.with_accounts(|a| a.get(id)).unwrap().status, LoginStatus::LoggedIn);
    let events = events.lock().unwrap();
    assert!(matches!(events.first(), Some(RunEvent::RunStarted { total: 1, .. })));
    assert!(events.contains(&RunEvent::AccountFinished { account_id: id, ok: true, message: "Account saved".into() }));
}

#[tokio::test]
async fn broker_rejection_is_never_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/motilal/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "ERROR", "message": "Invalid password" })))
        .expect(1) // retries: 2 in options, but a rejection must not be retried
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let deps = deps(bundle_for(&server.uri()), dir.path());
    let id = motilal(&deps);

    let summary = run(&deps, &[id], &options(), &CancellationToken::new(), |_| {}).await;

    assert_eq!(summary.failed, 1);
    assert_eq!(summary.failed_accounts, vec!["Motilal Oswal EMUM1".to_string()]);
    let account = deps.with_accounts(|a| a.get(id)).unwrap();
    assert_eq!(account.status, LoginStatus::Failed);
    assert_eq!(account.last_error.as_deref(), Some("authdirectapi rejected: Invalid password"));
}

#[tokio::test]
async fn unknown_account_on_cirrus_gets_first_login_hint() {
    let server = MockServer::start().await;
    Mock::given(path("/motilal/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "SUCCESS", "AuthToken": "tok" })))
        .mount(&server)
        .await;
    Mock::given(path("/api/sessions/motilal-oswal/callback"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "detail": "Not authenticated" })))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let deps = deps(bundle_for(&server.uri()), dir.path());
    let id = motilal(&deps);

    run(&deps, &[id], &options(), &CancellationToken::new(), |_| {}).await;

    let error = deps.with_accounts(|a| a.get(id)).unwrap().last_error.unwrap();
    assert!(error.contains("Log in to it once on app.cirrus.trade"), "{error}");
}

#[tokio::test]
async fn skips_accounts_that_cannot_run_with_reasons() {
    let dir = tempfile::tempdir().unwrap();
    let deps = deps(ManifestBundle::bundled().unwrap(), dir.path());
    let fyers = add(&deps, "fyers", &[("client_id", "XA1"), ("mpin", "1234"), ("totp_key", TOTP)]);
    let pocketful = add(&deps, "pocketful", &[("client_id", "P1")]);
    let (events, emit) = collect_events();

    let summary = run(&deps, &[fyers, pocketful, 999], &options(), &CancellationToken::new(), emit).await;

    assert_eq!(summary.skipped, 3);
    let reasons: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            RunEvent::AccountSkipped { reason, .. } => Some(reason.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(reasons[0], "Fyers support is coming soon.");
    assert_eq!(reasons[1], "Needs setup: add Password, PIN.");
    assert_eq!(reasons[2], "account not found");
}

#[tokio::test]
async fn cancelled_run_does_not_touch_accounts() {
    let server = MockServer::start().await;
    Mock::given(path("/motilal/login")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let deps = deps(bundle_for(&server.uri()), dir.path());
    let id = motilal(&deps);
    let cancel = CancellationToken::new();
    cancel.cancel();

    let summary = run(&deps, &[id], &options(), &cancel, |_| {}).await;

    assert!(summary.cancelled);
    assert_eq!(deps.with_accounts(|a| a.get(id)).unwrap().status, LoginStatus::LoggedOut);
}
