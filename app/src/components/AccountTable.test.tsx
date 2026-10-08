// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Account } from "../lib/types";
import { AccountTable, type AccountListProps } from "./AccountTable";

const LONG_ERROR = "Upstox's login page didn't look as expected. A screenshot was saved; see Activity log.";

const upstox: Account = {
  id: 3,
  tenant_id: "cirrus",
  broker_id: "upstox",
  client_id: "7HQ2LP",
  tag: null,
  fields: {},
  secret_keys: ["password"],
  status: "failed",
  effective_status: "failed",
  last_login: null,
  last_error: LONG_ERROR,
  added_on: "2026-10-05T00:00:00Z",
  broker_name: "Upstox",
  missing: [],
  coming_soon: false,
};

function setViewport(isPhone: boolean) {
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: isPhone,
    media: query,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }));
}

function renderList(overrides: Partial<AccountListProps> = {}) {
  const props: AccountListProps = {
    accounts: [upstox],
    live: {},
    selected: new Set(),
    showWorkspace: false,
    tenantName: (id) => id,
    onToggle: vi.fn(),
    onToggleAll: vi.fn(),
    onEdit: vi.fn(),
    ...overrides,
  };
  render(<AccountTable {...props} />);
  return props;
}

afterEach(cleanup);

describe("AccountTable", () => {
  it("shows a table on wide screens", () => {
    setViewport(false);
    renderList();
    expect(screen.getByRole("table")).toBeTruthy();
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("shows cards on phones, with the full failure message", () => {
    setViewport(true);
    renderList();
    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.getByRole("list")).toBeTruthy();
    expect(screen.getByText("Failed")).toBeTruthy();
    expect(screen.getByText(LONG_ERROR)).toBeTruthy();
    expect(screen.getByText("Last login: Never")).toBeTruthy();
  });

  it("shows each account's name under its client ID, on wide screens and phones", () => {
    const tagged = { ...upstox, tag: "Vinit ant" };
    setViewport(false);
    renderList({ accounts: [tagged] });
    expect(screen.getByText("Vinit ant")).toBeTruthy();
    expect(screen.getByRole("checkbox", { name: "Select Upstox 7HQ2LP (Vinit ant)" })).toBeTruthy();
    cleanup();

    setViewport(true);
    renderList({ accounts: [tagged] });
    expect(screen.getByText("Vinit ant")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Edit Upstox 7HQ2LP (Vinit ant)" })).toBeTruthy();
  });

  it("opens the drawer and toggles selection from a card", () => {
    setViewport(true);
    const props = renderList();
    fireEvent.click(screen.getByRole("button", { name: "Edit Upstox 7HQ2LP" }));
    expect(props.onEdit).toHaveBeenCalledWith(upstox);
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Upstox 7HQ2LP" }));
    expect(props.onToggle).toHaveBeenCalledWith(3);
    fireEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
    expect(props.onToggleAll).toHaveBeenCalledWith(true);
  });
});
