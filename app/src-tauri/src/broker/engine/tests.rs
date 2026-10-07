use std::collections::HashMap;
use std::sync::Mutex;

use super::*;
use crate::broker::context::AccountValues;
use crate::broker::manifest::{CallbackSpec, FailureSpec, SuccessSpec};
use crate::broker::registry::ManifestBundle;

/// Scripted page: selectors present, plus reactions to clicks.
#[derive(Default)]
struct FakePage {
    state: Mutex<FakeState>,
}

#[derive(Default)]
struct FakeState {
    present: HashMap<String, usize>,
    actions: Vec<String>,
    url: String,
    body: String,
    captured: Option<String>,
    /// selector clicked -> (selector that appears, count)
    on_click_show: HashMap<String, (String, usize)>,
    /// selector clicked -> captured callback URL
    on_click_capture: HashMap<String, String>,
    /// Number of upcoming `count` calls that fail like a page mid-navigation.
    navigating_for: usize,
    /// Same for `body_text`.
    body_navigating_for: usize,
}

impl FakePage {
    fn with(present: &[(&str, usize)]) -> Self {
        let page = Self::default();
        page.state.lock().unwrap().present = present.iter().map(|(s, n)| (s.to_string(), *n)).collect();
        page
    }
    fn actions(&self) -> Vec<String> {
        self.state.lock().unwrap().actions.clone()
    }
    fn set(&self, f: impl FnOnce(&mut FakeState)) {
        f(&mut self.state.lock().unwrap());
    }
}

impl PageDriver for FakePage {
    async fn goto(&self, url: &str) -> Result<(), DriverError> {
        let mut s = self.state.lock().unwrap();
        s.url = url.to_string();
        s.actions.push(format!("goto {url}"));
        Ok(())
    }
    async fn count(&self, selector: &Selector) -> Result<usize, DriverError> {
        let mut s = self.state.lock().unwrap();
        if s.navigating_for > 0 {
            s.navigating_for -= 1;
            return Err(DriverError::Browser("Execution context was destroyed".into()));
        }
        Ok(*s.present.get(&selector.to_string()).unwrap_or(&0))
    }
    async fn click(&self, selector: &Selector, index: usize) -> Result<(), DriverError> {
        let mut s = self.state.lock().unwrap();
        let key = selector.to_string();
        s.actions.push(format!("click {key}[{index}]"));
        if let Some((shown, n)) = s.on_click_show.get(&key).cloned() {
            s.present.insert(shown, n);
        }
        if let Some(url) = s.on_click_capture.get(&key).cloned() {
            s.captured = Some(url);
        }
        Ok(())
    }
    async fn fill(&self, selector: &Selector, index: usize, value: &str) -> Result<(), DriverError> {
        self.state.lock().unwrap().actions.push(format!("fill {selector}[{index}]={value}"));
        Ok(())
    }
    async fn type_text(&self, selector: &Selector, index: usize, value: &str) -> Result<(), DriverError> {
        self.state.lock().unwrap().actions.push(format!("type {selector}[{index}]={value}"));
        Ok(())
    }
    async fn current_url(&self) -> Result<String, DriverError> {
        Ok(self.state.lock().unwrap().url.clone())
    }
    async fn body_text(&self) -> Result<String, DriverError> {
        let mut s = self.state.lock().unwrap();
        if s.body_navigating_for > 0 {
            s.body_navigating_for -= 1;
            return Err(DriverError::Browser("Cannot find context with specified id".into()));
        }
        Ok(s.body.clone())
    }
    async fn evaluate(&self, js: &str) -> Result<(), DriverError> {
        self.state.lock().unwrap().actions.push(format!("eval {js}"));
        Ok(())
    }
    fn captured_callback(&self) -> Option<String> {
        self.state.lock().unwrap().captured.clone()
    }
}

