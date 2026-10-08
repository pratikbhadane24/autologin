//! Runs the real AliceBlue manifest (brokers/aliceblue.toml) in Chrome against a
//! local imitation of ANT's web login: User ID and password on one form, then
//! a code screen that submits the six-digit TOTP by itself, then the one-time
//! "Authorize Alice Blue" screen, then the redirect with authCode and userId.
//! Needs Chrome or Edge: `cargo test --test chrome_aliceblue -- --ignored`.
#![cfg(desktop)] // drives desktop Chrome; nothing to run on phones

use std::collections::HashMap;

use autologin_lib::broker::context::{AccountValues, TemplateContext};
use autologin_lib::broker::engine::{EngineError, Outcome, StepEngine};
use autologin_lib::broker::registry::ManifestBundle;
use autologin_lib::browser::chromium::{ChromeSession, LaunchOptions};
use autologin_lib::logging::Redactor;
use regex::Regex;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MANIFEST: &str = include_str!("../brokers/aliceblue.toml");
const ANT_WEB: &str = "https://ant.aliceblueonline.com";

// Ids and texts as on the live page and in ANT's bundle (2026-10-08). The code
// screen appears after a short delay (ANT checks the password first), accepts
// digits only, submits on the sixth digit, and every submit is counted. The
// CleverTap push prompt shows up too.
const LOGIN_PAGE: &str = r##"<!doctype html><html><head><style>.hide{display:none}</style></head><body>
<form id="userAndPasswordForm" autocomplete="off">
  <label for="new_login_userId">User ID / Mobile Number / Email</label>
  <input id="new_login_userId" placeholder="User ID / Mobile Number / Email">
  <label for="new_login_password">Password</label>
  <input id="new_login_password" type="password" maxlength="16">
  <button type="submit" class="submitButton" id="buttonLabel_Next"><span>Next</span></button>
</form>
<form id="otp_verification_form" class="hide">
  <label for="new_login_otp" id="new_login_otp_label">TOTP</label>
  <input id="new_login_otp" type="text" maxlength="6" autocomplete="off">
  <div class="bottom-button-container"><span>Mobile OTP</span><span>Reset TOTP</span></div>
  <button type="submit" class="submitButton" id="buttonLabel_Next"><span>Next</span></button>
</form>
<form id="authorizeForm" class="hide">
  <div>Authorize Alice Blue</div><div>Permission required by the app</div>
  <button type="submit" class="submitButton" id="buttonLabel_Authorize"><span>Authorize</span></button>
</form>
<script>
  const $ = (s) => document.querySelector(s);
  const show = (id) => document.querySelectorAll('form').forEach((f) => f.classList.toggle('hide', f.id !== id));
  let submits = 0;
  $('#userAndPasswordForm').onsubmit = (e) => { e.preventDefault(); setTimeout(() => show('otp_verification_form'), 400); };
  const otp = $('#new_login_otp');
  otp.addEventListener('keypress', (e) => { if (!/[0-9]/.test(e.key)) e.preventDefault(); });
  function verify() { submits += 1; setTimeout(() => show('authorizeForm'), 300); }
  otp.addEventListener('input', () => { if (otp.value.length === 6) verify(); });
  $('#otp_verification_form').onsubmit = (e) => { e.preventDefault(); verify(); };
  $('#authorizeForm').onsubmit = (e) => {
    e.preventDefault();
    const appcode = new URLSearchParams(location.search).get('appcode');
    location.href = '/add-broker-account/aliceblue?authCode=AUTH1&userId=' + $('#new_login_userId').value.toUpperCase()
      + '&appcode=' + appcode + '&password=' + encodeURIComponent($('#new_login_password').value)
      + '&totp=' + otp.value + '&submits=' + submits;
  };
  // CleverTap's push prompt, as on the live page: it arrives a moment after
  // load, and its full-page overlay takes every click.
  setTimeout(() => document.body.insertAdjacentHTML('beforeend',
    '<div id="wzrk_wrapper"><div class="wzrk-overlay" style="position:fixed;inset:0;z-index:9999;background:rgba(0,0,0,.3)"></div>'
    + '<div class="wzrk-alert" style="position:fixed;top:0;right:0;z-index:10000"><div class="wzrk-alert-heading">ANT Expertise Unlocked!</div>'
    + '<button id="wzrk-cancel">Not Interested</button><button id="wzrk-confirm">Interested</button></div></div>'), 150);
