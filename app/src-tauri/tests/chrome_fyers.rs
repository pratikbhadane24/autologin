//! Runs the real Fyers manifest (brokers/fyers.toml) in Chrome against a local
//! imitation of Fyers' v3 login page: a mobile/Client ID switch, six TOTP boxes,
//! four PIN boxes that submit by themselves, and hidden screens that reuse the
//! same ids. Needs Chrome or Edge: `cargo test --test chrome_fyers -- --ignored`.
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

const MANIFEST: &str = include_str!("../brokers/fyers.toml");
const FYERS_LOGIN: &str = "https://api-t1.fyers.in/api/v3/generate-authcode";

// Same ids and classes as the live page (2026-10-08). Screens are sections
// shown one at a time; the hidden "forgot" and "create PIN" screens reuse ids
// and similar classes, so a sloppy selector would type into them.
const LOGIN_PAGE: &str = r##"<!doctype html><html><head><style>.hide{display:none}</style></head><body>
<section id="mobile-page" class="page">
  <label><input type="radio" name="loginType" id="mobile_rb" checked><p>Mobile number</p></label>
  <label><input type="radio" name="loginType" id="clientId_rb"><p>Client ID</p></label>
  <div id="mobile-input-section"><input type="tel" id="mobile-code"><button id="mobileNumberSubmit" disabled>Continue</button></div>
  <div id="clientid-input-section" class="hide">
    <input type="text" id="fy_client_id" class="client_id"><button id="clientIdSubmit" disabled>Continue</button>
  </div>
</section>
<section id="confirm-otp-page" class="page hide"><div class="otp-container">
  <input type="number" class="otp-field" id="first"><input type="number" class="otp-field" id="second">
  <input type="number" class="otp-field" id="third"><input type="number" class="otp-field" id="fourth">
  <input type="number" class="otp-field" id="fifth"><input type="number" class="otp-field" id="sixth">
</div><button id="confirmOtpSubmit" disabled>Confirm OTP</button></section>
<section id="verify-pin-page" class="page hide"><div class="pin-container">
  <input type="number" class="pin-field fy-secure-input" id="first"><input type="number" class="pin-field fy-secure-input" id="second">
  <input type="number" class="pin-field fy-secure-input" id="third"><input type="number" class="pin-field fy-secure-input" id="fourth">
</div><button id="verifyPinSubmit" disabled>Login</button></section>
<section id="forgot-clientid-page" class="page hide"><input type="text" id="fy_client_id"></section>
<section id="create-pin-page" class="page hide">
  <input class="c-pin-field"><input class="c-pin-field"><input class="c-pin-field"><input class="c-pin-field">
</section>
<script>
  const $ = (s) => document.querySelector(s);
  const show = (id) => document.querySelectorAll('.page').forEach((p) => p.classList.toggle('hide', p.id !== id));
  const digits = (cls) => [...document.querySelectorAll('#confirm-otp-page .' + cls + ', #verify-pin-page .' + cls)].map((b) => b.value).join('');
  $('#clientId_rb').onchange = () => { $('#mobile-input-section').classList.add('hide'); $('#clientid-input-section').classList.remove('hide'); };
  $('#clientid-input-section #fy_client_id').oninput = (e) => { $('#clientIdSubmit').disabled = !e.target.value; };
  $('#clientIdSubmit').onclick = () => show('confirm-otp-page');
  document.querySelectorAll('.otp-field').forEach((box) => box.addEventListener('input', () => {
    $('#confirmOtpSubmit').disabled = digits('otp-field').length !== 6;
  }));
  $('#confirmOtpSubmit').onclick = () => show('verify-pin-page');
  // The last PIN box submits by itself (data-autosubmit on the live page).
  document.querySelectorAll('.pin-field').forEach((box) => box.addEventListener('input', () => {
    if (digits('pin-field').length === 4) {
      const id = $('#clientid-input-section #fy_client_id').value;
      const state = new URLSearchParams(location.search).get('state');
      location.href = '/add-broker-account/fyers?s=ok&code=200&auth_code=AC1&state=' + state
        + '&typed_id=' + id + '&totp=' + digits('otp-field') + '&pin=' + digits('pin-field');
    }
  }));
</script></body></html>"##;

fn bundle(server: &str) -> ManifestBundle {
    let hosts = format!("^{}", regex::escape(server)).replace('\\', "\\\\");
    let index = format!(
        "schema_version = 1\nmanifest_version = 1\ndefault_tenant = \"t\"\n[tenants.t]\nname = \"Test\"\ncirrus_app = \"{server}\"\nbroker_auth_api = \"{server}\"\ncallback_hosts = \"{hosts}\"\n"
    );
    let manifest = MANIFEST.replace(FYERS_LOGIN, &format!("{server}/api/v3/generate-authcode"));
    ManifestBundle::from_files([("index.toml", index.as_str()), ("fyers.toml", manifest.as_str())]).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Chrome or Edge"]
async fn fyers_types_client_id_totp_and_pin() {
    let server = MockServer::start().await;
    let redirect = format!("{}/add-broker-account/fyers", server.uri());
    Mock::given(method("GET"))
        .and(path("/api/v3/generate-authcode"))
        .and(query_param("client_id", "XB12345-100"))
        .and(query_param("redirect_uri", redirect.as_str()))
        .and(query_param("response_type", "code"))
        .and(query_param("state", "XA00451"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(LOGIN_PAGE, "text/html"))
        .mount(&server)
        .await;

    let bundle = bundle(&server.uri());
    let manifest = bundle.get("fyers").unwrap();
    let account: AccountValues = HashMap::from([
        ("client_id".to_string(), "XA00451".to_string()),
        ("api_key".to_string(), "XB12345-100".to_string()),
        ("totp_key".to_string(), "JBSWY3DPEHPK3PXP".to_string()),
        ("mpin".to_string(), "4826".to_string()),
    ]);
    let mut ctx = TemplateContext::new(manifest, &bundle, &account);
    let pattern = ctx.render(manifest.callback.as_ref().unwrap().url_matches.as_ref().unwrap()).unwrap();

    let session = ChromeSession::launch(&LaunchOptions { headless: true, executable: None }).await.unwrap();
    let page = session.new_page(Some(Regex::new(&pattern).unwrap())).await.unwrap();
    let http = reqwest::Client::new();
    let redactor = Redactor::new(["4826"]);
    let outcome = StepEngine::new(&page, &mut ctx, &http, &redactor).run(manifest).await;
    page.close(&session).await;
    session.close().await;

    let outcome = outcome.unwrap();
    let Outcome::Callback(url) = outcome else { panic!("expected the Cirrus callback, got {outcome:?}") };
    let query: HashMap<String, String> = url::Url::parse(&url).unwrap().query_pairs().into_owned().collect();
    assert_eq!(query["auth_code"], "AC1");
    assert_eq!(query["state"], "XA00451");
    assert_eq!(query["typed_id"], "XA00451", "client ID must go into the visible Client ID box");
    assert_eq!(query["pin"], "4826");
    assert_eq!(query["totp"].len(), 6);
    assert!(query["totp"].chars().all(|c| c.is_ascii_digit()));
}
