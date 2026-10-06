//! System tray: the app keeps running there so scheduled runs happen with
//! the window closed.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Manager};

use super::run_control::{self, Selection};
use super::settings;
use super::state::AppState;
use crate::runner::Trigger;

const OPEN: &str = "open";
const LOGIN_ALL: &str = "login_all";
const LOGIN_FAILED: &str = "login_failed";
const QUIT: &str = "quit";

pub fn create(app: &App) -> tauri::Result<()> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, OPEN, "Open AutoLogin", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, LOGIN_ALL, "Log in all accounts", true, None::<&str>)?,
            &MenuItem::with_id(app, LOGIN_FAILED, "Retry failed accounts", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, QUIT, "Quit AutoLogin", true, None::<&str>)?,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("AutoLogin")
        .menu(&menu)
        .on_menu_event(|app, event| handle(app, event.id().as_ref()));
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn handle(app: &AppHandle, id: &str) {
    match id {
        OPEN => show(app),
        LOGIN_ALL | LOGIN_FAILED => {
            let selection = if id == LOGIN_ALL { Selection::All } else { Selection::Failed };
            // Tray runs follow the scheduled-run browser setting: hidden by default.
            let headless = app
                .state::<AppState>()
                .conn
                .lock()
                .map(|conn| settings::load(&conn).schedule.headless)
                .unwrap_or(true);
            if let Err(reason) = run_control::start(app, &selection, Trigger::Manual, headless) {
                tracing::warn!(%reason, "tray run not started");
            }
        }
        QUIT => app.exit(0),
        _ => {}
    }
}

pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
