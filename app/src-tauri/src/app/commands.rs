//! Tauri commands: the only entry points the UI can call.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

use super::run_control::{self, Selection};
use super::settings::{self, AppSettings};
use super::state::AppState;
use super::views::{self, AccountView, Catalog};
use crate::migrate_v1::MigrationOutcome;
use crate::paste::{self, PasteResult};
use crate::runner::artifacts::FAILURES_DIR;
use crate::runner::Trigger;
use crate::store::accounts::{AccountError, AccountInput, Accounts};
use crate::store::backup::{self, FileKind};
use crate::store::transfer::{self, ImportReport};
use crate::store::validate::{Completeness, FieldErrors};

/// Largest file accepted for import (backups are a few KB per account).
const MAX_IMPORT_BYTES: u64 = 10 * 1024 * 1024;
const MAX_LOG_LINES: usize = 2_000;

#[derive(Debug, Serialize)]
pub struct CommandError {
    pub message: String,
    /// Per-field problems for forms.
    pub fields: Option<FieldErrors>,
}

impl CommandError {
    fn msg(message: impl Into<String>) -> Self {
        Self { message: message.into(), fields: None }
    }
}

impl From<AccountError> for CommandError {
    fn from(error: AccountError) -> Self {
        match error {
            AccountError::Invalid(fields) => Self { message: "Please fix the highlighted fields.".into(), fields: Some(fields) },
            other => Self::msg(other.to_string()),
        }
    }
}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self::msg(message)
    }
}

type CmdResult<T> = Result<T, CommandError>;

