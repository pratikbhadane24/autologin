//! App self-updates from GitHub releases (Tauri updater, signature-checked).
//!
//! Updates install automatically unless the user turned that off, but never
//! during a login run or shortly before the scheduled login: an update
//! restarts the app.

use std::time::Duration;

use chrono::Utc;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

use super::settings;
use super::state::AppState;

pub const UPDATE_EVENT: &str = "update-status";
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// Don't restart for an update this close to the scheduled login.
const QUIET_BEFORE_SCHEDULE_MINUTES: i64 = 30;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateStatus {
    UpToDate,
    Available { version: String, notes: Option<String> },
    Downloading { percent: Option<u8> },
    Failed { message: String },
}

/// Whether the build carries an updater public key (set before releasing).
fn updater_configured(app: &AppHandle) -> bool {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|cfg| cfg.get("pubkey"))
        .and_then(|key| key.as_str())
        .is_some_and(|key| !key.trim().is_empty())
}

pub fn spawn(app: AppHandle) {
    if !updater_configured(&app) {
        tracing::info!("app updates disabled: no updater public key in this build");
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_AFTER).await;
        loop {
            if let Err(message) = check(&app, false).await {
                tracing::warn!(%message, "update check failed");
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

fn emit(app: &AppHandle, status: &UpdateStatus) {
    if let Err(error) = app.emit(UPDATE_EVENT, status) {
        tracing::debug!(%error, "could not emit update status");
    }
}

/// Is now a good moment to restart for an update?
fn safe_to_restart(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    if state.is_running() {
        return false;
    }
    let Ok(conn) = state.conn.lock() else { return false };
    let saved = settings::load(&conn);
    if !saved.auto_update {
        return false;
    }
    let now = Utc::now();
    saved
        .schedule
        .next_run_after(now)
        .is_none_or(|next| (next - now).num_minutes() > QUIET_BEFORE_SCHEDULE_MINUTES)
}

/// Check for an update; install it when `force` (user clicked) or when it is
/// safe to restart automatically.
pub async fn check(app: &AppHandle, force: bool) -> Result<UpdateStatus, String> {
    if !updater_configured(app) {
        return Err("Updates aren't available in this build.".into());
    }
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        emit(app, &UpdateStatus::UpToDate);
        return Ok(UpdateStatus::UpToDate);
    };
    let available = UpdateStatus::Available { version: update.version.clone(), notes: update.body.clone() };
    tracing::info!(version = %update.version, "update available");
    emit(app, &available);
    if !force && !safe_to_restart(app) {
        return Ok(available);
    }

    let progress_app = app.clone();
    let mut downloaded: usize = 0;
    let mut last_percent = None;
    let result = update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk;
                let percent = total.map(|t| ((downloaded as u64 * 100) / t.max(1)).min(100) as u8);
                if percent != last_percent {
                    last_percent = percent;
                    emit(&progress_app, &UpdateStatus::Downloading { percent });
                }
            },
            || tracing::info!("update downloaded"),
        )
        .await;
    match result {
        Ok(()) => {
            tracing::info!(version = %update.version, "update installed; restarting");
            app.restart();
        }
        Err(error) => {
            let status = UpdateStatus::Failed { message: error.to_string() };
            emit(app, &status);
            Err(error.to_string())
        }
    }
}
