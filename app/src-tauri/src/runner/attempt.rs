//! One login attempt for one account, plus the retry decision.
//!
//! Retry policy: an attempt may be retried only if it failed for an
//! infrastructure reason (network, browser, page never loaded) *before* any
//! secret was typed or sent. Retrying after credentials were submitted could
//! lock the broker account on a wrong password.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::broker::context::{AccountValues, TemplateContext};
use crate::broker::engine::{EngineError, Outcome, StepEngine};
use crate::broker::http_flows::{self, FlowError};
use crate::broker::manifest::{BrokerKind, BrokerManifest};
use crate::broker::registry::ManifestBundle;
use crate::browser::{BrowserPage, BrowserSession};
use crate::browser::DriverError;
use crate::cirrus::{CirrusClient, CirrusError};
use crate::logging::Redactor;

const SUBMIT_TRIES: u32 = 3;
const SUBMIT_RETRY_DELAY: Duration = Duration::from_secs(2);
const FIRST_LOGIN_HINT: &str =
    "Cirrus doesn't recognise this account yet. Log in to it once on app.cirrus.trade, then AutoLogin can refresh it daily.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptError {
    pub message: String,
    pub retryable: bool,
}

impl AttemptError {
    fn final_(message: impl Into<String>) -> Self {
        Self { message: message.into(), retryable: false }
    }
}

pub struct Attempt<'a> {
    pub manifest: &'a BrokerManifest,
    pub bundle: &'a ManifestBundle,
    pub tenant_id: &'a str,
    pub values: &'a AccountValues,
    pub redactor: &'a Redactor,
    pub http: &'a reqwest::Client,
    pub browser: Option<&'a BrowserSession>,
    /// Where to save a screenshot/HTML if the browser flow fails.
    pub failure_stem: PathBuf,
}

impl Attempt<'_> {
    fn context(&self) -> TemplateContext<'_> {
        TemplateContext::new(self.manifest, self.bundle, self.values).with_tenant(self.tenant_id)
    }

    fn cirrus(&self) -> Result<CirrusClient, AttemptError> {
        let base = self
            .bundle
            .tenant_value(self.tenant_id, "broker_auth_api")
            .ok_or_else(|| AttemptError::final_("this workspace has no Cirrus API configured"))?;
        Ok(CirrusClient::new(base))
    }

    /// Run the attempt; on success returns Cirrus's (or the page's) message.
    pub async fn run(&self) -> Result<String, AttemptError> {
        match self.manifest.kind {
            BrokerKind::Http => self.run_http().await,
            BrokerKind::Browser | BrokerKind::Redirect => self.run_browser().await,
        }
    }

    async fn run_http(&self) -> Result<String, AttemptError> {
        let flow = self.manifest.flow.as_deref().ok_or_else(|| AttemptError::final_("broker has no login flow"))?;
        let mut ctx = self.context();
        let vars = http_flows::run(flow, &ctx, self.http).await.map_err(|e| self.flow_error(e))?;
        for (key, value) in vars {
            ctx.set_var(key, value);
        }
        self.submit(&ctx).await
    }

    async fn run_browser(&self) -> Result<String, AttemptError> {
        let browser = self
            .browser
            .ok_or_else(|| AttemptError { message: "browser is not available".into(), retryable: true })?;
        let mut ctx = self.context();
        let intercept = match self.manifest.callback.as_ref().and_then(|c| c.url_matches.as_deref()) {
            Some(pattern) => {
                let rendered = ctx.render(pattern).map_err(|e| AttemptError::final_(e.to_string()))?;
                Some(regex::Regex::new(&rendered).map_err(|e| AttemptError::final_(e.to_string()))?)
            }
            None => None,
        };
        let page = browser.new_page(intercept).await.map_err(driver_error)?;

        let (outcome, credentials_entered) = {
            let mut engine = StepEngine::new(&page, &mut ctx, self.http, self.redactor);
            let outcome = engine.run(self.manifest).await;
            (outcome, engine.credentials_entered())
        };
        let result = match outcome {
            Ok(Outcome::Callback(url)) => {
                ctx.set_query(query_pairs(&url));
                self.submit(&ctx).await
            }
            Ok(Outcome::PageSuccess) => Ok("Account saved".to_string()),
            Err(error) => {
                self.save_failure(&page).await;
                Err(self.engine_error(error, credentials_entered))
            }
        };
        page.close(browser).await;
        result
    }

    /// Post the login to Cirrus. Network failures are retried here (the
    /// broker code was intercepted, never used, so re-sending it is safe).
    async fn submit(&self, ctx: &TemplateContext<'_>) -> Result<String, AttemptError> {
        let Some(spec) = &self.manifest.callback else {
            return Err(AttemptError::final_("broker has no Cirrus callback configured"));
        };
        let cirrus = self.cirrus()?;
        let mut last = None;
        for attempt in 1..=SUBMIT_TRIES {
            match cirrus.submit_callback(spec, ctx, None).await {
                Ok(message) => return Ok(message),
                Err(CirrusError::Network(error)) => {
                    tracing::warn!(attempt, "Cirrus unreachable: {}", self.redactor.apply(&error.to_string()));
                    last = Some(error.to_string());
                    tokio::time::sleep(SUBMIT_RETRY_DELAY).await;
                }
                Err(CirrusError::Unauthorized) => return Err(AttemptError::final_(FIRST_LOGIN_HINT)),
                Err(other) => return Err(AttemptError::final_(self.redactor.apply(&other.to_string()))),
            }
        }
        Err(AttemptError::final_(format!(
            "Logged in to the broker, but Cirrus could not be reached: {}",
            self.redactor.apply(&last.unwrap_or_default())
        )))
    }

    async fn save_failure(&self, page: &BrowserPage) {
        let png = self.failure_stem.with_extension("png");
        let html = self.failure_stem.with_extension("html");
        if let Some(dir) = png.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        match page.save_failure_artifacts(&png, &html).await {
            Ok(()) => tracing::info!(screenshot = %png.display(), "saved failure screenshot"),
            Err(error) => tracing::warn!(%error, "could not save failure screenshot"),
        }
    }

    fn engine_error(&self, error: EngineError, credentials_entered: bool) -> AttemptError {
        // Technical detail goes to the log; the user gets what happened and what to do.
        tracing::warn!(broker = %self.manifest.id, detail = %self.redactor.apply(&error.to_string()), "browser login failed");
        let retryable = is_infrastructure(&error) && !credentials_entered;
        AttemptError { message: friendly_engine_message(&self.manifest.name, &error), retryable }
    }

    fn flow_error(&self, error: FlowError) -> AttemptError {
        // Only a failed *connection* proves the request never reached the broker.
        let retryable = matches!(&error, FlowError::Network { source, .. } if source.is_connect());
        AttemptError { message: self.redactor.apply(&error.to_string()), retryable }
    }
}

