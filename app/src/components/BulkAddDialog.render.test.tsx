// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../lib/api";
import type { Catalog, FieldSpec, PasteResult, PastedAccount } from "../lib/types";
import { BulkAddDialog, pastedName } from "./BulkAddDialog";

vi.mock("../lib/api", () => ({ api: { addAccounts: vi.fn() } }));

const field = (key: string, label: string, extra: Partial<FieldSpec> = {}): FieldSpec => ({
  key,
  label,
  required: true,
  secret: false,
  totp: false,
  from_cirrus: false,
  pattern: null,
  placeholder: null,
  help: null,
  ...extra,
});

const catalog: Catalog = {
  default_tenant: "cirrus",
  manifest_version: 1,
  tenants: [{ id: "cirrus", name: "Cirrus" }],
  brokers: [
    {
      id: "zerodha",
      name: "Zerodha",
      kind: "browser",
      coming_soon: false,
      help: null,
      fields: [field("client_id", "User ID", { from_cirrus: true }), field("password", "Password", { secret: true })],
    },
  ],
};

const pasted = (client: string, extra: Partial<PastedAccount> = {}): PastedAccount => ({
  tenant_id: "cirrus",
  broker_id: "zerodha",
  fields: { client_id: client },
  tag: null,
  ignored: [],
  coming_soon: false,
  already_added: false,
  kept_values: [],
  ...extra,
});

const paste: PasteResult = {
  accounts: [
    pasted("ZGN479", { tag: "Pratik D" }),
    pasted("AB1234", { tag: "Cirrus name", already_added: true, kept_values: ["Account Tag"] }),
  ],
  problems: [],
  issued_to: null,
};

afterEach(cleanup);

describe("BulkAddDialog", () => {
  it("prefills each new account's name from Cirrus and sends the edited name", async () => {
    vi.mocked(api.addAccounts).mockResolvedValue({ added: [], updated: [], errors: [] });
    render(<BulkAddDialog catalog={catalog} paste={paste} onClose={vi.fn()} onDone={vi.fn()} />);

    const name = screen.getByLabelText(/^Name/) as HTMLInputElement;
    expect(name.value).toBe("Pratik D");
    expect(screen.getByRole("button", { name: "Show password" })).toBeTruthy();
    // An account already here keeps the name it has; the dialog says Cirrus differs.
    expect(screen.getByText(/Cirrus has a different Account Tag/)).toBeTruthy();

    fireEvent.change(name, { target: { value: "Pratik (Dad)" } });
    fireEvent.click(screen.getByRole("button", { name: "Add 1, update 1" }));

    await waitFor(() => expect(api.addAccounts).toHaveBeenCalled());
    const [inputs] = vi.mocked(api.addAccounts).mock.calls[0];
    expect(inputs.map((i) => i.tag)).toEqual(["Pratik (Dad)", "Cirrus name"]);
  });

  it("names pasted accounts with their tag", () => {
    expect(pastedName("Fyers", pasted("XA1", { tag: "Vinit ant" }))).toBe("Fyers XA1 (Vinit ant)");
    expect(pastedName("Fyers", pasted("XA1"))).toBe("Fyers XA1");
  });
});
