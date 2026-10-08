// Mirrors the Rust types sent over IPC (src-tauri/src/app/views.rs etc.).
// No type here ever carries a secret value.

export type LoginStatus = "logged_out" | "logged_in" | "failed";
export type BrokerKind = "browser" | "http" | "redirect";

export interface FieldSpec {
  key: string;
  label: string;
  required: boolean;
  secret: boolean;
  totp: boolean;
  from_cirrus: boolean;
  pattern: string | null;
  placeholder: string | null;
  help: string | null;
}

export interface BrokerInfo {
  id: string;
  name: string;
  kind: BrokerKind;
  coming_soon: boolean;
  help: string | null;
  fields: FieldSpec[];
}

export interface TenantInfo {
  id: string;
  name: string;
}

export interface Catalog {
  brokers: BrokerInfo[];
  tenants: TenantInfo[];
  default_tenant: string;
  manifest_version: number;
}

export interface Account {
  id: number;
  tenant_id: string;
  broker_id: string;
  client_id: string;
  fields: Record<string, string>;
  secret_keys: string[];
  status: LoginStatus;
  effective_status: LoginStatus;
  last_login: string | null;
  last_error: string | null;
  added_on: string;
  broker_name: string;
  missing: string[];
  coming_soon: boolean;
}

export interface AccountInput {
  tenant_id: string;
  broker_id: string;
  values: Record<string, string>;
}

export type Weekday = "Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun";

export interface Schedule {
  enabled: boolean;
  time: string; // "HH:MM:SS"
  days: Weekday[];
  tz: string;
  retry_failed_after_minutes: number | null;
  headless: boolean;
  /** Phones: open and log in with no tap (needs extra permissions). */
  phone_automatic: boolean;
}

/** Phones: what automatic daily login still needs. */
export interface PhoneStatus {
  notifications: boolean;
  exact_alarms: boolean;
  overlay: boolean;
  battery_unrestricted: boolean;
}

export type PhoneSetting = "notifications" | "exactAlarms" | "overlay" | "battery";

export interface AppSettings {
  manual_headless: boolean;
  schedule: Schedule;
  retries: number;
  concurrency: number;
  start_with_computer: boolean;
  auto_update: boolean;
  privacy_acknowledged: boolean;
  last_seen_version: string | null;
}

export interface SettingsView {
  settings: AppSettings;
  next_scheduled_run: string | null;
  recommendation: string;
}

export interface AppInfo {
  version: string;
  manifest_version: number;
  privacy_acknowledged: boolean;
  show_whats_new: boolean;
}

export type Selection =
  | { kind: "all" }
  | { kind: "failed" }
  | { kind: "not_logged_in" }
  | { kind: "ids"; ids: number[] };

export interface RunSummary {
  succeeded: number;
  failed: number;
  skipped: number;
  cancelled: boolean;
  failed_accounts: string[];
  failed_ids: number[];
}

export type RunEvent =
  | { type: "run_started"; run_id: number; total: number }
  | { type: "account_started"; account_id: number; attempt: number }
  | { type: "account_finished"; account_id: number; ok: boolean; message: string }
  | { type: "account_skipped"; account_id: number; reason: string }
  | { type: "run_finished"; run_id: number; summary: RunSummary };

export interface PastedAccount {
  tenant_id: string;
  broker_id: string;
  fields: Record<string, string>;
  ignored: string[];
  coming_soon: boolean;
  /** Already in AutoLogin: adding refreshes Cirrus's values, keeps secrets. */
  already_added: boolean;
}

export interface PasteResult {
  accounts: PastedAccount[];
  problems: { position: number; reason: string }[];
  /** Cirrus username the copy was made for (signed copies). */
  issued_to: string | null;
}

export interface ImportReport {
  added: number;
  updated: number;
  needs_setup: number;
  problems: { row: number; reason: string }[];
}

export type FileKind = "encrypted" | "plain" | "csv";
export type ExportFormat = FileKind;
export type Folder = "logs" | "data" | "failures";

export interface MigrationOutcome {
  report: ImportReport;
  waiting_brokers: string[];
  background_login: boolean | null;
}

export interface CommandError {
  message: string;
  fields: Record<string, string> | null;
}

export type UpdateStatus =
  | { state: "up_to_date" }
  | { state: "available"; version: string; notes: string | null }
  | { state: "downloading"; percent: number | null }
  | { state: "failed"; message: string };

export interface BulkResult {
  added: Account[];
  updated: Account[];
  errors: { index: number; error: CommandError }[];
}
