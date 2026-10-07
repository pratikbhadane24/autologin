import { describe, expect, it } from "vitest";
import { accountName, allSelected, statusOf } from "../accountStatus";
import type { Account } from "../types";

function account(overrides: Partial<Account> = {}): Account {
  return {
    id: 1,
    tenant_id: "cirrus",
    broker_id: "zerodha",
    client_id: "AB1",
    fields: {},
    secret_keys: ["password"],
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

describe("statusOf", () => {
  it("prefers the live run phase over the stored status", () => {
    const stored = account({ effective_status: "failed", last_error: "old" });
    expect(statusOf(stored, { phase: "running", attempt: 2, message: null })).toEqual({
      tone: "busy",
      text: "Logging in (try 2)",
      detail: null,
    });
    expect(statusOf(stored, { phase: "ok", attempt: 1, message: null }).tone).toBe("ok");
  });

  it("shows the stored failure message as the detail", () => {
    const failed = account({ effective_status: "failed", last_error: "Broker page changed" });
    expect(statusOf(failed)).toEqual({ tone: "fail", text: "Failed", detail: "Broker page changed" });
  });

  it("asks for missing fields before anything else is tried", () => {
    expect(statusOf(account({ missing: ["Password", "PIN"] }))).toEqual({
      tone: "warn",
      text: "Needs setup",
      detail: "Add Password, PIN.",
    });
  });

  it("tells an expired session apart from never logged in", () => {
    expect(statusOf(account({ status: "logged_in", effective_status: "logged_out" })).text).toBe("Session expired");
    expect(statusOf(account()).text).toBe("Not logged in");
  });
});

describe("accountName", () => {
  it("joins broker and client ID", () => {
    expect(accountName(account({ broker_name: "Upstox", client_id: "7HQ2LP" }))).toBe("Upstox 7HQ2LP");
  });
});

describe("allSelected", () => {
  it("is false for an empty list", () => {
    expect(allSelected([], new Set())).toBe(false);
  });

  it("is true only when every account is selected", () => {
    const list = [account({ id: 1 }), account({ id: 2 })];
    expect(allSelected(list, new Set([1]))).toBe(false);
    expect(allSelected(list, new Set([1, 2, 3]))).toBe(true);
  });
});
