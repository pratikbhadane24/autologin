//! Runs the real Tradejini manifest (brokers/tradejini.toml) in Chrome against a
//! local imitation of CubePlus' SSO login, built from CubePlus' web bundle (the
//! live page needs a real API key): User ID, four PIN boxes, then a 2FA screen
//! that opens on SMS OTP and submits the six-digit code by itself.
//! Needs Chrome or Edge: `cargo test --test chrome_tradejini -- --ignored`.
#![cfg(desktop)] // drives desktop Chrome; nothing to run on phones

use std::collections::HashMap;

use autologin_lib::broker::context::{AccountValues, TemplateContext};
use autologin_lib::broker::engine::{Outcome, StepEngine};
use autologin_lib::broker::registry::ManifestBundle;
use autologin_lib::browser::chromium::{ChromeSession, LaunchOptions};
use autologin_lib::logging::Redactor;
use regex::Regex;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MANIFEST: &str = include_str!("../brokers/tradejini.toml");
const TRADEJINI_API: &str = "https://api.tradejini.com";

// Ids, names and data-cy attributes as in CubePlus' login components. The
// User ID box starts pre-filled (CubePlus can fill it from the link), the
// 2FA screen opens on SMS OTP, and every code submit is counted.
const SSO_PAGE: &str = r##"<!doctype html><html><head><style>.hide{display:none}</style></head><body>
<form id="login" class="login-base user-form">
  <h1>Login to CubePlus</h1>
  <input id="USER_ID" name="USER_ID" type="text" data-cy="fp-userid" value="OLD1">
  <div class="pin-input">
    <input id="PIN" name="PIN" type="password" maxlength="1" data-cy="login-pin"><input id="PIN-1" name="PIN" type="password" maxlength="1" data-cy="login-pin">
    <input id="PIN-2" name="PIN" type="password" maxlength="1" data-cy="login-pin"><input id="PIN-3" name="PIN" type="password" maxlength="1" data-cy="login-pin">
  </div>
  <button type="submit" data-cy="login-submit-btn">Login</button>
</form>
<form id="validate" class="login-base hide">
  <div id="sms-hint">Enter the OTP sent to your mobile</div>
  <input name="otp" type="tel" maxlength="1" data-cy="validate-otp"><input name="otp" type="tel" maxlength="1" data-cy="validate-otp">
  <input name="otp" type="tel" maxlength="1" data-cy="validate-otp"><input name="otp" type="tel" maxlength="1" data-cy="validate-otp">
  <input name="otp" type="tel" maxlength="1" data-cy="validate-otp"><input name="otp" type="tel" maxlength="1" data-cy="validate-otp">
  <button type="submit" data-cy="validate-submit-btn">Proceed</button>
  <div class="prelogin-link sign-up pointer sms-email-otp" data-cy="validate-totp-btn">Switch to TOTP</div>
</form>
<script>
  const $ = (s) => document.querySelector(s);
  const all = (s) => [...document.querySelectorAll(s)];
  let mode = 'otp';
  let submits = 0;
  $('#login').onsubmit = (e) => { e.preventDefault(); $('#login').classList.add('hide'); $('#validate').classList.remove('hide'); };
  $('[data-cy="validate-totp-btn"]').onclick = () => {
    mode = 'totp';
    all('[data-cy="validate-otp"]').forEach((b) => { b.value = ''; });
    $('[data-cy="validate-totp-btn"]').classList.add('hide');
  };
  function finish() {
    submits += 1;
    const pin = all('[data-cy="login-pin"]').map((b) => b.value).join('');
    const code = all('[data-cy="validate-otp"]').map((b) => b.value).join('');
    const state = new URLSearchParams(location.search).get('state');
    setTimeout(() => {
      location.href = '/add-broker-account/tradejini?code=C1&state=' + state + '&user=' + $('#USER_ID').value
        + '&pin=' + pin + '&totp=' + code + '&mode=' + mode + '&submits=' + submits;
    }, 300);
  }
  // Like CubePlus: six digits submit the code without a click.
  all('[data-cy="validate-otp"]').forEach((box) => box.addEventListener('input', () => {
    if (all('[data-cy="validate-otp"]').map((b) => b.value).join('').length === 6) finish();
  }));
  $('#validate').onsubmit = (e) => { e.preventDefault(); finish(); };
</script></body></html>"##;

fn bundle(server: &str) -> ManifestBundle {
    let hosts = format!("^{}", regex::escape(server)).replace('\\', "\\\\");
    let index = format!(
        "schema_version = 1\nmanifest_version = 1\ndefault_tenant = \"t\"\n[tenants.t]\nname = \"Test\"\ncirrus_app = \"{server}\"\nbroker_auth_api = \"{server}\"\ncallback_hosts = \"{hosts}\"\n"
    );
    let manifest = MANIFEST.replace(TRADEJINI_API, server);
    ManifestBundle::from_files([("index.toml", index.as_str()), ("tradejini.toml", manifest.as_str())]).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Chrome or Edge"]
async fn tradejini_types_user_id_pin_and_totp() {
    let server = MockServer::start().await;
    let redirect = format!("{}/add-broker-account/tradejini", server.uri());
    Mock::given(method("GET"))
        .and(path("/v2/api-gw/oauth/authorize"))
        .and(query_param("client_id", "TJ-KEY-1"))
        .and(query_param("redirect_uri", redirect.as_str()))
        .and(query_param("response_type", "code"))
        .and(query_param("scope", "general"))
        .and(query_param("state", "JD1234"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(SSO_PAGE, "text/html"))
        .mount(&server)
        .await;

    let bundle = bundle(&server.uri());
    let manifest = bundle.get("tradejini").unwrap();
    let account: AccountValues = HashMap::from([
        ("client_id".to_string(), "JD1234".to_string()),
        ("api_key".to_string(), "TJ-KEY-1".to_string()),
        ("password".to_string(), "7391".to_string()),
        ("totp_key".to_string(), "JBSWY3DPEHPK3PXP".to_string()),
    ]);
    let mut ctx = TemplateContext::new(manifest, &bundle, &account);
    let pattern = ctx.render(manifest.callback.as_ref().unwrap().url_matches.as_ref().unwrap()).unwrap();

    let session = ChromeSession::launch(&LaunchOptions { headless: true, executable: None }).await.unwrap();
    let page = session.new_page(Some(Regex::new(&pattern).unwrap())).await.unwrap();
    let http = reqwest::Client::new();
    let redactor = Redactor::new(["7391"]);
    let outcome = StepEngine::new(&page, &mut ctx, &http, &redactor).run(manifest).await;
    page.close(&session).await;
    session.close().await;

    let outcome = outcome.unwrap();
    let Outcome::Callback(url) = outcome else { panic!("expected the Cirrus callback, got {outcome:?}") };
    let query: HashMap<String, String> = url::Url::parse(&url).unwrap().query_pairs().into_owned().collect();
    assert_eq!(query["code"], "C1");
    assert_eq!(query["state"], "JD1234");
    assert_eq!(query["user"], "JD1234", "the pre-filled User ID must be replaced");
    assert_eq!(query["pin"], "7391");
    assert_eq!(query["mode"], "totp", "the 2FA screen must be switched to TOTP");
    assert_eq!(query["submits"], "1", "the code must be submitted once");
    assert_eq!(query["totp"].len(), 6);
    assert!(query["totp"].chars().all(|c| c.is_ascii_digit()));
}
