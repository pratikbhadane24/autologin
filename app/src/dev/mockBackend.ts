// Dev-only fake backend so the UI can be viewed in a normal browser:
// http://localhost:1420/?mock  (never bundled into release builds' code path).
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { Account, AppSettings, Catalog, RunEvent } from "../lib/types";

const now = Date.now();
const iso = (minutesAgo: number) => new Date(now - minutesAgo * 60000).toISOString();

const field = (key: string, label: string, extra: Partial<Catalog["brokers"][0]["fields"][0]> = {}) => ({
  key, label, required: true, secret: false, totp: false, from_cirrus: false, pattern: null, placeholder: null, help: null, ...extra,
});

const catalog: Catalog = {
  default_tenant: "cirrus",
  manifest_version: 1,
  tenants: [{ id: "cirrus", name: "Cirrus" }, { id: "pocketful", name: "Pocketful" }],
  brokers: [
    { id: "pocketful", name: "Pocketful", kind: "browser", coming_soon: false, help: null,
      fields: [field("client_id", "Client ID", { from_cirrus: true }), field("password", "Password", { secret: true }), field("mpin", "PIN", { secret: true, required: false })] },
    { id: "zerodha", name: "Zerodha", kind: "browser", coming_soon: false, help: null,
      fields: [field("client_id", "User ID", { from_cirrus: true }), field("api_key", "Kite API Key", { from_cirrus: true, help: "From your own Kite Connect app. Set its redirect URL to https://app.cirrus.trade/add-broker-account/zerodha." }), field("password", "Password", { secret: true }), field("totp_key", "TOTP Secret", { secret: true, totp: true })] },
    { id: "fyers", name: "Fyers", kind: "http", coming_soon: true, help: null, fields: [field("client_id", "Fyers ID")] },
  ],
};

function account(id: number, broker: string, name: string, client: string, extra: Partial<Account> = {}): Account {
  return {
    id, tenant_id: "cirrus", broker_id: broker, client_id: client, tag: null, fields: {}, secret_keys: ["password"],
    status: "logged_out", effective_status: "logged_out", last_login: null, last_error: null, added_on: iso(9000),
    broker_name: name, missing: [], coming_soon: false, ...extra,
  };
}

let accounts: Account[] = [
  account(1, "zerodha", "Zerodha", "AB1234", { tag: "Pratik D", status: "logged_in", effective_status: "logged_in", last_login: iso(12) }),
  account(2, "zerodha", "Zerodha", "XY9876", { status: "logged_in", effective_status: "logged_in", last_login: iso(13) }),
  account(3, "upstox", "Upstox", "7HQ2LP", { tag: "Vinit ant", status: "failed", effective_status: "failed", last_login: iso(1500), last_error: "Upstox's login page didn't look as expected. A screenshot was saved; see Activity log. If this keeps happening, Upstox may have changed its page; a broker update will fix it." }),
  account(4, "pocketful", "Pocketful", "PK00321", { tag: "Family HUF", missing: ["Password"] }),
  account(5, "motilal", "Motilal Oswal", "EMUM1022", { status: "logged_in", effective_status: "logged_out", last_login: iso(1600) }),
  account(6, "fyers", "Fyers", "XA00451", { coming_soon: true }),
];

let settings: AppSettings = {
  manual_headless: false,
  schedule: { enabled: true, time: "08:45:00", days: ["Mon", "Tue", "Wed", "Thu", "Fri"], tz: "Asia/Kolkata", retry_failed_after_minutes: 5, headless: true, phone_automatic: false },
  retries: 1, concurrency: 4, start_with_computer: true, auto_update: true, privacy_acknowledged: !location.search.includes("first"), last_seen_version: "2.0.0-beta.1",
};

const view = () => ({ settings, next_scheduled_run: new Date(now + 15 * 3600000).toISOString(), recommendation: "Recommended: 8:45 AM. Brokers have finished their overnight session reset by then, and your accounts are ready before the 9:00 AM pre-open." });

function simulateRun(ids: number[]) {
  const send = (e: RunEvent, at: number) => setTimeout(() => emit("run-event", e), at);
  send({ type: "run_started", run_id: 1, total: ids.length }, 50);
  ids.forEach((id, i) => {
    const target = accounts.find((a) => a.id === id)!;
    if (target.missing.length || target.coming_soon) {
      send({ type: "account_skipped", account_id: id, reason: target.coming_soon ? "Fyers support is coming soon." : "Needs setup: add Password." }, 200);
      return;
    }
    send({ type: "account_started", account_id: id, attempt: 1 }, 300 + i * 400);
    send({ type: "account_finished", account_id: id, ok: id !== 3, message: id !== 3 ? "Account saved" : "broker showed: Invalid TOTP" }, 2200 + i * 700);
  });
  send({ type: "run_finished", run_id: 1, summary: { succeeded: 3, failed: 1, skipped: 2, cancelled: false, failed_accounts: ["Upstox 7HQ2LP (Vinit ant)"], failed_ids: [3] } }, 2400 + ids.length * 700);
}

export function installMockBackend() {
  mockIPC(
    (cmd, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "app_info": return { version: "2.0.0", manifest_version: 1, privacy_acknowledged: settings.privacy_acknowledged, show_whats_new: location.search.includes("whatsnew") };
        case "get_catalog": return catalog;
        case "list_accounts": return accounts;
        case "get_settings_view": return view();
        case "save_settings": settings = args.settings as AppSettings; return view();
        case "take_migration_outcome": return location.search.includes("migrated") ? { report: { added: 4, updated: 0, needs_setup: 1, problems: [] }, waiting_brokers: ["kotakneo"], background_login: false } : null;
        case "is_running": return false;
        case "run_accounts": {
          const sel = args.selection as { kind: string; ids?: number[] };
          const ids = sel.kind === "ids" ? sel.ids! : sel.kind === "failed" ? [3] : accounts.map((a) => a.id);
          simulateRun(ids);
          return ids.length;
        }
        case "plugin:clipboard-manager|read_text":
          return "mock clipboard";
        case "parse_paste":
          return {
            accounts: [
              { tenant_id: "cirrus", broker_id: "pocketful", fields: { client_id: "PK00999" }, tag: "Vinit ant", ignored: [], coming_soon: false, already_added: false, kept_values: [] },
              { tenant_id: "cirrus", broker_id: "zerodha", fields: { client_id: "AB1234", api_key: "kite_key" }, tag: "Pratik Dad", ignored: ["api_secret"], coming_soon: false, already_added: true, kept_values: ["Kite API Key", "Account Tag"] },
              { tenant_id: "pocketful", broker_id: "pocketful", fields: { client_id: "PK00999" }, tag: null, ignored: [], coming_soon: false, already_added: false, kept_values: [] },
              { tenant_id: "cirrus", broker_id: "fyers", fields: { client_id: "XA00451" }, tag: "Pratik D", ignored: [], coming_soon: true, already_added: false, kept_values: [] },
            ],
            problems: [],
            issued_to: "demo_user",
          };
        case "add_accounts":
          return { added: [], updated: [], errors: [] };
        case "recent_logs": return "2026-10-05T03:15:00Z  INFO starting run trigger=Scheduled count=6 headless=true\n2026-10-05T03:15:04Z  INFO login finished account=1 name=Zerodha AB1234 (Pratik D) ok=true";
        case "acknowledge_privacy": settings = { ...settings, privacy_acknowledged: true }; return null;
        default: return null;
      }
    },
    { shouldMockEvents: true },
  );
}
