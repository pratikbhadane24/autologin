//! Desktop app wiring: startup, state, commands, tray and scheduler.

pub mod commands;
pub mod deep_link;
pub mod dev;
pub mod manifest_service;
pub mod run_control;
pub mod schedule_service;
pub mod settings;
pub mod state;
pub mod tray;
pub mod update_service;
pub mod views;

use std::sync::{Arc, Mutex};

use tauri::{App, Manager};
use tokio::sync::Notify;

use crate::broker::registry::ManifestBundle;
use crate::store::accounts::Accounts;
use crate::store::secrets::SecretStore;
use crate::store::vault::{KeychainMasterKey, VaultStore};
use crate::{logging, migrate_v1, store};
use state::AppState;

/// Keeps the log writer alive for the app's lifetime.
struct LogGuard(#[allow(dead_code)] Mutex<tracing_appender::non_blocking::WorkerGuard>);

pub fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let dev = dev::options();
    let (data_dir, log_dir) = match &dev.data_dir {
        Some(dir) => (dir.clone(), dir.join("logs")),
        None => (app.path().app_data_dir()?, app.path().app_log_dir()?),
    };
    std::fs::create_dir_all(&data_dir)?;
    std::fs::create_dir_all(&log_dir)?;
    let guard = logging::init(&log_dir)?;
    app.manage(LogGuard(Mutex::new(guard)));
    let version = app.package_info().version.to_string();
    tracing::info!(%version, data = %data_dir.display(), "AutoLogin starting");

    let conn = store::db::open(&data_dir)?;
    let bundle = manifest_service::initial_bundle(&data_dir)?;
    tracing::info!(manifest_version = bundle.index.manifest_version, "broker manifests loaded");
    // One keychain item (the vault key) for all accounts: one permission
    // prompt per app version instead of one per account.
    let secrets: Arc<dyn SecretStore> = if dev.memory_secrets {
        tracing::warn!("development: secrets kept in memory only");
        Arc::new(crate::store::secrets::MemoryStore::default())
    } else {
        Arc::new(VaultStore::new(Box::new(KeychainMasterKey)))
    };
    let migration = migrate_from_v1(&conn, secrets.as_ref(), &bundle);

    // Unlock the vault now, so any OS permission prompt (first run, or after
    // an update) appears at launch rather than during the scheduled login.
    let unlocker = Arc::clone(&secrets);
    std::thread::spawn(move || {
        if let Err(error) = unlocker.unlock() {
            tracing::warn!(%error, "could not unlock saved passwords at launch");
        }
    });

    app.manage(AppState {
        conn: Arc::new(Mutex::new(conn)),
        secrets,
        bundle: arc_swap::ArcSwap::from_pointee(bundle),
        data_dir,
        log_dir,
        active_run: Mutex::new(None),
        migration: Mutex::new(migration),
        pending_import: Mutex::new(None),
        schedule_changed: Notify::new(),
        app_version: version,
    });
    tray::create(app)?;
    register_app_links(app);
    // Windows: remove the AutoLogin 1.x program (its data is migrated above).
    // Runs every launch; it's a quick registry check once v1 is gone.
    std::thread::spawn(|| {
        let removed = crate::v1_uninstall::remove_v1();
        if !removed.is_empty() {
            tracing::info!(count = removed.len(), "AutoLogin 1.x uninstalled");
        }
    });
    schedule_service::spawn(app.handle().clone());
    manifest_service::spawn(app.handle().clone());
    update_service::spawn(app.handle().clone());
    if dev.run_all_then_quit {
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            tracing::warn!("development: starting a run of all accounts");
            if let Err(reason) = run_control::start(&handle, &run_control::Selection::All, crate::runner::Trigger::Manual, false) {
                tracing::error!(%reason, "development run not started");
                handle.exit(1);
            }
        });
    }
    Ok(())
}

fn migrate_from_v1(
    conn: &rusqlite::Connection,
    secrets: &dyn SecretStore,
    bundle: &ManifestBundle,
) -> Option<migrate_v1::MigrationOutcome> {
    let v1_dir = migrate_v1::v1_data_dir()?;
    let accounts = Accounts::new(conn, secrets, bundle);
    match migrate_v1::run(&v1_dir, conn, &accounts, bundle) {
        Ok(Some(outcome)) => {
            if let Some(background) = outcome.background_login {
                let current = settings::load(conn);
                let carried = settings::AppSettings { manual_headless: background, ..current };
                if let Err(error) = settings::save(conn, &carried) {
                    tracing::warn!(%error, "could not carry over v1 browser preference");
                }
            }
            Some(outcome)
        }
        Ok(None) => None,
        Err(error) => {
            tracing::error!(%error, "v1 migration failed; v1 data left untouched");
            None
        }
    }
}

/// `autologin://` links: register the scheme (Windows/Linux need it at
/// runtime; macOS gets it from the bundle) and handle the launch link and
/// later ones. Single-instance forwards links from a second launch here.
fn register_app_links(app: &App) {
    use tauri_plugin_deep_link::DeepLinkExt;
    let links = app.deep_link();
    #[cfg(any(windows, target_os = "linux"))]
    if let Err(error) = links.register_all() {
        tracing::warn!(%error, "could not register autologin:// links");
    }
    let handle = app.handle().clone();
    links.on_open_url(move |event| deep_link::handle(&handle, event.urls()));
    match links.get_current() {
        Ok(Some(urls)) => deep_link::handle(app.handle(), urls),
        Ok(None) => {}
        Err(error) => tracing::debug!(%error, "no launch link"),
    }
}
