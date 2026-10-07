//! Runs the real 5Paisa manifest (brokers/fivepaisa.toml) in Chrome against a
//! local imitation of 5Paisa's vendor login: client code, six TOTP boxes, six
//! PIN boxes in sections shown one at a time, and a page that submits the PIN
//! by itself. Needs Chrome or Edge: `cargo test --test chrome_fivepaisa -- --ignored`.
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

const MANIFEST: &str = include_str!("../brokers/fivepaisa.toml");

// Same ids and "hide" sections as the live page (2026-10-07); the PIN is
// submitted by the page's own script once all six boxes are filled.
const VENDOR_LOGIN: &str = r##"<!doctype html><html><head><style>.hide{display:none}</style></head><body>
<div class="sec_login"><input type="text" id="ObjVLoginModal_UserName"><button id="BtnGenOtp">Proceed</button></div>
<div class="sec_TOTP hide"><div id="dvLoginTOTP">
  <input type="number" id="dvLoginTOTP1"><input type="number" id="dvLoginTOTP2"><input type="number" id="dvLoginTOTP3">
  <input type="number" id="dvLoginTOTP4"><input type="number" id="dvLoginTOTP5"><input type="number" id="dvLoginTOTP6">
</div><button id="btnTOTPVerify">Verify</button></div>
<div class="sec_enterPin hide"><div id="dvPin">
  <input type="number" id="dvPin1"><input type="number" id="dvPin2"><input type="number" id="dvPin3">
  <input type="number" id="dvPin4"><input type="number" id="dvPin5"><input type="number" id="dvPin6">
</div><button id="btnVerificationSubmit" disabled>Submit</button></div>
<script>
  const show = (cls) => document.querySelectorAll('[class^="sec_"]').forEach((s) => s.classList.toggle('hide', !s.classList.contains(cls)));
  const digits = (id) => [...document.querySelectorAll('#' + id + ' input')].map((b) => b.value).join('');
  document.getElementById('BtnGenOtp').onclick = () => show('sec_TOTP');
  document.getElementById('btnTOTPVerify').onclick = () => show('sec_enterPin');
  document.querySelectorAll('#dvPin input').forEach((box) => box.addEventListener('input', () => {
    if (digits('dvPin').length === 6) {
      const user = document.getElementById('ObjVLoginModal_UserName').value;
      location.href = '/add-broker-account/5paisa?RequestToken=RT1&state=' + user + '&totp=' + digits('dvLoginTOTP') + '&pin=' + digits('dvPin');
    }
  }));
</script></body></html>"##;

fn bundle(server: &str) -> ManifestBundle {
    let hosts = format!("^{}", regex::escape(server)).replace('\\', "\\\\");
    let index = format!(
        "schema_version = 1\nmanifest_version = 1\ndefault_tenant = \"t\"\n[tenants.t]\nname = \"Test\"\ncirrus_app = \"{server}\"\nbroker_auth_api = \"{server}\"\ncallback_hosts = \"{hosts}\"\n"
    );
    let manifest = MANIFEST.replace("https://dev-openapi.5paisa.com", server);
    ManifestBundle::from_files([("index.toml", index.as_str()), ("fivepaisa.toml", manifest.as_str())]).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Chrome or Edge"]
async fn fivepaisa_types_client_code_totp_and_pin() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/WebVendorLogin/VLogin/Index"))
        .and(query_param("VendorKey", "USERKEY123"))
        .and(query_param("State", "52011223"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(VENDOR_LOGIN, "text/html"))
        .mount(&server)
        .await;

    let bundle = bundle(&server.uri());
    let manifest = bundle.get("fivepaisa").unwrap();
    let account: AccountValues = HashMap::from([
        ("client_id".to_string(), "52011223".to_string()),
        ("api_key".to_string(), "USERKEY123".to_string()),
        ("totp_key".to_string(), "JBSWY3DPEHPK3PXP".to_string()),
        ("mpin".to_string(), "246810".to_string()),
    ]);
    let mut ctx = TemplateContext::new(manifest, &bundle, &account);
    let pattern = ctx.render(manifest.callback.as_ref().unwrap().url_matches.as_ref().unwrap()).unwrap();

    let session = ChromeSession::launch(&LaunchOptions { headless: true, executable: None }).await.unwrap();
    let page = session.new_page(Some(Regex::new(&pattern).unwrap())).await.unwrap();
    let http = reqwest::Client::new();
    let redactor = Redactor::new(["246810"]);
    let outcome = StepEngine::new(&page, &mut ctx, &http, &redactor).run(manifest).await;
    page.close(&session).await;
    session.close().await;

    let outcome = outcome.unwrap();
    let Outcome::Callback(url) = outcome else { panic!("expected the Cirrus callback, got {outcome:?}") };
    let query: HashMap<String, String> = url::Url::parse(&url).unwrap().query_pairs().into_owned().collect();
    assert_eq!(query["RequestToken"], "RT1");
    assert_eq!(query["state"], "52011223");
    assert_eq!(query["pin"], "246810");
    assert_eq!(query["totp"].len(), 6);
    assert!(query["totp"].chars().all(|c| c.is_ascii_digit()));
}