fn with_accounts<T>(state: &AppState, f: impl FnOnce(&Accounts<'_>) -> Result<T, AccountError>) -> CmdResult<T> {
    let bundle = state.bundle();
    let conn = state.conn.lock().map_err(|_| CommandError::msg("database is busy"))?;
    Ok(f(&Accounts::new(&conn, state.secrets.as_ref(), &bundle))?)
}

fn load_settings(state: &AppState) -> CmdResult<AppSettings> {
    let conn = state.conn.lock().map_err(|_| CommandError::msg("database is busy"))?;
    Ok(settings::load(&conn))
}

// ---- catalog & accounts ----

#[tauri::command]
pub fn get_catalog(state: State<'_, AppState>) -> Catalog {
    views::catalog(&state.bundle())
}

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> CmdResult<Vec<AccountView>> {
    let accounts = with_accounts(&state, |a| a.list())?;
    let bundle = state.bundle();
    Ok(accounts.into_iter().map(|a| views::account_view(a, &bundle)).collect())
}

#[tauri::command]
pub fn create_account(state: State<'_, AppState>, input: AccountInput) -> CmdResult<AccountView> {
    let account = with_accounts(&state, |a| a.create(&input, Completeness::Strict))?;
    Ok(views::account_view(account, &state.bundle()))
}

#[derive(Debug, Serialize)]
pub struct BulkRowError {
    /// Position in the submitted list (0-based).
    pub index: usize,
    pub error: CommandError,
}

#[derive(Debug, Serialize)]
pub struct BulkResult {
    pub added: Vec<AccountView>,
    pub errors: Vec<BulkRowError>,
}

/// Add several accounts at once (bulk paste from Cirrus). Rows whose secrets
/// are left blank are still added and shown as "Needs setup".
#[tauri::command]
pub fn add_accounts(state: State<'_, AppState>, inputs: Vec<AccountInput>) -> CmdResult<BulkResult> {
    let bundle = state.bundle();
    let outcomes = with_accounts(&state, |accounts| {
        Ok(inputs
            .iter()
            .map(|input| accounts.create(input, Completeness::AllowMissingSecrets))
            .collect::<Vec<_>>())
    })?;
    let mut result = BulkResult { added: Vec::new(), errors: Vec::new() };
    for (index, outcome) in outcomes.into_iter().enumerate() {
        match outcome {
            Ok(account) => result.added.push(views::account_view(account, &bundle)),
            Err(error) => result.errors.push(BulkRowError { index, error: error.into() }),
        }
    }
    tracing::info!(added = result.added.len(), failed = result.errors.len(), "bulk add");
    Ok(result)
}

#[tauri::command]
pub fn update_account(state: State<'_, AppState>, id: i64, input: AccountInput) -> CmdResult<AccountView> {
    let account = with_accounts(&state, |a| a.update(id, &input, Completeness::Strict))?;
    Ok(views::account_view(account, &state.bundle()))
}

#[tauri::command]
pub fn delete_accounts(state: State<'_, AppState>, ids: Vec<i64>) -> CmdResult<usize> {
    if state.is_running() {
        return Err(CommandError::msg("Wait for the current run to finish before deleting accounts."));
    }
    with_accounts(&state, |a| a.delete(&ids))
}

#[tauri::command]
pub fn parse_paste(state: State<'_, AppState>, text: String) -> CmdResult<PasteResult> {
    paste::parse(&text, &state.bundle()).map_err(|e| CommandError::msg(e.to_string()))
}

// ---- runs ----

#[tauri::command]
pub fn run_accounts(app: AppHandle, state: State<'_, AppState>, selection: Selection, headless: Option<bool>) -> CmdResult<usize> {
    let headless = match headless {
        Some(value) => value,
        None => load_settings(&state)?.manual_headless,
    };
    Ok(run_control::start(&app, &selection, Trigger::Manual, headless)?)
}

#[tauri::command]
pub fn stop_run(app: AppHandle) -> bool {
    run_control::stop(&app)
}

#[tauri::command]
pub fn is_running(state: State<'_, AppState>) -> bool {
    state.is_running()
}

// ---- settings ----

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> CmdResult<AppSettings> {
    load_settings(&state)
}

#[derive(Debug, Serialize)]
pub struct SettingsView {
    pub settings: AppSettings,
    pub next_scheduled_run: Option<chrono::DateTime<chrono::Utc>>,
    pub recommendation: &'static str,
}

#[tauri::command]
pub fn get_settings_view(state: State<'_, AppState>) -> CmdResult<SettingsView> {
    let settings = load_settings(&state)?;
    Ok(SettingsView {
        next_scheduled_run: settings.schedule.next_run_after(chrono::Utc::now()),
        recommendation: crate::scheduler::RECOMMENDATION,
        settings,
    })
}

#[tauri::command]
pub fn save_settings(app: AppHandle, state: State<'_, AppState>, settings: AppSettings) -> CmdResult<SettingsView> {
    let saved = {
        let conn = state.conn.lock().map_err(|_| CommandError::msg("database is busy"))?;
        settings::save(&conn, &settings).map_err(|e| CommandError::msg(e.to_string()))?
    };
    let autostart = app.autolaunch();
    let result = if saved.start_with_computer { autostart.enable() } else { autostart.disable() };
    if let Err(error) = result {
        tracing::warn!(%error, "could not change start-with-computer");
    }
    state.schedule_changed.notify_one();
    get_settings_view(state)
}

#[tauri::command]
pub fn acknowledge_privacy(state: State<'_, AppState>) -> CmdResult<()> {
    update_settings(&state, |s| AppSettings { privacy_acknowledged: true, ..s })
}

#[tauri::command]
pub fn mark_whats_new_seen(state: State<'_, AppState>) -> CmdResult<()> {
    let version = state.app_version.clone();
    update_settings(&state, |s| AppSettings { last_seen_version: Some(version), ..s })
}

fn update_settings(state: &AppState, change: impl FnOnce(AppSettings) -> AppSettings) -> CmdResult<()> {
    let conn = state.conn.lock().map_err(|_| CommandError::msg("database is busy"))?;
    settings::save(&conn, &change(settings::load(&conn))).map(|_| ()).map_err(|e| CommandError::msg(e.to_string()))
}

// ---- export / import ----

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Encrypted,
    Plain,
    Csv,
}

#[tauri::command]
pub fn export_accounts(state: State<'_, AppState>, path: PathBuf, format: ExportFormat, password: Option<String>) -> CmdResult<usize> {
    let data = with_accounts(&state, |a| transfer::collect_backup(a, &state.app_version))?;
    let count = data.accounts.len();
    let text = match format {
        ExportFormat::Encrypted => {
            let password = password.ok_or_else(|| CommandError::msg("Enter a password for the backup."))?;
            backup::export_encrypted(&data, &password).map_err(|e| CommandError::msg(e.to_string()))?
        }
        ExportFormat::Plain => backup::export_plain(&data),
        ExportFormat::Csv => backup::export_csv(&data.accounts),
    };
    write_private(&path, &text).map_err(|e| CommandError::msg(format!("Could not save the file: {e}")))?;
    tracing::info!(count, ?format, "accounts exported");
    Ok(count)
}

