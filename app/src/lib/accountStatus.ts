// What each account row shows, shared by the desktop table and the phone cards.
import type { LiveAccount } from "./runState";
import type { Account } from "./types";

export type StatusTone = "ok" | "fail" | "warn" | "busy" | "idle";

export interface StatusView {
  tone: StatusTone;
  text: string;
  detail: string | null;
}

export function statusOf(account: Account, live?: LiveAccount): StatusView {
  if (live?.phase === "running") {
    return { tone: "busy", text: live.attempt > 1 ? `Logging in (try ${live.attempt})` : "Logging in", detail: null };
  }
  if (live?.phase === "skipped") return { tone: "warn", text: "Skipped", detail: live.message };
  if (live?.phase === "failed") return { tone: "fail", text: "Failed", detail: live.message };
  if (live?.phase === "ok") return { tone: "ok", text: "Logged in", detail: null };
  if (account.coming_soon) return { tone: "idle", text: "Coming soon", detail: `${account.broker_name} support is on its way.` };
  if (account.missing.length > 0) return { tone: "warn", text: "Needs setup", detail: `Add ${account.missing.join(", ")}.` };
  if (account.effective_status === "logged_in") return { tone: "ok", text: "Logged in", detail: null };
  if (account.effective_status === "failed") return { tone: "fail", text: "Failed", detail: account.last_error };
  if (account.status === "logged_in") return { tone: "idle", text: "Session expired", detail: null };
  return { tone: "idle", text: "Not logged in", detail: null };
}

/** "Zerodha AB1234 (Pratik D)": names an account in labels like "Select …" and "Edit …". */
export function accountName(account: Pick<Account, "broker_name" | "client_id" | "tag">): string {
  const name = `${account.broker_name} ${account.client_id}`;
  return account.tag ? `${name} (${account.tag})` : name;
}

/** True when there is at least one account and every one of them is selected. */
export function allSelected(accounts: Account[], selected: ReadonlySet<number>): boolean {
  return accounts.length > 0 && accounts.every((a) => selected.has(a.id));
}
