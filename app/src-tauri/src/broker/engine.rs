//! Runs a broker manifest's `[[steps]]` against a page and waits for the
//! result. Every wait polls a condition with a timeout; there are no fixed
//! sleeps like v1's `sleep(10)`.

use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::Value;
use thiserror::Error;
use tokio::time::sleep;

use super::context::TemplateContext;
use super::manifest::{BrokerManifest, Step, DEFAULT_STEP_TIMEOUT_MS, DEFAULT_SUCCESS_TIMEOUT_MS, TOTP_KEY};
use super::vars::{self, TemplateError};
use crate::browser::{DriverError, PageDriver, Selector};
use crate::logging::Redactor;
use crate::totp;

pub const POLL_INTERVAL: Duration = Duration::from_millis(150);
const DEFAULT_OPTIONAL_TIMEOUT_MS: u64 = 5_000;
/// Don't submit a TOTP code with less than this many seconds of validity left.
const MIN_TOTP_SECONDS_LEFT: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Navigation to the broker's redirect URL was intercepted.
    Callback(String),
    /// The page met the manifest's `[success]` conditions.
    PageSuccess,
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Template(#[from] TemplateError),
    #[error(transparent)]
    Driver(#[from] DriverError),
    #[error("step {step} ({action}): none of [{selectors}] appeared within {timeout_ms} ms")]
    SelectorTimeout { step: usize, action: &'static str, selectors: String, timeout_ms: u64 },
    #[error("step {step} (wait_for): condition not met within {timeout_ms} ms")]
    WaitTimeout { step: usize, timeout_ms: u64 },
    #[error("step {step} (fill_split): found {found} boxes for {needed} characters")]
    NotEnoughBoxes { step: usize, needed: usize, found: usize },
    #[error("step {step} (http): {message}")]
    Http { step: usize, message: String },
    #[error("invalid regex {0:?}")]
    InvalidRegex(String),
    #[error("broker showed: {0}")]
    BrokerRejected(String),
    #[error("login did not finish within {0} ms")]
    ResultTimeout(u64),
    /// Internal: the broker already redirected to Cirrus, so the remaining
    /// steps are unnecessary. Never surfaced to callers.
    #[error("login already finished")]
    CallbackCaptured,
}

pub struct StepEngine<'e, 'c, D: PageDriver> {
    driver: &'e D,
    ctx: &'e mut TemplateContext<'c>,
    http: &'e reqwest::Client,
    redactor: &'e Redactor,
    broker_id: String,
    credentials_entered: bool,
}

impl<'e, 'c, D: PageDriver> StepEngine<'e, 'c, D> {
    pub fn new(
        driver: &'e D,
        ctx: &'e mut TemplateContext<'c>,
        http: &'e reqwest::Client,
        redactor: &'e Redactor,
    ) -> Self {
        let broker_id = ctx.manifest().id.clone();
        Self { driver, ctx, http, redactor, broker_id, credentials_entered: false }
    }

    /// Whether a password, PIN, TOTP or other secret was typed into the page
    /// during this run. After that, retrying could lock the broker account.
    pub fn credentials_entered(&self) -> bool {
        self.credentials_entered
    }

    fn note_credentials(&mut self, template: &str) {
        let manifest = self.ctx.manifest();
        let uses_secret = vars::keys(template).unwrap_or_default().iter().any(|key| {
            key == TOTP_KEY || manifest.field(key).is_some_and(|f| f.secret)
        });
        self.credentials_entered |= uses_secret;
    }

    /// Run all steps, then wait for the callback or success condition.
    pub async fn run(&mut self, manifest: &BrokerManifest) -> Result<Outcome, EngineError> {
        self.run_steps(&manifest.steps, 0).await?;
        self.await_result(manifest).await
    }

    async fn run_steps(&mut self, steps: &[Step], depth_offset: usize) -> Result<(), EngineError> {
        for (index, step) in steps.iter().enumerate() {
            let number = depth_offset + index + 1;
            if self.driver.captured_callback().is_some() {
                tracing::debug!(broker = %self.broker_id, step = number, "login finished; skipping remaining steps");
                return Ok(());
            }
            let started = Instant::now();
            match self.run_step(number, step).await {
                Err(EngineError::CallbackCaptured) => return Ok(()),
                other => other?,
            }
            tracing::debug!(
                broker = %self.broker_id,
                step = number,
                action = action_name(step),
                elapsed_ms = started.elapsed().as_millis() as u64,
                "step done"
            );
        }
        Ok(())
    }

    async fn run_step(&mut self, number: usize, step: &Step) -> Result<(), EngineError> {
        match step {
            Step::Goto { url } => {
                let url = self.render(url).await?;
                self.driver.goto(&url).await?;
            }
            Step::Fill { sel, value, timeout_ms } => {
                let (selector, _) = self.find(number, "fill", sel, *timeout_ms).await?;
                self.note_credentials(value);
                let value = self.render(value).await?;
                self.driver.fill(&selector, 0, &value).await?;
            }
            Step::Type { sel, value, timeout_ms } => {
                let (selector, _) = self.find(number, "type", sel, *timeout_ms).await?;
                self.note_credentials(value);
                let value = self.render(value).await?;
                self.driver.type_text(&selector, 0, &value).await?;
            }
            Step::Click { sel, timeout_ms } => {
                let (selector, _) = self.find(number, "click", sel, *timeout_ms).await?;
                self.driver.click(&selector, 0).await?;
            }
            Step::FillSplit { sel, value, timeout_ms } => {
                let (selector, found) = self.find(number, "fill_split", sel, *timeout_ms).await?;
                self.note_credentials(value);
                let value = self.render(value).await?;
                let needed = value.chars().count();
                if found < needed {
                    return Err(EngineError::NotEnoughBoxes { step: number, needed, found });
                }
                for (index, ch) in value.chars().enumerate() {
                    self.driver.type_text(&selector, index, &ch.to_string()).await?;
                }
            }
            Step::WaitFor { sel, url, timeout_ms } => self.wait_for(number, sel, url.as_deref(), *timeout_ms).await?,
            Step::Optional { when, timeout_ms, steps } => {
                let timeout = timeout_ms.unwrap_or(DEFAULT_OPTIONAL_TIMEOUT_MS);
                let present = match self.find(number, "optional", when, Some(timeout)).await {
                    Ok(_) => true,
                    Err(EngineError::CallbackCaptured) => return Err(EngineError::CallbackCaptured),
                    Err(rejected @ EngineError::BrokerRejected(_)) => return Err(rejected),
                    Err(_) => false,
                };
                tracing::debug!(broker = %self.broker_id, step = number, present, "optional step");
                if present {
                    Box::pin(self.run_steps(steps, number * 100)).await?;
                }
            }
            Step::Http { method, url, headers, body, extract } => {
                self.http_step(number, method, url, headers, body.as_deref(), extract).await?
            }
            Step::Eval { js } => {
                let js = self.render(js).await?;
                self.driver.evaluate(&js).await?;
            }
        }
        Ok(())
    }

    /// Render a template; if it uses `{totp}`, first make sure the current
    /// code won't expire mid-submit.
    async fn render(&self, template: &str) -> Result<String, EngineError> {
        if vars::keys(template)?.iter().any(|k| k == TOTP_KEY) {
            let left = totp::seconds_remaining();
            if left < MIN_TOTP_SECONDS_LEFT {
                tracing::debug!(broker = %self.broker_id, wait_s = left, "waiting for fresh TOTP window");
                sleep(Duration::from_secs(left + 1)).await;
            }
        }
        Ok(self.ctx.render(template)?)
    }

    /// The broker's error message, if the page shows one of the manifest's
    /// failure phrases. Read errors (page navigating) count as "no error".
    async fn broker_error(&self) -> Option<String> {
        let phrases = &self.ctx.manifest().failure.text_any;
        if phrases.is_empty() {
            return None;
        }
        let text = self.driver.body_text().await.ok()?;
        failure_line(&text, phrases)
    }

    /// A manifest URL pattern with its `{tenant.*}` values filled in.
    pub fn pattern(&self, template: &str) -> Result<Regex, EngineError> {
        compile(&self.ctx.render(template)?)
    }

    /// Poll the selector list (in order) until one matches.
    async fn find(
        &self,
        step: usize,
        action: &'static str,
        raw: &[String],
        timeout_ms: Option<u64>,
    ) -> Result<(Selector, usize), EngineError> {
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_STEP_TIMEOUT_MS);
        let selectors: Vec<Selector> = raw.iter().map(|s| Selector::parse(s)).collect();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        // Checks can fail briefly while the page navigates ("execution
        // context destroyed"); keep polling and only report the error if the
        // page never answered before the deadline.
        let mut last_error: Option<DriverError> = None;
        loop {
            for selector in &selectors {
                match self.driver.count(selector).await {
                    Ok(count) if count > 0 => return Ok((selector.clone(), count)),
                    Ok(_) => last_error = None,
                    Err(error) => last_error = Some(error),
                }
            }
            if self.driver.captured_callback().is_some() {
                return Err(EngineError::CallbackCaptured);
            }
            // The broker rejected the login (wrong password etc.): stop now
            // instead of waiting out the step's timeout.
            if let Some(message) = self.broker_error().await {
                return Err(EngineError::BrokerRejected(message));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(match last_error {
                    Some(error) => EngineError::Driver(error),
                    None => EngineError::SelectorTimeout { step, action, selectors: raw.join(", "), timeout_ms },
                });
            }
            sleep(POLL_INTERVAL).await;
        }
    }

    async fn wait_for(
        &self,
        step: usize,
        sel: &[String],
        url: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> Result<(), EngineError> {
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_STEP_TIMEOUT_MS);
        let url_re = url.map(|p| self.pattern(p)).transpose()?;
        let selectors: Vec<Selector> = sel.iter().map(|s| Selector::parse(s)).collect();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            // Transient navigation errors count as "not yet", like in `find`.
            let url_ok = match &url_re {
                Some(re) => self.driver.current_url().await.is_ok_and(|u| re.is_match(&u)),
                None => true,
            };
            let mut sel_ok = selectors.is_empty();
            for selector in &selectors {
                if self.driver.count(selector).await.is_ok_and(|n| n > 0) {
                    sel_ok = true;
                    break;
                }
            }
            if url_ok && sel_ok {
                return Ok(());
            }
            if self.driver.captured_callback().is_some() {
                return Err(EngineError::CallbackCaptured);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(EngineError::WaitTimeout { step, timeout_ms });
            }
            sleep(POLL_INTERVAL).await;
        }
    }

    async fn http_step(
        &mut self,
        step: usize,
        method: &str,
        url: &str,
        headers: &std::collections::BTreeMap<String, String>,
        body: Option<&str>,
        extract: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), EngineError> {
        let http_error = |message: String| EngineError::Http { step, message: self.redactor.apply(&message) };
        let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| http_error(e.to_string()))?;
        let mut request = self.http.request(method, self.ctx.render(url)?);
        for (name, template) in headers {
            request = request.header(name, self.ctx.render(template)?);
        }
        if let Some(body) = body {
            request = request.header("Content-Type", "application/json").body(self.ctx.render(body)?);
        }
        let response = request.send().await.map_err(|e| http_error(e.to_string()))?;
        let status = response.status();
        let json: Value = response.json().await.map_err(|e| http_error(e.to_string()))?;
        if !status.is_success() {
            return Err(http_error(format!("HTTP {status}")));
        }
        let mut found = Vec::new();
        for (var, pointer) in extract {
            let value = json
                .pointer(pointer)
                .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
                .ok_or_else(|| http_error(format!("response has no {pointer}")))?;
            found.push((var.clone(), value));
        }
        for (var, value) in found {
            self.ctx.set_var(var, value);
        }
        Ok(())
    }

    /// Wait for the intercepted callback, a broker error text, or the
    /// `[success]` conditions, whichever comes first.
    pub async fn await_result(&self, manifest: &BrokerManifest) -> Result<Outcome, EngineError> {
        let timeout_ms = manifest.success.as_ref().map_or(DEFAULT_SUCCESS_TIMEOUT_MS, |s| s.timeout_ms);
        let intercepting = manifest.callback.as_ref().is_some_and(|c| c.url_matches.is_some());
        let success = manifest.success.as_ref().filter(|_| !intercepting);
        let success_url = success.and_then(|s| s.url_matches.as_deref()).map(|p| self.pattern(p)).transpose()?;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        let mut last_error: Option<DriverError>;

        loop {
            if let Some(url) = self.driver.captured_callback().filter(|_| intercepting) {
                return Ok(Outcome::Callback(url));
            }
            // Reading the page fails briefly while it navigates (e.g. after an
            // auto-submitted TOTP); treat that as "not yet" until the deadline.
            let text = match self.driver.body_text().await {
                Ok(text) => {
                    last_error = None;
                    text
                }
                Err(error) => {
                    last_error = Some(error);
                    String::new()
                }
            };
            if let Some(message) = failure_line(&text, &manifest.failure.text_any) {
                return Err(EngineError::BrokerRejected(message));
            }
            if let Some(success) = success {
                let url_ok = match &success_url {
                    Some(re) => self.driver.current_url().await.is_ok_and(|u| re.is_match(&u)),
                    None => true,
                };
                let text_ok = success.text.as_ref().is_none_or(|t| text.contains(t.as_str()));
                if url_ok && text_ok {
                    return Ok(Outcome::PageSuccess);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(match last_error {
                    Some(error) => EngineError::Driver(error),
                    None => EngineError::ResultTimeout(timeout_ms),
                });
            }
            sleep(POLL_INTERVAL).await;
        }
    }
}

