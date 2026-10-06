//! Starting and stopping runs (from the UI, the tray and the scheduler), and
//! the end-of-run notification.

use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

use super::settings;
use super::state::AppState;
use crate::runner::{self, RunEvent, RunOptions, RunSummary, Trigger};
use crate::store::accounts::{Accounts, LoginStatus};

pub const RUN_EVENT: &str = "run-event";
const MAX_NAMES_IN_NOTIFICATION: usize = 3;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", content = "ids", rename_all = "snake_case")]
pub enum Selection {
    All,
    /// Accounts whose last login failed.
    Failed,
    /// Accounts not currently logged in (failed, logged out or expired).
    NotLoggedIn,
    Ids(Vec<i64>),
}

fn resolve(state: &AppState, selection: &Selection) -> Result<Vec<i64>, String> {
    if let Selection::Ids(ids) = selection {
        return Ok(ids.clone());
    }
    let bundle = state.bundle();
    let conn = state.conn.lock().map_err(|_| "database is busy")?;
    let accounts = Accounts::new(&conn, state.secrets.as_ref(), &bundle).list().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now();
    Ok(accounts
        .into_iter()
        .filter(|a| match selection {
            Selection::All | Selection::Ids(_) => true,
            Selection::Failed => a.status == LoginStatus::Failed,
            Selection::NotLoggedIn => {
                a.effective_status(bundle.get(&a.broker_id), now) != LoginStatus::LoggedIn
            }
        })
        .map(|a| a.id)
        .collect())
}

/// Start a run in the background. Only one run at a time.
pub fn start(app: &AppHandle, selection: &Selection, trigger: Trigger, headless: bool) -> Result<usize, String> {
    let state = app.state::<AppState>();
    let ids = resolve(&state, selection)?;
    if ids.is_empty() {
        return Err("No accounts to log in.".into());
    }
    let token = {
        let mut active = state.active_run.lock().map_err(|_| "busy")?;
        if active.is_some() {
            return Err("A login run is already in progress.".into());
        }
        let token = CancellationToken::new();
        *active = Some(token.clone());
        token
    };
    let saved = {
        let conn = state.conn.lock().map_err(|_| "database is busy")?;
        settings::load(&conn)
    };
    let options = RunOptions { retries: saved.retries, concurrency: saved.concurrency, ..RunOptions::new(trigger, headless) };
    let deps = state.runner_deps();
    let app = app.clone();
    let count = ids.len();
    tracing::info!(?trigger, count, headless, "starting run");

    tauri::async_runtime::spawn(async move {
        let emitter = app.clone();
        let summary = runner::run(&deps, &ids, &options, &token, move |event: RunEvent| {
            if let Err(error) = emitter.emit(RUN_EVENT, &event) {
                tracing::debug!(%error, "could not emit run event");
            }
        })
        .await;
        if let Ok(mut active) = app.state::<AppState>().active_run.lock() {
            *active = None;
        }
        notify(&app, trigger, &summary);
        if super::dev::options().run_all_then_quit {
            tracing::warn!(?summary, "development run finished; quitting");
            app.exit(0);
        }
        if trigger == Trigger::Scheduled {
            super::schedule_service::schedule_retry(&app, &summary);
        }
    });
    Ok(count)
}

pub fn stop(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    let active = state.active_run.lock().expect("lock");
    match active.as_ref() {
        Some(token) => {
            token.cancel();
            tracing::info!("run stop requested");
            true
        }
        None => false,
    }
}

/// Notify unless the user is looking at the window (they see it live).
fn notify(app: &AppHandle, trigger: Trigger, summary: &RunSummary) {
    let window_focused = app.get_webview_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false);
    if window_focused && trigger == Trigger::Manual {
        return;
    }
    super::notify(app, "AutoLogin", &summary_text(summary));
}

pub fn summary_text(summary: &RunSummary) -> String {
    if summary.cancelled {
        return format!("Stopped. {} logged in, {} failed.", summary.succeeded, summary.failed);
    }
    if summary.failed == 0 {
        return format!("All {} accounts logged in.", summary.succeeded);
    }
    let names: Vec<&str> = summary.failed_accounts.iter().take(MAX_NAMES_IN_NOTIFICATION).map(String::as_str).collect();
    let more = summary.failed.saturating_sub(names.len());
    let suffix = if more > 0 { format!(" and {more} more") } else { String::new() };
    format!("{} logged in, {} failed: {}{suffix}", summary.succeeded, summary.failed, names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_text_variants() {
        let ok = RunSummary { succeeded: 5, ..RunSummary::default() };
        assert_eq!(summary_text(&ok), "All 5 accounts logged in.");
        let failed = RunSummary {
            succeeded: 2,
            failed: 4,
            failed_accounts: vec!["Zerodha A".into(), "Upstox B".into(), "Pocketful C".into(), "Zerodha D".into()],
            ..RunSummary::default()
        };
        assert_eq!(summary_text(&failed), "2 logged in, 4 failed: Zerodha A, Upstox B, Pocketful C and 1 more");
        let stopped = RunSummary { cancelled: true, succeeded: 1, ..RunSummary::default() };
        assert_eq!(summary_text(&stopped), "Stopped. 1 logged in, 0 failed.");
    }
}
