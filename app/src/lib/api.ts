// Typed wrappers around Tauri commands (src-tauri/src/app/commands.rs).
import { invoke } from "@tauri-apps/api/core";
import type {
  Account,
  AccountInput,
  AppInfo,
  AppSettings,
  BulkResult,
  Catalog,
  CommandError,
  ExportFormat,
  FileKind,
  Folder,
  ImportReport,
  MigrationOutcome,
  PasteResult,
  Selection,
  SettingsView,
  UpdateStatus,
} from "./types";

/** Normalizes anything a command can throw into a CommandError. */
export function toCommandError(error: unknown): CommandError {
  if (error && typeof error === "object" && "message" in error) {
    const { message, fields } = error as Partial<CommandError>;
    return { message: String(message), fields: fields ?? null };
  }
  return { message: String(error), fields: null };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw toCommandError(error);
  }
}

export const api = {
  catalog: () => call<Catalog>("get_catalog"),
  appInfo: () => call<AppInfo>("app_info"),
  listAccounts: () => call<Account[]>("list_accounts"),
  createAccount: (input: AccountInput) => call<Account>("create_account", { input }),
  updateAccount: (id: number, input: AccountInput) => call<Account>("update_account", { id, input }),
  addAccounts: (inputs: AccountInput[]) => call<BulkResult>("add_accounts", { inputs }),
  deleteAccounts: (ids: number[]) => call<number>("delete_accounts", { ids }),
  parsePaste: (text: string) => call<PasteResult>("parse_paste", { text }),
  runAccounts: (selection: Selection, headless?: boolean) =>
    call<number>("run_accounts", { selection, headless: headless ?? null }),
  stopRun: () => call<boolean>("stop_run"),
  isRunning: () => call<boolean>("is_running"),
  settingsView: () => call<SettingsView>("get_settings_view"),
  saveSettings: (settings: AppSettings) => call<SettingsView>("save_settings", { settings }),
  acknowledgePrivacy: () => call<void>("acknowledge_privacy"),
  markWhatsNewSeen: () => call<void>("mark_whats_new_seen"),
  exportAccounts: (path: string, format: ExportFormat, password?: string) =>
    call<number>("export_accounts", { path, format, password: password ?? null }),
  inspectImport: (path: string) => call<FileKind>("inspect_import", { path }),
  importAccounts: (path: string, password?: string) =>
    call<ImportReport>("import_accounts", { path, password: password ?? null }),
  takePendingImport: () => call<string | null>("take_pending_import"),
  takeMigrationOutcome: () => call<MigrationOutcome | null>("take_migration_outcome"),
  recentLogs: (maxLines: number) => call<string>("recent_logs", { maxLines }),
  openFolder: (folder: Folder) => call<void>("open_folder", { folder }),
  checkForUpdate: () => call<UpdateStatus>("check_for_update"),
};
