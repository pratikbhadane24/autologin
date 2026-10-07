// The readiness line: how many accounts are ready for market open.
import type { Account } from "./types";
import type { LiveAccount } from "./runState";

export type Segment = "ready" | "running" | "failed" | "attention" | "waiting" | "unsupported";

export interface Readiness {
  ready: number;
  total: number;
  segments: { id: number; state: Segment; label: string }[];
  headline: string;
}

export function segmentFor(account: Account, live?: LiveAccount): Segment {
  if (live?.phase === "running") return "running";
  if (live?.phase === "failed") return "failed";
  if (live?.phase === "ok") return "ready";
  if (account.coming_soon) return "unsupported";
  if (account.missing.length > 0) return "attention";
  if (account.effective_status === "logged_in") return "ready";
  if (account.effective_status === "failed") return "failed";
  return "waiting";
}

export function readiness(accounts: Account[], live: Record<number, LiveAccount>): Readiness {
  const segments = accounts.map((account) => ({
    id: account.id,
    state: segmentFor(account, live[account.id]),
    label: `${account.broker_name} ${account.client_id}`,
  }));
  const ready = segments.filter((s) => s.state === "ready").length;
  // Accounts for brokers AutoLogin can't log in yet don't count against readiness.
  const total = segments.filter((s) => s.state !== "unsupported").length;
  return { ready, total, segments, headline: headline(ready, total) };
}

function headline(ready: number, total: number): string {
  if (total === 0) return "Add a broker account to get started";
  if (ready === total) return total === 1 ? "Your account is ready for market open" : `All ${total} accounts are ready for market open`;
  return `${ready} of ${total} accounts ready for market open`;
}