/// A failure a fresh attempt might not hit (page or network trouble), as
/// opposed to an answer: a broker or service that refused the login.
fn is_infrastructure(error: &EngineError) -> bool {
    match error {
        EngineError::Driver(_) | EngineError::SelectorTimeout { .. } | EngineError::WaitTimeout { .. } => true,
        EngineError::Http { refused, .. } => refused.is_none(),
        _ => false,
    }
}

/// What to tell the user about a failed browser login.
pub fn friendly_engine_message(broker: &str, error: &EngineError) -> String {
    const SEE_SCREENSHOT: &str = "A screenshot was saved; see Activity log.";
    match error {
        EngineError::BrokerRejected(text) => format!("{broker} said: \"{text}\""),
        EngineError::SelectorTimeout { .. } | EngineError::WaitTimeout { .. } | EngineError::NotEnoughBoxes { .. } => format!(
            "{broker}'s login page didn't look as expected. {SEE_SCREENSHOT} If this keeps happening, {broker} may have changed its page; a broker update will fix it."
        ),
        EngineError::ResultTimeout(_) => format!("The {broker} login didn't finish in time. {SEE_SCREENSHOT}"),
        EngineError::Driver(_) => "The browser stopped responding or its window was closed.".to_string(),
        EngineError::Http { refused: Some(reason), .. } => reason.clone(),
        EngineError::Http { refused: None, .. } => {
            format!("Couldn't reach the service {broker}'s login needs. Check your internet connection.")
        }
        EngineError::Template(_) | EngineError::InvalidRegex(_) | EngineError::CallbackCaptured => {
            format!("AutoLogin's {broker} setup has a problem. Update AutoLogin or report it on GitHub.")
        }
    }
}

fn driver_error(error: DriverError) -> AttemptError {
    AttemptError { message: error.to_string(), retryable: true }
}

fn query_pairs(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .map(|u| u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_messages_hide_selectors() {
        let timeout = EngineError::SelectorTimeout { step: 4, action: "type", selectors: "#otpNum".into(), timeout_ms: 10_000 };
        let message = friendly_engine_message("Upstox", &timeout);
        assert!(!message.contains("#otpNum") && message.contains("Upstox's login page"), "{message}");
        let rejected = friendly_engine_message("Zerodha", &EngineError::BrokerRejected("Invalid TOTP".into()));
        assert_eq!(rejected, "Zerodha said: \"Invalid TOTP\"");
    }

    #[test]
    fn a_refused_request_shows_the_services_reason_and_is_final() {
        let refused = EngineError::Http {
            step: 1,
            message: "HTTP 401".into(),
            refused: Some("Log in to this Dhan account on Cirrus once first.".into()),
        };
        assert_eq!(friendly_engine_message("Dhan", &refused), "Log in to this Dhan account on Cirrus once first.");
        assert!(!is_infrastructure(&refused));

        let unreachable = EngineError::Http { step: 1, message: "connection refused".into(), refused: None };
        assert!(friendly_engine_message("Dhan", &unreachable).contains("internet connection"));
        assert!(is_infrastructure(&unreachable));
    }

    #[test]
    fn query_pairs_decode_values() {
        let pairs = query_pairs("https://app.cirrus.trade/add-broker-account/upstox?code=a%2Bb&state=UP1");
        assert_eq!(pairs["code"], "a+b");
        assert_eq!(pairs["state"], "UP1");
        assert!(query_pairs("not a url").is_empty());
    }
}