const MAX_BROKER_MESSAGE_CHARS: usize = 200;

/// The page line containing a known failure phrase, so the user sees the
/// broker's full message ("43500 - Something went wrong, ...").
fn failure_line(text: &str, phrases: &[String]) -> Option<String> {
    let phrase = phrases.iter().find(|p| text.contains(p.as_str()))?;
    let line = text.lines().find(|l| l.contains(phrase.as_str())).unwrap_or(phrase).trim();
    let message = json_error_message(line).unwrap_or_else(|| line.to_string());
    Some(message.chars().take(MAX_BROKER_MESSAGE_CHARS).collect())
}

/// Brokers' APIs often answer with a JSON body; show just its message.
fn json_error_message(line: &str) -> Option<String> {
    let body: serde_json::Value = serde_json::from_str(line).ok()?;
    body.get("message")?.as_str().map(str::to_string)
}

fn compile(pattern: &str) -> Result<Regex, EngineError> {
    Regex::new(pattern).map_err(|_| EngineError::InvalidRegex(pattern.to_string()))
}

fn action_name(step: &Step) -> &'static str {
    match step {
        Step::Goto { .. } => "goto",
        Step::Fill { .. } => "fill",
        Step::Type { .. } => "type",
        Step::Click { .. } => "click",
        Step::FillSplit { .. } => "fill_split",
        Step::WaitFor { .. } => "wait_for",
        Step::Optional { .. } => "optional",
        Step::Http { .. } => "http",
        Step::Eval { .. } => "eval",
    }
}

#[cfg(test)]
mod tests;
