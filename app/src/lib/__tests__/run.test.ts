import { describe, expect, it } from "vitest";
import { idleRun, reduceRun } from "../runState";
import { readiness, segmentFor } from "../readiness";
import type { Account } from "../types";

function account(overrides: Partial<Account> = {}): Account {
  return {
    id: 1,
    tenant_id: "cirrus",
    broker_id: "zerodha",
    client_id: "AB1",
    fields: {},
    secret_keys: ["password", "totp_key"],
    status: "logged_out",
    effective_status: "logged_out",
    last_login: null,
    last_error: null,
    added_on: "2026-10-05T00:00:00Z",
    broker_name: "Zerodha",
    missing: [],
    coming_soon: false,
    ...overrides,
  };
}

describe("reduceRun", () => {
  it("tracks each account through a run", () => {
    let state = reduceRun(idleRun, { type: "run_started", run_id: 1, total: 2 });
    state = reduceRun(state, { type: "account_started", account_id: 1, attempt: 1 });
    expect(state.accounts[1]).toEqual({ phase: "running", attempt: 1, message: null });
    state = reduceRun(state, { type: "account_started", account_id: 1, attempt: 2 });
    state = reduceRun(state, { type: "account_finished", account_id: 1, ok: true, message: "Account saved" });
    expect(state.accounts[1]).toEqual({ phase: "ok", attempt: 2, message: "Account saved" });
    state = reduceRun(state, { type: "account_skipped", account_id: 2, reason: "Needs setup" });
    expect(state.accounts[2].phase).toBe("skipped");
    const summary = { succeeded: 1, failed: 0, skipped: 1, cancelled: false, failed_accounts: [], failed_ids: [] };
    state = reduceRun(state, { type: "run_finished", run_id: 1, summary });
    expect(state.running).toBe(false);
    expect(state.summary).toEqual(summary);
  });

  it("does not mutate the previous state", () => {
    const started = reduceRun(idleRun, { type: "run_started", run_id: 1, total: 1 });
    reduceRun(started, { type: "account_started", account_id: 1, attempt: 1 });
    expect(started.accounts).toEqual({});
  });
});

describe("readiness", () => {
  it("counts ready accounts and writes the headline", () => {
    const accounts = [
      account({ id: 1, effective_status: "logged_in" }),
      account({ id: 2, effective_status: "failed" }),
      account({ id: 3, missing: ["Password"] }),
    ];
    const result = readiness(accounts, {});
    expect(result.headline).toBe("1 of 3 accounts ready for market open");
    expect(result.segments.map((s) => s.state)).toEqual(["ready", "failed", "attention"]);
  });

  it("leaves coming-soon brokers out of the count", () => {
    const accounts = [account({ id: 1, effective_status: "logged_in" }), account({ id: 2, coming_soon: true })];
    const result = readiness(accounts, {});
    expect(result.headline).toBe("Your account is ready for market open");
    expect(result.segments[1].state).toBe("unsupported");
  });

  it("live progress overrides stored status", () => {
    expect(segmentFor(account({ effective_status: "logged_in" }), { phase: "running", attempt: 1, message: null })).toBe(
      "running",
    );
  });

  it("has friendly headlines for empty and all-ready", () => {
    expect(readiness([], {}).headline).toBe("Add a broker account to get started");
    const all = [account({ effective_status: "logged_in" }), account({ id: 2, effective_status: "logged_in" })];
    expect(readiness(all, {}).headline).toBe("All 2 accounts are ready for market open");
  });
});
