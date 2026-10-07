// Live run progress, derived from "run-event" messages.
import type { RunEvent, RunSummary } from "./types";

export type Phase = "running" | "ok" | "failed" | "skipped";

export interface LiveAccount {
  phase: Phase;
  attempt: number;
  message: string | null;
}

export interface RunState {
  running: boolean;
  total: number;
  accounts: Record<number, LiveAccount>;
  summary: RunSummary | null;
}

export const idleRun: RunState = { running: false, total: 0, accounts: {}, summary: null };

export function reduceRun(state: RunState, event: RunEvent): RunState {
  switch (event.type) {
    case "run_started":
      return { running: true, total: event.total, accounts: {}, summary: null };
    case "account_started":
      return withAccount(state, event.account_id, { phase: "running", attempt: event.attempt, message: null });
    case "account_finished":
      return withAccount(state, event.account_id, {
        phase: event.ok ? "ok" : "failed",
        attempt: state.accounts[event.account_id]?.attempt ?? 1,
        message: event.message,
      });
    case "account_skipped":
      return withAccount(state, event.account_id, { phase: "skipped", attempt: 0, message: event.reason });
    case "run_finished":
      return { ...state, running: false, summary: event.summary };
  }
}

function withAccount(state: RunState, id: number, live: LiveAccount): RunState {
  return { ...state, accounts: { ...state.accounts, [id]: live } };
}
