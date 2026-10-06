//! Background task that fires scheduled runs.
//!
//! Waits in short slices and re-reads the wall clock each time, because
//! timers can stall while the computer sleeps. A due run is detected with
//! `Schedule::missed_run`, which also covers waking up after the time.

use std::time::Duration;

use chrono::{DateTime, Utc};
use std::sync::Arc;

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use super::run_control::{self, Selection};
use super::settings;
use super::state::AppState;
use crate::runner::{RunSummary, Trigger};
use crate::store::settings as kv;

const LAST_RUN_KEY: &str = "schedule.last_run";
/// How long a scheduled run waits for the user to answer a keychain prompt.
const UNLOCK_WAIT: Duration = Duration::from_secs(120);
/// Upper bound on one wait, so clock jumps and sleep/wake are noticed quickly.
const MAX_WAIT: Duration = Duration::from_secs(60);

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let wait = tick(&app).await;
            let state = app.state::<AppState>();
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = state.schedule_changed.notified() => tracing::debug!("schedule changed"),
            }
        }
    });
}

/// Fire a due run if there is one; return how long to wait before checking again.
async fn tick(app: &AppHandle) -> Duration {
    let state = app.state::<AppState>();
    let (schedule, last_run) = {
        let Ok(conn) = state.conn.lock() else { return MAX_WAIT };
        let last: Option<DateTime<Utc>> = kv::get(&conn, LAST_RUN_KEY).ok().flatten();
        (settings::load(&conn).schedule, last)
    };
    let now = Utc::now();

    if let Some(due) = schedule.missed_run(now, last_run) {
        if let Ok(conn) = state.conn.lock() {
            if let Err(error) = kv::set(&conn, LAST_RUN_KEY, &now) {
                tracing::error!(%error, "could not record scheduled run");
            }
        }
        tracing::info!(%due, "scheduled run due");
        if ensure_unlocked(app).await {
            if let Err(reason) = run_control::start(app, &Selection::All, Trigger::Scheduled, schedule.headless) {
                tracing::warn!(%reason, "scheduled run not started");
            }
        }
    }

    schedule
        .next_run_after(now)
        .and_then(|next| (next - now).to_std().ok())
        .map_or(MAX_WAIT, |until_next| until_next.min(MAX_WAIT))
}

/// Make sure saved passwords can be read without blocking forever on an
/// unanswered OS permission prompt. Returns false (and notifies) on timeout.
async fn ensure_unlocked(app: &AppHandle) -> bool {
    let secrets = Arc::clone(&app.state::<AppState>().secrets);
    if secrets.is_unlocked() {
        return true;
    }
    let unlocking = tauri::async_runtime::spawn_blocking(move || secrets.unlock());
    let message = match tokio::time::timeout(UNLOCK_WAIT, unlocking).await {
        Ok(Ok(Ok(()))) => return true,
        Ok(Ok(Err(error))) => error.to_string(),
        Ok(Err(_)) | Err(_) => {
            "AutoLogin needs permission to read your saved passwords. Open AutoLogin and choose Always Allow.".to_string()
        }
    };
    tracing::warn!(%message, "scheduled run skipped: saved passwords locked");
    if let Err(error) = app.notification().builder().title("AutoLogin couldn't log in").body(&message).show() {
        tracing::debug!(%error, "notification failed");
    }
    false
}

/// After a scheduled run, re-run the failed accounts once if the user asked.
pub fn schedule_retry(app: &AppHandle, summary: &RunSummary) {
    if summary.failed_ids.is_empty() || summary.cancelled {
        return;
    }
    let state = app.state::<AppState>();
    let schedule = match state.conn.lock() {
        Ok(conn) => settings::load(&conn).schedule,
        Err(_) => return,
    };
    let Some(minutes) = schedule.retry_failed_after_minutes else { return };
    let ids = summary.failed_ids.clone();
    let app = app.clone();
    tracing::info!(count = ids.len(), minutes, "retrying failed accounts later");
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(u64::from(minutes) * 60)).await;
        if let Err(reason) = run_control::start(&app, &Selection::Ids(ids), Trigger::Retry, schedule.headless) {
            tracing::warn!(%reason, "retry run not started");
        }
    });
}
