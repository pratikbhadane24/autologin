pub mod app;
pub mod broker;
pub mod browser;
pub mod cirrus;
pub mod datetime;
pub mod logging;
pub mod migrate_v1;
pub mod paste;
pub mod runner;
pub mod scheduler;
pub mod session;
pub mod store;
pub mod totp;
pub mod v1_uninstall;

#[cfg(desktop)]
use tauri::{Manager, WindowEvent};
#[cfg(desktop)]
use tauri_plugin_autostart::MacosLauncher;

use app::commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = desktop_plugins(tauri::Builder::default())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            app::setup(app)?;
            // Started at login by autostart: stay in the tray.
            #[cfg(desktop)]
            if std::env::args().any(|a| a == "--hidden") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        });
    #[cfg(desktop)]
    let builder = builder.on_window_event(|window, event| {
        // Closing hides to the tray so scheduled runs keep working.
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window.hide();
        }
    });
    builder
        .invoke_handler(tauri::generate_handler![
            commands::get_catalog,
            commands::list_accounts,
            commands::create_account,
            commands::update_account,
            commands::add_accounts,
            commands::delete_accounts,
            commands::parse_paste,
            commands::run_accounts,
            commands::stop_run,
            commands::is_running,
            commands::get_settings,
            commands::get_settings_view,
            commands::save_settings,
            commands::acknowledge_privacy,
            commands::mark_whats_new_seen,
            commands::export_accounts,
            commands::inspect_import,
            commands::import_accounts,
            commands::take_migration_outcome,
            commands::take_pending_import,
            commands::app_info,
            commands::recent_logs,
            commands::open_folder,
            commands::check_for_update,
            commands::show_main_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AutoLogin");
}

/// Plugins that only exist on desktop: single instance (a second launch
/// focuses the first; also forwards autologin:// links), self-update and
/// start-with-computer.
#[cfg(desktop)]
fn desktop_plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    // A separate development data folder runs as its own instance.
    let builder = if app::dev::options().data_dir.is_some() {
        builder
    } else {
        builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| app::show_main_window(app)))
    };
    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
}

#[cfg(mobile)]
fn desktop_plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
}
