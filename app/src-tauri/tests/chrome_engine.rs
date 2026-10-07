//! Drives real Chrome against local fixture pages that mimic a broker login.
//! Needs Chrome or Edge installed: `cargo test --test chrome_engine -- --ignored`.
#![cfg(desktop)] // drives desktop Chrome; nothing to run on phones

use std::collections::HashMap;

use autologin_lib::broker::context::{AccountValues, TemplateContext};
use autologin_lib::broker::engine::{Outcome, StepEngine};
use autologin_lib::browser::chromium::{ChromeSession, LaunchOptions};
use autologin_lib::browser::PageDriver;
use autologin_lib::logging::Redactor;
use regex::Regex;

mod common;
use common::{bundle, serve_fixture};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Chrome or Edge"]
async fn logs_in_through_fixture_and_intercepts_callback() {
    let server = serve_fixture().await;
    let bundle = bundle(&server.uri());
    let manifest = bundle.get("fixture").unwrap();
    let account: AccountValues = HashMap::from([
        ("client_id".to_string(), "AB12".to_string()),
        ("password".to_string(), "s3cret!".to_string()),
        ("mpin".to_string(), "4321".to_string()),
    ]);

    let session = ChromeSession::launch(&LaunchOptions { headless: true, executable: None }).await.unwrap();
    let pattern = Regex::new(manifest.callback.as_ref().unwrap().url_matches.as_ref().unwrap()).unwrap();
    let page = session.new_page(Some(pattern)).await.unwrap();

    let mut ctx = TemplateContext::new(manifest, &bundle, &account);
    let http = reqwest::Client::new();
    let redactor = Redactor::new(["s3cret!", "4321"]);
    let outcome = StepEngine::new(&page, &mut ctx, &http, &redactor).run(manifest).await;

    page.close(&session).await;
    session.close().await;

    let outcome = outcome.unwrap();
    let Outcome::Callback(url) = outcome else { panic!("expected intercepted callback, got {outcome:?}") };
    assert!(url.contains("code=ONE-TIME"), "{url}");

    // The /otp request proves the form submitted the typed values.
    let requests = server.received_requests().await.unwrap();
    let otp = requests.iter().find(|r| r.url.path() == "/otp").expect("login form submitted");
    assert!(otp.url.query().unwrap().contains("user=AB12"));
    let finish = requests.iter().find(|r| r.url.path() == "/finish").expect("pin form submitted");
    assert_eq!(finish.url.query().unwrap(), "p1=4&p2=3&p3=2&p4=1");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Chrome or Edge"]
async fn two_sessions_run_side_by_side_with_separate_profiles() {
    // With chromiumoxide's shared default profile, the second launch failed.
    let options = LaunchOptions { headless: true, executable: None };
    let first = ChromeSession::launch(&options).await.expect("first session");
    let second = ChromeSession::launch(&options).await.expect("second session must start while the first runs");
    let page = first.new_page(None).await.unwrap();
    page.goto("data:text/html,<p>ok</p>").await.unwrap();
    let profile = second.profile_dir().to_path_buf();
    assert!(profile.exists());
    page.close(&first).await;
    second.close().await;
    first.close().await;
    assert!(!profile.exists(), "a closed session's profile folder is deleted");
}
