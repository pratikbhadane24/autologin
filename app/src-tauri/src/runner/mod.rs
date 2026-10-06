//! Runs logins for a set of accounts: parallel, cancellable, with safe
//! retries, progress events, and each result saved as soon as it is known.

pub mod artifacts;
mod attempt;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use chrono::Utc;
use futures::StreamExt;
use rusqlite::{params, Connection};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::broker::manifest::{Availability, BrokerKind};
use crate::broker::registry::ManifestBundle;
use crate::browser::chromium::{ChromeSession, LaunchOptions};
use crate::logging::Redactor;
use crate::store::accounts::{Account, Accounts, LoginResult};
use crate::store::secrets::SecretStore;
use crate::store::validate;
use attempt::{Attempt, AttemptError};

pub const DEFAULT_RETRIES: u32 = 1;
const MIN_CONCURRENCY: usize = 2;
const MAX_CONCURRENCY: usize = 6;
const RETRY_BACKOFF: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Manual,
    Scheduled,
    Retry,
}

impl Trigger {
    fn as_db(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Scheduled => "scheduled",
            Self::Retry => "retry",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub trigger: Trigger,
    pub headless: bool,
    pub retries: u32,
    pub concurrency: usize,
    pub chrome_executable: Option<PathBuf>,
}

impl RunOptions {
    pub fn new(trigger: Trigger, headless: bool) -> Self {
        Self { trigger, headless, retries: DEFAULT_RETRIES, concurrency: default_concurrency(), chrome_executable: None }
    }
}

/// One Chrome tab per concurrent login; bounded so laptops stay responsive.
pub fn default_concurrency() -> usize {
    std::thread::available_parallelism().map_or(MIN_CONCURRENCY, |n| n.get()).clamp(MIN_CONCURRENCY, MAX_CONCURRENCY)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    RunStarted { run_id: i64, total: usize },
    AccountStarted { account_id: i64, attempt: u32 },
    AccountFinished { account_id: i64, ok: bool, message: String },
    AccountSkipped { account_id: i64, reason: String },
    RunFinished { run_id: i64, summary: RunSummary },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RunSummary {
    pub succeeded: usize,
    pub failed: usize,
    pub skipped: usize,
    pub cancelled: bool,
    /// "Broker client_id" of each failed account, for the notification.
    pub failed_accounts: Vec<String>,
    /// Ids of failed accounts, for "retry failed after N minutes".
    pub failed_ids: Vec<i64>,
}

pub struct RunnerDeps {
    pub conn: Arc<Mutex<Connection>>,
    pub secrets: Arc<dyn SecretStore>,
    pub bundle: Arc<ManifestBundle>,
    pub data_dir: PathBuf,
}

impl RunnerDeps {
    fn with_accounts<T>(&self, f: impl FnOnce(&Accounts<'_>) -> T) -> T {
        let conn = self.conn.lock().expect("database lock poisoned");
        f(&Accounts::new(&conn, self.secrets.as_ref(), &self.bundle))
    }
}

enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
}

pub async fn run<E>(deps: &RunnerDeps, account_ids: &[i64], options: &RunOptions, cancel: &CancellationToken, emit: E) -> RunSummary
where
    E: Fn(RunEvent) + Send + Sync,
{
    artifacts::prune(&deps.data_dir, SystemTime::now());
    let run_id = start_run_row(deps, options.trigger);
    emit(RunEvent::RunStarted { run_id, total: account_ids.len() });

    let mut summary = RunSummary::default();
    let mut runnable = Vec::new();
    for &id in account_ids {
        match plan(deps, id) {
            Ok(account) => runnable.push(account),
            Err(reason) => {
                summary.skipped += 1;
                emit(RunEvent::AccountSkipped { account_id: id, reason });
            }
        }
    }

    let browser = launch_if_needed(deps, &runnable, options).await;
    let outcomes: Vec<(Account, Outcome)> = futures::stream::iter(runnable)
        .map(|account| {
            let browser = browser.as_ref();
            let emit = &emit;
            async move {
                let outcome = run_account(deps, &account, browser, options, cancel, emit).await;
                (account, outcome)
            }
        })
        .buffer_unordered(options.concurrency.max(1))
        .collect()
        .await;
    if let Some(Ok(session)) = browser {
        session.close().await;
    }

    for (account, outcome) in outcomes {
        match outcome {
            Outcome::Succeeded => summary.succeeded += 1,
            Outcome::Failed => {
                summary.failed += 1;
                summary.failed_ids.push(account.id);
                summary.failed_accounts.push(format!("{} {}", broker_name(deps, &account), account.client_id));
            }
            Outcome::Cancelled => summary.cancelled = true,
        }
    }
    summary.cancelled |= cancel.is_cancelled();
    finish_run_row(deps, run_id, &summary);
    tracing::info!(run_id, succeeded = summary.succeeded, failed = summary.failed, skipped = summary.skipped, "run finished");
    emit(RunEvent::RunFinished { run_id, summary: summary.clone() });
    summary
}

/// Load the account and decide whether it can run; `Err` is a skip reason.
fn plan(deps: &RunnerDeps, id: i64) -> Result<Account, String> {
    let account = deps.with_accounts(|a| a.get(id)).map_err(|e| e.to_string())?;
    let manifest = deps.bundle.get(&account.broker_id).ok_or("This broker isn't supported by this version.")?;
    if manifest.availability == Availability::ComingSoon {
        return Err(format!("{} support is coming soon.", manifest.name));
    }
    if !deps.bundle.has_tenant(&account.tenant_id) {
        return Err("Unknown Cirrus workspace.".into());
    }
    let missing = validate::missing_secrets(manifest, &account.secret_keys);
    if !missing.is_empty() {
        let labels: Vec<&str> = missing.iter().filter_map(|k| manifest.field(k)).map(|f| f.label.as_str()).collect();
        return Err(format!("Needs setup: add {}.", labels.join(", ")));
    }
    Ok(account)
}

async fn launch_if_needed(deps: &RunnerDeps, accounts: &[Account], options: &RunOptions) -> Option<Result<ChromeSession, String>> {
    let needs_browser = accounts
        .iter()
        .filter_map(|a| deps.bundle.get(&a.broker_id))
        .any(|m| m.kind != BrokerKind::Http);
    if !needs_browser {
        return None;
    }
    let launch = LaunchOptions { headless: options.headless, executable: options.chrome_executable.clone() };
    Some(ChromeSession::launch(&launch).await.map_err(|e| e.to_string()))
}

async fn run_account<E>(
    deps: &RunnerDeps,
    account: &Account,
    browser: Option<&Result<ChromeSession, String>>,
    options: &RunOptions,
    cancel: &CancellationToken,
    emit: &E,
) -> Outcome
where
    E: Fn(RunEvent) + Send + Sync,
{
    let manifest = deps.bundle.get(&account.broker_id).expect("planned accounts have a manifest");
    let values = match deps.with_accounts(|a| a.login_values(account.id)) {
        Ok(values) => values,
        Err(error) => return finish(deps, account.id, Err(error.to_string()), emit),
    };
    if let Some(Err(launch_error)) = browser.filter(|_| manifest.kind != BrokerKind::Http) {
        return finish(deps, account.id, Err(format!("Could not start the browser: {launch_error}")), emit);
    }
    let redactor = Redactor::new(manifest.secret_keys().filter_map(|k| values.get(k)));
    let http = crate::broker::http_flows::client();
    let session = browser.and_then(|b| b.as_ref().ok());

    let max_attempts = options.retries + 1;
    for attempt_number in 1..=max_attempts {
        if cancel.is_cancelled() {
            return cancelled(account.id, emit);
        }
        emit(RunEvent::AccountStarted { account_id: account.id, attempt: attempt_number });
        let attempt = Attempt {
            manifest,
            bundle: &deps.bundle,
            tenant_id: &account.tenant_id,
            values: &values,
            redactor: &redactor,
            http: &http,
            browser: session,
            failure_stem: artifacts::failure_stem(&deps.data_dir, &account.broker_id, &account.client_id, chrono::Local::now()),
        };
        let result = tokio::select! {
            result = attempt.run() => result,
            () = cancel.cancelled() => return cancelled(account.id, emit),
        };
        match result {
            Ok(message) => return finish(deps, account.id, Ok(message), emit),
            Err(AttemptError { message, retryable: true }) if attempt_number < max_attempts => {
                tracing::warn!(account = account.id, attempt = attempt_number, %message, "attempt failed; retrying");
                tokio::time::sleep(RETRY_BACKOFF * attempt_number).await;
            }
            Err(AttemptError { message, .. }) => return finish(deps, account.id, Err(message), emit),
        }
    }
    unreachable!("the loop returns on the last attempt")
}

fn finish<E: Fn(RunEvent)>(deps: &RunnerDeps, account_id: i64, result: Result<String, String>, emit: &E) -> Outcome {
    let (record, outcome, ok, message) = match result {
        Ok(message) => (LoginResult::Success { at: Utc::now() }, Outcome::Succeeded, true, message),
        Err(message) => (LoginResult::Failure { message: message.clone() }, Outcome::Failed, false, message),
    };
    if let Err(error) = deps.with_accounts(|a| a.record_result(account_id, &record)) {
        tracing::error!(account = account_id, %error, "could not save login result");
    }
    tracing::info!(account = account_id, ok, "login finished");
    emit(RunEvent::AccountFinished { account_id, ok, message });
    outcome
}

fn cancelled<E: Fn(RunEvent)>(account_id: i64, emit: &E) -> Outcome {
    emit(RunEvent::AccountFinished { account_id, ok: false, message: "Stopped".into() });
    Outcome::Cancelled
}

fn broker_name(deps: &RunnerDeps, account: &Account) -> String {
    deps.bundle.get(&account.broker_id).map_or_else(|| account.broker_id.clone(), |m| m.name.clone())
}

fn start_run_row(deps: &RunnerDeps, trigger: Trigger) -> i64 {
    let conn = deps.conn.lock().expect("database lock poisoned");
    let inserted = conn.execute(
        "INSERT INTO runs (trigger, started_at) VALUES (?1, ?2)",
        params![trigger.as_db(), Utc::now().to_rfc3339()],
    );
    match inserted {
        Ok(_) => conn.last_insert_rowid(),
        Err(error) => {
            tracing::error!(%error, "could not record run start");
            0
        }
    }
}

fn finish_run_row(deps: &RunnerDeps, run_id: i64, summary: &RunSummary) {
    let conn = deps.conn.lock().expect("database lock poisoned");
    let updated = conn.execute(
        "UPDATE runs SET finished_at = ?1, succeeded = ?2, failed = ?3 WHERE id = ?4",
        params![Utc::now().to_rfc3339(), summary.succeeded as i64, summary.failed as i64, run_id],
    );
    if let Err(error) = updated {
        tracing::error!(%error, "could not record run finish");
    }
}

#[cfg(test)]
mod tests;