/// Write a file readable only by the current user where the OS supports it.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn read_import(path: &Path) -> CmdResult<String> {
    let size = std::fs::metadata(path).map_err(|e| CommandError::msg(format!("Could not open the file: {e}")))?.len();
    if size > MAX_IMPORT_BYTES {
        return Err(CommandError::msg("This file is too large to be an AutoLogin backup."));
    }
    std::fs::read_to_string(path).map_err(|e| CommandError::msg(format!("Could not read the file: {e}")))
}

#[tauri::command]
pub fn inspect_import(path: PathBuf) -> CmdResult<FileKind> {
    backup::detect(&read_import(&path)?).map_err(|e| CommandError::msg(e.to_string()))
}

#[tauri::command]
pub fn import_accounts(state: State<'_, AppState>, path: PathBuf, password: Option<String>) -> CmdResult<ImportReport> {
    let text = read_import(&path)?;
    let bundle = &state.bundle();
    let (entries, mut problems) = match backup::detect(&text).map_err(|e| CommandError::msg(e.to_string()))? {
        FileKind::Csv => transfer::parse_csv(&text, bundle).map_err(|e| CommandError::msg(e.to_string()))?,
        FileKind::Encrypted | FileKind::Plain => {
            let data = backup::read_backup(&text, password.as_deref()).map_err(|e| CommandError::msg(e.to_string()))?;
            (data.accounts, Vec::new())
        }
    };
    let mut report = with_accounts(&state, |a| Ok(transfer::apply(a, bundle, &entries)))?;
    problems.append(&mut report.problems);
    Ok(ImportReport { problems, ..report })
}

/// Signed copy text received through an `autologin://import` link, if any.
#[tauri::command]
pub fn take_pending_import(state: State<'_, AppState>) -> Option<String> {
    state.pending_import.lock().ok().and_then(|mut p| p.take())
}

#[tauri::command]
pub fn take_migration_outcome(state: State<'_, AppState>) -> Option<MigrationOutcome> {
    state.migration.lock().ok().and_then(|mut m| m.take())
}

// ---- app info, logs, folders ----

#[derive(Debug, Serialize)]
pub struct AppInfo {
    pub version: String,
    pub manifest_version: u64,
    pub privacy_acknowledged: bool,
    pub show_whats_new: bool,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> CmdResult<AppInfo> {
    let settings = load_settings(&state)?;
    Ok(AppInfo {
        version: state.app_version.clone(),
        manifest_version: state.bundle().index.manifest_version,
        privacy_acknowledged: settings.privacy_acknowledged,
        show_whats_new: settings.privacy_acknowledged && settings.last_seen_version.as_deref() != Some(state.app_version.as_str()),
    })
}

/// The newest log file's last `max_lines` lines (already secret-redacted).
#[tauri::command]
pub fn recent_logs(state: State<'_, AppState>, max_lines: usize) -> CmdResult<String> {
    let newest = std::fs::read_dir(&state.log_dir)
        .map_err(|e| CommandError::msg(e.to_string()))?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(crate::logging::LOG_FILE_PREFIX))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    let Some(file) = newest else { return Ok(String::new()) };
    let text = std::fs::read_to_string(file.path()).map_err(|e| CommandError::msg(e.to_string()))?;
    let lines: Vec<&str> = text.lines().collect();
    let keep = max_lines.min(MAX_LOG_LINES);
    Ok(lines[lines.len().saturating_sub(keep)..].join("\n"))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Folder {
    Logs,
    Data,
    Failures,
}

#[tauri::command]
pub fn open_folder(app: AppHandle, state: State<'_, AppState>, folder: Folder) -> CmdResult<()> {
    let path = match folder {
        Folder::Logs => state.log_dir.clone(),
        Folder::Data => state.data_dir().clone(),
        Folder::Failures => state.data_dir().join(FAILURES_DIR),
    };
    std::fs::create_dir_all(&path).map_err(|e| CommandError::msg(e.to_string()))?;
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| CommandError::msg(e.to_string()))
}

/// "Check for updates" button: installs right away if one is found.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> CmdResult<super::update_service::UpdateStatus> {
    if app.state::<AppState>().is_running() {
        return Err(CommandError::msg("Wait for the current login run to finish, then check again."));
    }
    super::update_service::check(&app, true).await.map_err(CommandError::msg)
}

#[tauri::command]
pub fn show_main_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