fn demo_manifest(steps: Vec<Step>) -> BrokerManifest {
    let mut manifest = ManifestBundle::bundled().unwrap().get("zerodha").unwrap().clone();
    manifest.steps = steps;
    manifest.callback = None;
    manifest.failure = FailureSpec::default();
    manifest.success = Some(SuccessSpec { url_matches: None, text: Some("Account Saved!".into()), timeout_ms: 1_000 });
    manifest
}

fn sel(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

async fn run_with(page: &FakePage, manifest: &BrokerManifest, account: &AccountValues) -> Result<Outcome, EngineError> {
    let bundle = ManifestBundle::bundled().unwrap();
    let mut ctx = TemplateContext::new(manifest, &bundle, account);
    let http = reqwest::Client::new();
    let redactor = Redactor::default();
    StepEngine::new(page, &mut ctx, &http, &redactor).run(manifest).await
}

fn account() -> AccountValues {
    [("client_id", "AB12"), ("password", "pw")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[tokio::test(start_paused = true)]
async fn runs_steps_in_order_using_first_matching_selector() {
    let page = FakePage::with(&[("css=#new-user", 0), ("css=#userid", 1), ("css=#go", 1)]);
    page.set(|s| s.body = "Account Saved!".into());
    let manifest = demo_manifest(vec![
        Step::Goto { url: "https://broker/login?u={client_id}".into() },
        Step::Fill { sel: sel(&["#new-user", "#userid"]), value: "{client_id}".into(), timeout_ms: None },
        Step::Click { sel: sel(&["#go"]), timeout_ms: None },
    ]);

    let outcome = run_with(&page, &manifest, &account()).await.unwrap();

    assert_eq!(outcome, Outcome::PageSuccess);
    assert_eq!(
        page.actions(),
        vec!["goto https://broker/login?u=AB12", "fill css=#userid[0]=AB12", "click css=#go[0]"]
    );
}

#[tokio::test(start_paused = true)]
async fn missing_selector_times_out_with_step_details() {
    let page = FakePage::default();
    let manifest = demo_manifest(vec![Step::Click { sel: sel(&["#nope", "text=Login"]), timeout_ms: Some(500) }]);

    let err = run_with(&page, &manifest, &account()).await.unwrap_err();

    assert_eq!(err.to_string(), "step 1 (click): none of [#nope, text=Login] appeared within 500 ms");
}

#[tokio::test(start_paused = true)]
async fn waits_for_element_that_appears_after_a_click() {
    let page = FakePage::with(&[("css=#next", 1)]);
    page.set(|s| {
        s.on_click_show.insert("css=#next".into(), ("css=#pin".into(), 1));
        s.body = "Account Saved!".into();
    });
    let manifest = demo_manifest(vec![
        Step::Click { sel: sel(&["#next"]), timeout_ms: None },
        Step::Type { sel: sel(&["#pin"]), value: "{password}".into(), timeout_ms: None },
    ]);

    run_with(&page, &manifest, &account()).await.unwrap();

    assert_eq!(page.actions().last().unwrap(), "type css=#pin[0]=pw");
}

#[tokio::test(start_paused = true)]
async fn optional_steps_run_only_when_present() {
    let switch = || Step::Optional {
        when: sel(&["text=Switch to TOTP"]),
        timeout_ms: Some(300),
        steps: vec![Step::Click { sel: sel(&["text=Switch to TOTP"]), timeout_ms: None }],
    };

    let absent = FakePage::default();
    absent.set(|s| s.body = "Account Saved!".into());
    run_with(&absent, &demo_manifest(vec![switch()]), &account()).await.unwrap();
    assert!(absent.actions().is_empty());

    let present = FakePage::with(&[("text=Switch to TOTP", 1)]);
    present.set(|s| s.body = "Account Saved!".into());
    run_with(&present, &demo_manifest(vec![switch()]), &account()).await.unwrap();
    assert_eq!(present.actions(), vec!["click text=Switch to TOTP[0]"]);
}

#[tokio::test(start_paused = true)]
async fn fill_split_types_one_character_per_box() {
    let page = FakePage::with(&[("css=.otp input", 4)]);
    page.set(|s| s.body = "Account Saved!".into());
    let mut acct = account();
    acct.insert("mpin".into(), "9876".into());
    let manifest =
        demo_manifest(vec![Step::FillSplit { sel: sel(&[".otp input"]), value: "{mpin}".into(), timeout_ms: None }]);

    run_with(&page, &manifest, &acct).await.unwrap();

    let expected: Vec<String> = "9876".chars().enumerate().map(|(i, c)| format!("type css=.otp input[{i}]={c}")).collect();
    assert_eq!(page.actions(), expected);
}

#[tokio::test(start_paused = true)]
async fn fill_split_fails_when_too_few_boxes() {
    let page = FakePage::with(&[("css=.otp input", 3)]);
    let mut acct = account();
    acct.insert("mpin".into(), "9876".into());
    let manifest =
        demo_manifest(vec![Step::FillSplit { sel: sel(&[".otp input"]), value: "{mpin}".into(), timeout_ms: None }]);

    let err = run_with(&page, &manifest, &acct).await.unwrap_err();
    assert!(matches!(err, EngineError::NotEnoughBoxes { needed: 4, found: 3, .. }));
}

#[tokio::test(start_paused = true)]
async fn intercepted_callback_wins_and_page_success_is_ignored() {
    let page = FakePage::with(&[("css=#login", 1)]);
    page.set(|s| {
        s.on_click_capture.insert("css=#login".into(), "https://app.cirrus.trade/add-broker-account/zerodha?request_token=t".into());
    });
    let mut manifest = demo_manifest(vec![Step::Click { sel: sel(&["#login"]), timeout_ms: None }]);
    manifest.callback = Some(CallbackSpec {
        url_matches: Some("cirrus".into()),
        endpoint: "/api/sessions/zerodha/callback".into(),
        body: [("request_token".to_string(), "{query.request_token}".to_string())].into(),
    });

    let outcome = run_with(&page, &manifest, &account()).await.unwrap();

    assert_eq!(
        outcome,
        Outcome::Callback("https://app.cirrus.trade/add-broker-account/zerodha?request_token=t".into())
    );
}

#[tokio::test(start_paused = true)]
async fn broker_error_text_fails_fast() {
    let page = FakePage::default();
    page.set(|s| s.body = "Invalid TOTP. Try again".into());
    let mut manifest = demo_manifest(vec![]);
    manifest.failure = FailureSpec { text_any: vec!["Invalid TOTP".into()] };

    let err = run_with(&page, &manifest, &account()).await.unwrap_err();
    assert_eq!(err.to_string(), "broker showed: Invalid TOTP. Try again");
}

#[tokio::test(start_paused = true)]
async fn page_success_requires_url_and_text_together() {
    let page = FakePage::default();
    page.set(|s| {
        s.body = "Account Saved!".into();
        s.url = "https://broker/still-here".into();
    });
    let mut manifest = demo_manifest(vec![]);
    manifest.success = Some(SuccessSpec {
        url_matches: Some("^https://app\\.cirrus\\.trade/".into()),
        text: Some("Account Saved!".into()),
        timeout_ms: 1_000,
    });

    let err = run_with(&page, &manifest, &account()).await.unwrap_err();
    assert!(matches!(err, EngineError::ResultTimeout(1_000)));
}

#[tokio::test(start_paused = true)]
async fn tracks_whether_credentials_were_entered() {
    let page = FakePage::with(&[("css=#userid", 1), ("css=#pw", 1)]);
    let bundle = ManifestBundle::bundled().unwrap();
    let acct = account();
    let http = reqwest::Client::new();
    let redactor = Redactor::default();

    // Typing only the (non-secret) client id, then failing: safe to retry.
    let manifest = demo_manifest(vec![
        Step::Fill { sel: sel(&["#userid"]), value: "{client_id}".into(), timeout_ms: None },
        Step::Click { sel: sel(&["#missing"]), timeout_ms: Some(100) },
    ]);
    let mut ctx = TemplateContext::new(&manifest, &bundle, &acct);
    let mut engine = StepEngine::new(&page, &mut ctx, &http, &redactor);
    assert!(engine.run(&manifest).await.is_err());
    assert!(!engine.credentials_entered());

    // Password typed before the failure: not safe to retry.
    let manifest = demo_manifest(vec![
        Step::Fill { sel: sel(&["#pw"]), value: "{password}".into(), timeout_ms: None },
        Step::Click { sel: sel(&["#missing"]), timeout_ms: Some(100) },
    ]);
    let mut ctx = TemplateContext::new(&manifest, &bundle, &acct);
    let mut engine = StepEngine::new(&page, &mut ctx, &http, &redactor);
    assert!(engine.run(&manifest).await.is_err());
    assert!(engine.credentials_entered());
}

#[tokio::test(start_paused = true)]
async fn stops_waiting_once_the_callback_is_captured() {
    // Login finished without a PIN page: the optional PIN wait must not run
    // out its timeout, and later steps are skipped.
    let page = FakePage::with(&[("css=#login", 1)]);
    page.set(|s| {
        s.on_click_capture.insert("css=#login".into(), "https://app.cirrus.trade/add-broker-account/pocketful?code=c".into());
    });
    let mut manifest = demo_manifest(vec![
        Step::Click { sel: sel(&["#login"]), timeout_ms: None },
        Step::Optional {
            when: sel(&["text=PIN"]),
            timeout_ms: Some(8_000),
            steps: vec![Step::Click { sel: sel(&["text=PIN"]), timeout_ms: None }],
        },
        Step::Click { sel: sel(&["#never-there"]), timeout_ms: Some(10_000) },
    ]);
    manifest.callback = Some(CallbackSpec {
        url_matches: Some("cirrus".into()),
        endpoint: "/api/sessions/pocketful/callback".into(),
        body: [("code".to_string(), "{query.code}".to_string())].into(),
    });
    let started = tokio::time::Instant::now();

    let outcome = run_with(&page, &manifest, &account()).await.unwrap();

    assert!(matches!(outcome, Outcome::Callback(_)));
    assert!(started.elapsed() < std::time::Duration::from_secs(1), "waited {:?}", started.elapsed());
}

#[tokio::test(start_paused = true)]
async fn keeps_waiting_through_navigation_errors() {
    // After "Sign In" the page navigates; checks briefly fail. The optional
    // PIN step must keep looking instead of treating that as "not present".
    let page = FakePage::with(&[("css=#pin", 1)]);
    page.set(|s| {
        s.navigating_for = 3;
        s.body = "Account Saved!".into();
    });
    let mut acct = account();
    acct.insert("mpin".into(), "4321".into());
    let manifest = demo_manifest(vec![Step::Optional {
        when: sel(&["#pin"]),
        timeout_ms: Some(5_000),
        steps: vec![Step::Fill { sel: sel(&["#pin"]), value: "{mpin}".into(), timeout_ms: None }],
    }]);

    run_with(&page, &manifest, &acct).await.unwrap();

    assert_eq!(page.actions(), vec!["fill css=#pin[0]=4321"]);
}

#[tokio::test(start_paused = true)]
async fn a_page_that_never_recovers_still_fails_at_the_deadline() {
    let page = FakePage::default();
    page.set(|s| s.navigating_for = usize::MAX);
    let manifest = demo_manifest(vec![Step::Click { sel: sel(&["#go"]), timeout_ms: Some(1_000) }]);

    let err = run_with(&page, &manifest, &account()).await.unwrap_err();

    assert!(matches!(err, EngineError::Driver(_)), "{err:?}");
}

#[tokio::test(start_paused = true)]
async fn result_wait_survives_navigation_errors() {
    // Zerodha auto-submits the TOTP; reading the page during that navigation
    // fails briefly. That must not count as a failed login.
    let page = FakePage::default();
    page.set(|s| {
        s.body_navigating_for = 3;
        s.body = "Account Saved!".into();
    });
    let outcome = run_with(&page, &demo_manifest(vec![]), &account()).await.unwrap();
    assert_eq!(outcome, Outcome::PageSuccess);
}

#[tokio::test(start_paused = true)]
async fn broker_error_stops_a_waiting_step_with_the_full_message() {
    // Pocketful shows an error under the form instead of the PIN page.
    let page = FakePage::default();
    page.set(|s| s.body = "Sign In\n43500 - Something went wrong, please retry or contact broker\n".into());
    let mut manifest = demo_manifest(vec![Step::Optional {
        when: sel(&["#pin"]),
        timeout_ms: Some(10_000),
        steps: vec![],
    }]);
    manifest.failure = FailureSpec { text_any: vec!["Something went wrong".into()] };
    let started = tokio::time::Instant::now();

    let err = run_with(&page, &manifest, &account()).await.unwrap_err();

    assert_eq!(
        err.to_string(),
        "broker showed: 43500 - Something went wrong, please retry or contact broker"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1), "waited {:?}", started.elapsed());
}

#[test]
fn failure_line_returns_the_matching_page_line() {
    let phrases = vec!["Invalid TOTP".to_string()];
    let text = "Kite\nInvalid TOTP. Try again.\nForgot?";
    assert_eq!(failure_line(text, &phrases).as_deref(), Some("Invalid TOTP. Try again."));
}

#[test]
fn failure_line_shows_only_the_message_of_a_json_error_body() {
    let phrases = vec!["Invalid `api_key`".to_string()];
    let text = r#"{"status":"error","message":"Invalid `api_key`.","data":null,"error_type":"InputException"}"#;
    assert_eq!(failure_line(text, &phrases).as_deref(), Some("Invalid `api_key`."));
}

fn http_step(url: String) -> Step {
    Step::Http {
        method: "POST".into(),
        url,
        headers: Default::default(),
        body: Some(r#"{{"client_id":{json:client_id}}}"#.into()),
        extract: [("login_url".to_string(), "/data/login_url".to_string())].into(),
    }
}

#[tokio::test]
async fn http_step_values_feed_later_steps() {
    use wiremock::matchers::{body_json, method, path};
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(method("POST"))
        .and(path("/consent"))
        .and(body_json(serde_json::json!({ "client_id": "AB12" })))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "data": { "login_url": "https://broker/consent?id=7" } })),
        )
        .mount(&server)
        .await;
    let page = FakePage::with(&[]);
    page.set(|s| s.body = "Account Saved!".into());
    let manifest = demo_manifest(vec![
        http_step(format!("{}/consent", server.uri())),
        Step::Goto { url: "{vars.login_url}".into() },
    ]);

    run_with(&page, &manifest, &account()).await.unwrap();

    assert_eq!(page.actions(), vec!["goto https://broker/consent?id=7".to_string()]);
}

#[tokio::test]
async fn http_step_failure_shows_the_servers_reason() {
    use wiremock::matchers::method;
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(method("POST"))
        .respond_with(
            wiremock::ResponseTemplate::new(401)
                // The broker-auth API's error envelope.
                .set_body_json(serde_json::json!({
                    "success": false,
                    "message": "Log in to this Dhan account on Cirrus once first.",
                    "data": null
                })),
        )
        .mount(&server)
        .await;
    let page = FakePage::with(&[]);
    let manifest = demo_manifest(vec![http_step(format!("{}/consent", server.uri()))]);

    let error = run_with(&page, &manifest, &account()).await.unwrap_err().to_string();

    assert!(error.contains("Log in to this Dhan account on Cirrus once first."), "{error}");
}
