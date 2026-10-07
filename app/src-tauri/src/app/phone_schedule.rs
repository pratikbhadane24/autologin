//! The daily login on phones. A timer can't run while Android has the app
//! closed, so the upcoming run times are handed to native alarms
//! (LoginAlarms.kt). When one fires, the user taps its notification (or,
//! in automatic mode, AutoLogin opens by itself) and the run starts here.

use std::sync::OnceLock;
use std::time::Duration;

use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{AppHandle, Manager, Wry};

use super::schedule_service;
use super::settings;
use super::state::AppState;

/// Alarms set ahead; re-sent every time the app starts or the schedule changes.
const UPCOMING_RUNS: usize = 30;
/// How often to check for an alarm that fired while AutoLogin was open.
const POLL: Duration = Duration::from_secs(15);

static PLUGIN: OnceLock<PluginHandle<Wry>> = OnceLock::new();
/// The background poll and the app coming to the front can both ask at once.
static TAKING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn init() -> TauriPlugin<Wry> {
    Builder::new("login-schedule")
        .setup(|_app, api| {
            #[cfg(target_os = "android")]
            {
                let handle = api.register_android_plugin("trade.autologin.autologin", "LoginSchedulePlugin")?;
                let _ = PLUGIN.set(handle);
            }
            #[cfg(not(target_os = "android"))]
            let _ = api;
            Ok(())
        })
        .build()
}

async fn call<T: DeserializeOwned>(command: &str, payload: impl Serialize) -> Result<T, String> {
    let plugin = PLUGIN.get().ok_or("Daily login on this phone isn't supported yet.")?;
    plugin.run_mobile_plugin_async(command, payload).await.map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct Value<T> {
    value: T,
}

/// What automatic mode still needs; shown as a checklist in Settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
pub struct PhoneStatus {
    pub notifications: bool,
    pub exact_alarms: bool,
    pub overlay: bool,
    pub battery_unrestricted: bool,
}

pub async fn status() -> Result<PhoneStatus, String> {
    call("status", ()).await
}

pub async fn open_setting(which: &str) -> Result<(), String> {
    #[derive(Serialize)]
    struct Which<'a> {
        which: &'a str,
    }
    call::<IgnoredAny>("openSetting", Which { which }).await.map(|_| ())
}

/// Keeps the alarms in step with the schedule and starts runs they request.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            sync_alarms(&app).await;
            start_if_requested(&app).await;
            let state = app.state::<AppState>();
            tokio::select! {
                () = tokio::time::sleep(POLL) => {}
                () = state.schedule_changed.notified() => tracing::debug!("schedule changed"),
            }
        }
    });
}

async fn sync_alarms(app: &AppHandle) {
    let schedule = {
        let state = app.state::<AppState>();
        let Ok(conn) = state.conn.lock() else { return };
        settings::load(&conn).schedule
    };
    #[derive(Serialize)]
    struct Alarms {
        times: Vec<i64>,
        automatic: bool,
    }
    let times = schedule.upcoming_runs(chrono::Utc::now(), UPCOMING_RUNS).iter().map(|t| t.timestamp_millis()).collect();
    if let Err(error) = call::<IgnoredAny>("setSchedule", Alarms { times, automatic: schedule.phone_automatic }).await {
        tracing::warn!(%error, "could not set the daily login alarm");
    }
}

/// Starts the scheduled run if an alarm (or its notification) asked for one.
pub async fn start_if_requested(app: &AppHandle) {
    let Ok(_only_one) = TAKING.try_lock() else { return };
    match call::<Value<bool>>("takePendingRun", ()).await {
        Ok(Value { value: true }) => {
            let headless = {
                let state = app.state::<AppState>();
                let Ok(conn) = state.conn.lock() else { return };
                settings::load(&conn).schedule.headless
            };
            tracing::info!("scheduled run requested by the daily alarm");
            schedule_service::record_scheduled_run(app);
            let app = app.clone();
            tauri::async_runtime::spawn(async move { schedule_service::start_scheduled(&app, headless).await });
        }
        Ok(_) => {}
        Err(error) => tracing::debug!(%error, "could not check for a requested run"),
    }
}
