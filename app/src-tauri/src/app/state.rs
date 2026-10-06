//! State shared by all commands and background services.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use rusqlite::Connection;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::broker::registry::ManifestBundle;
use crate::migrate_v1::MigrationOutcome;
use crate::runner::RunnerDeps;
use crate::store::secrets::SecretStore;

pub struct AppState {
    pub conn: Arc<Mutex<Connection>>,
    pub secrets: Arc<dyn SecretStore>,
    /// Broker manifests; replaced in place when a newer signed copy arrives.
    pub bundle: ArcSwap<ManifestBundle>,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    /// Cancel token of the run in progress; `None` when idle.
    pub active_run: Mutex<Option<CancellationToken>>,
    /// Result of the v1 import done at this launch, shown once by the UI.
    pub migration: Mutex<Option<MigrationOutcome>>,
    /// Accounts received through an `autologin://import` link, waiting for the UI.
    pub pending_import: Mutex<Option<String>>,
    /// Wakes the scheduler when the user changes the schedule.
    pub schedule_changed: Notify,
    pub app_version: String,
}

impl AppState {
    /// The current broker manifests. Hold the snapshot for a whole operation
    /// so a background update can't change brokers halfway through.
    pub fn bundle(&self) -> Arc<ManifestBundle> {
        self.bundle.load_full()
    }

    /// Everything a run needs, with a fixed manifest snapshot.
    pub fn runner_deps(&self) -> RunnerDeps {
        RunnerDeps {
            conn: Arc::clone(&self.conn),
            secrets: Arc::clone(&self.secrets),
            bundle: self.bundle(),
            data_dir: self.data_dir.clone(),
        }
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    pub fn is_running(&self) -> bool {
        self.active_run.lock().expect("lock").is_some()
    }
}
