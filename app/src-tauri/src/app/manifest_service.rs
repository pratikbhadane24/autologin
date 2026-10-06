//! Keeps broker manifests current: newest of bundled vs cached at startup,
//! then a signed remote check every few hours. A broker page change can be
//! fixed without an app release.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use super::state::AppState;
use crate::broker::registry::{ManifestBundle, RegistryError};
use crate::broker::remote::{self, DEFAULT_MANIFEST_URL, TRUSTED_MANIFEST_KEYS};

pub const CATALOG_UPDATED_EVENT: &str = "catalog-updated";
const REFRESH_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const CACHE_DIR: &str = "manifests";

fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(CACHE_DIR)
}

pub fn initial_bundle(data_dir: &Path) -> Result<ManifestBundle, RegistryError> {
    let cached = remote::load_cached(&cache_dir(data_dir), TRUSTED_MANIFEST_KEYS);
    Ok(remote::choose(ManifestBundle::bundled()?, cached))
}

pub fn spawn(app: AppHandle) {
    if TRUSTED_MANIFEST_KEYS.is_empty() {
        tracing::info!("remote broker updates disabled: no trusted manifest key configured");
        return;
    }
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::new();
        let mut interval = tokio::time::interval(REFRESH_EVERY);
        loop {
            interval.tick().await;
            refresh_once(&app, &client).await;
        }
    });
}

async fn refresh_once(app: &AppHandle, client: &reqwest::Client) {
    let state = app.state::<AppState>();
    let current = state.bundle().index.manifest_version;
    let dir = cache_dir(&state.data_dir);
    match remote::refresh(client, DEFAULT_MANIFEST_URL, &dir, TRUSTED_MANIFEST_KEYS, current).await {
        Ok(Some(newer)) => {
            let version = newer.index.manifest_version;
            state.bundle.store(Arc::new(newer));
            tracing::info!(from = current, to = version, "broker manifests updated");
            if let Err(error) = app.emit(CATALOG_UPDATED_EVENT, version) {
                tracing::debug!(%error, "could not notify UI of broker update");
            }
        }
        Ok(None) => tracing::debug!(current, "broker manifests up to date"),
        Err(error) => tracing::warn!(%error, "broker manifest refresh failed"),
    }
}