</script></body></html>"##;

fn bundle(server: &str) -> ManifestBundle {
    let hosts = format!("^{}", regex::escape(server)).replace('\\', "\\\\");
    let index = format!(
        "schema_version = 1\nmanifest_version = 1\ndefault_tenant = \"t\"\n[tenants.t]\nname = \"Test\"\ncirrus_app = \"{server}\"\nbroker_auth_api = \"{server}\"\ncallback_hosts = \"{hosts}\"\n"
    );
    let manifest = MANIFEST.replace(ANT_WEB, server);
    ManifestBundle::from_files([("index.toml", index.as_str()), ("aliceblue.toml", manifest.as_str())]).unwrap()
}

/// Serve `login_page` as ANT and run the real manifest against it.
async fn run_login(login_page: &str) -> Result<Outcome, EngineError> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(query_param("appcode", "AppCode1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(login_page.to_string(), "text/html"))
        .mount(&server)
        .await;

    let bundle = bundle(&server.uri());
    let manifest = bundle.get("aliceblue").unwrap();
    let account: AccountValues = HashMap::from([
        ("client_id".to_string(), "ab1234".to_string()),
        ("api_key".to_string(), "AppCode1".to_string()),
        ("password".to_string(), "S3cret&Pass".to_string()),
        ("totp_key".to_string(), "JBSWY3DPEHPK3PXP".to_string()),
    ]);
    let mut ctx = TemplateContext::new(manifest, &bundle, &account);
    let pattern = ctx.render(manifest.callback.as_ref().unwrap().url_matches.as_ref().unwrap()).unwrap();

    let session = ChromeSession::launch(&LaunchOptions { headless: true, executable: None }).await.unwrap();
    let page = session.new_page(Some(Regex::new(&pattern).unwrap())).await.unwrap();
    let http = reqwest::Client::new();
    let redactor = Redactor::new(["S3cret&Pass"]);
    let outcome = StepEngine::new(&page, &mut ctx, &http, &redactor).run(manifest).await;
    page.close(&session).await;
    session.close().await;
    outcome
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Chrome or Edge"]
async fn aliceblue_types_user_id_password_and_totp_then_authorizes() {
    let outcome = run_login(LOGIN_PAGE).await.unwrap();
    let Outcome::Callback(url) = outcome else { panic!("expected the Cirrus callback, got {outcome:?}") };
    let query: HashMap<String, String> = url::Url::parse(&url).unwrap().query_pairs().into_owned().collect();
    assert_eq!(query["authCode"], "AUTH1");
    assert_eq!(query["userId"], "AB1234");
    assert_eq!(query["appcode"], "AppCode1");
    assert_eq!(query["password"], "S3cret&Pass");
    assert_eq!(query["submits"], "1", "the TOTP must be submitted once");
    assert_eq!(query["totp"].len(), 6);
    assert!(query["totp"].chars().all(|c| c.is_ascii_digit()));

    // The callback hands Cirrus the redirect's own authCode and userId.
    let bundle = bundle("https://cirrus.example");
    let callback = bundle.get("aliceblue").unwrap().callback.clone().unwrap();
    assert_eq!(callback.endpoint, "/api/sessions/aliceblue/callback");
    assert_eq!(callback.body["auth_code"], "{query.authCode}");
    assert_eq!(callback.body["user_id"], "{query.userId}");
}

// An account without TOTP gets ANT's SMS OTP screen; no code may be typed there.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Chrome or Edge"]
async fn aliceblue_stops_on_the_sms_otp_screen() {
    let page = LOGIN_PAGE
        .replace(">TOTP</label>", ">Mobile OTP</label>")
        .replace("<span>Mobile OTP</span><span>Reset TOTP</span>", "<span>Register for TOTP</span><span>Resend OTP</span>");
    match run_login(&page).await {
        Err(EngineError::BrokerRejected(message)) => assert!(message.contains("Register for TOTP"), "{message}"),
        other => panic!("expected the SMS OTP screen to stop the login, got {other:?}"),
    }
}
