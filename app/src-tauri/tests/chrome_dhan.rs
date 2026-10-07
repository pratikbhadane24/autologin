//! Runs the real Dhan manifest (brokers/dhan.toml) in Chrome against a local
//! imitation of Dhan's consent login: mobile number, then six TOTP boxes that
//! Dhan swaps for six fresh PIN boxes using the same widget.
//! Needs Chrome or Edge: `cargo test --test chrome_dhan -- --ignored`.

use std::collections::HashMap;

use autologin_lib::broker::context::{AccountValues, TemplateContext};
use autologin_lib::broker::engine::{Outcome, StepEngine};
use autologin_lib::broker::registry::ManifestBundle;
use autologin_lib::browser::chromium::{ChromeSession, LaunchOptions};
use autologin_lib::logging::Redactor;
use regex::Regex;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DHAN_MANIFEST: &str = include_str!("../brokers/dhan.toml");

// Like Dhan's Angular page: the TOTP widget is replaced by a new one for the
// PIN a moment after the sixth digit, and the old boxes keep their digits
// until then.
const CONSENT_PAGE: &str = r##"<!doctype html><html><body>
<form id="mobile-form"><input type="tel" id="mobile"><button type="submit">Proceed</button></form>
<div id="widget"></div>
<script>
  let totp = '';
  function boxes(onDone) {
    const widget = document.createElement('code-input');
    for (let i = 0; i < 6; i++) {
      const box = document.createElement('input');
      box.maxLength = 1;
      box.addEventListener('input', () => {
        const all = [...widget.querySelectorAll('input')];
        if (all.every((b) => b.value.length === 1)) onDone(all.map((b) => b.value).join(''));
      });
      widget.appendChild(box);
    }
    const holder = document.getElementById('widget');
    holder.innerHTML = '';
    holder.appendChild(widget);
  }
  document.getElementById('mobile-form').addEventListener('submit', (e) => {
    e.preventDefault();
    document.getElementById('mobile-form').style.display = 'none';
    boxes((code) => { totp = code; setTimeout(() => boxes(finish), 400); });
  });
  function finish(pin) {
    const mobile = document.getElementById('mobile').value;
    location.href = '/add-broker-account/dhan?tokenId=T1&consentId=C1&mobile=' + mobile + '&totp=' + totp + '&pin=' + pin;
  }
</script>
</body></html>"##;

fn bundle(server: &str) -> ManifestBundle {
    let hosts = format!("^{}", regex::escape(server)).replace('\\', "\\\\");
    let index = format!(
        "schema_version = 1\nmanifest_version = 1\ndefault_tenant = \"t\"\n[tenants.t]\nname = \"Test\"\ncirrus_app = \"{server}\"\nbroker_auth_api = \"{server}\"\ncallback_hosts = \"{hosts}\"\n"
    );
    ManifestBundle::from_files([("index.toml", index.as_str()), ("dhan.toml", DHAN_MANIFEST)]).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Chrome or Edge"]
async fn dhan_types_mobile_totp_and_pin_into_the_right_boxes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/sessions/dhan/consent"))
        .and(body_json(serde_json::json!({ "client_id": "1100223344" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": { "login_url": format!("{}/consent-login?consentId=C1", server.uri()), "consent_id": "C1" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/consent-login"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(CONSENT_PAGE, "text/html"))
        .mount(&server)
        .await;

    let bundle = bundle(&server.uri());
    let manifest = bundle.get("dhan").unwrap();
    let account: AccountValues = HashMap::from([
        ("client_id".to_string(), "1100223344".to_string()),
        ("mobile_number".to_string(), "9876543210".to_string()),
        ("totp_key".to_string(), "JBSWY3DPEHPK3PXP".to_string()),
        ("mpin".to_string(), "135790".to_string()),
    ]);
    let mut ctx = TemplateContext::new(manifest, &bundle, &account);
    let pattern = ctx.render(manifest.callback.as_ref().unwrap().url_matches.as_ref().unwrap()).unwrap();

    let session = ChromeSession::launch(&LaunchOptions { headless: true, executable: None }).await.unwrap();
    let page = session.new_page(Some(Regex::new(&pattern).unwrap())).await.unwrap();
    let http = reqwest::Client::new();
    let redactor = Redactor::new(["9876543210", "135790"]);
    let outcome = StepEngine::new(&page, &mut ctx, &http, &redactor).run(manifest).await;
    page.close(&session).await;
    session.close().await;

    let outcome = outcome.unwrap();
    let Outcome::Callback(url) = outcome else { panic!("expected the Cirrus callback, got {outcome:?}") };
    let query: HashMap<String, String> = url::Url::parse(&url).unwrap().query_pairs().into_owned().collect();
    assert_eq!(query["tokenId"], "T1");
    assert_eq!(query["mobile"], "9876543210");
    assert_eq!(query["pin"], "135790", "PIN must go into the fresh boxes");
    assert_eq!(query["totp"].len(), 6);
    assert!(query["totp"].chars().all(|c| c.is_ascii_digit()));
}
