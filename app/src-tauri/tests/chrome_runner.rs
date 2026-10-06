//! Full browser path through the runner with real Chrome: fixture broker →
//! intercepted callback → Cirrus API (mock) → account saved as logged in.
//! `cargo test --test chrome_runner -- --ignored`

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use autologin_lib::runner::{self, RunOptions, RunnerDeps, Trigger};
use autologin_lib::store::accounts::{AccountInput, Accounts, LoginStatus};
use autologin_lib::store::db;
use autologin_lib::store::secrets::MemoryStore;
use autologin_lib::store::validate::Completeness;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, ResponseTemplate};

mod common;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Chrome or Edge"]
async fn runner_logs_in_with_browser_and_reports_to_cirrus() {
    let server = common::serve_fixture().await;
    Mock::given(method("POST"))
        .and(path("/api/sessions/fixture/callback"))
        .and(body_json(json!({ "code": "ONE-TIME" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "message": "Account saved" })))
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let deps = RunnerDeps {
        conn: Arc::new(Mutex::new(db::open_in_memory().unwrap())),
        secrets: Arc::new(MemoryStore::default()),
        bundle: Arc::new(common::bundle(&server.uri())),
        data_dir: dir.path().to_path_buf(),
    };
    let input = AccountInput {
        tenant_id: "t".into(),
        broker_id: "fixture".into(),
        values: BTreeMap::from([
            ("client_id".into(), "AB12".into()),
            ("password".into(), "s3cret!".into()),
            ("mpin".into(), "4321".into()),
        ]),
    };
    let id = {
        let conn = deps.conn.lock().unwrap();
        Accounts::new(&conn, deps.secrets.as_ref(), &deps.bundle).create(&input, Completeness::Strict).unwrap().id
    };

    let summary = runner::run(&deps, &[id], &RunOptions::new(Trigger::Manual, true), &CancellationToken::new(), |_| {}).await;

    assert_eq!((summary.succeeded, summary.failed), (1, 0), "{summary:?}");
    let conn = deps.conn.lock().unwrap();
    let account = Accounts::new(&conn, deps.secrets.as_ref(), &deps.bundle).get(id).unwrap();
    assert_eq!(account.status, LoginStatus::LoggedIn);
}
