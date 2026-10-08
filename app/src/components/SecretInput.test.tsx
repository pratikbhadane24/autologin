// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { inSentence, SecretInput } from "./SecretInput";

function Harness({ label = "Password", initial = "" }: { label?: string; initial?: string }) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <label htmlFor="secret">{label}</label>
      <SecretInput id="secret" label={label} value={value} onChange={setValue} />
    </>
  );
}

const field = () => screen.getByLabelText("Password", { selector: "input" }) as HTMLInputElement;

afterEach(cleanup);

describe("SecretInput", () => {
  it("starts hidden, with autofill and spellcheck off", () => {
    render(<Harness />);
    expect(field().type).toBe("password");
    expect(field().getAttribute("autocomplete")).toBe("off");
    expect(field().getAttribute("spellcheck")).toBe("false");
    const toggle = screen.getByRole("button", { name: "Show password" });
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    expect(toggle.getAttribute("type")).toBe("button");
  });

  it("shows and hides what is typed, keeping focus and the caret in the input", () => {
    render(<Harness />);
    fireEvent.change(field(), { target: { value: "s3cret" } });
    field().focus();
    field().setSelectionRange(2, 2);

    fireEvent.click(screen.getByRole("button", { name: "Show password" }));

    expect(field().type).toBe("text");
    expect(field().value).toBe("s3cret");
    expect(document.activeElement).toBe(field());
    expect(field().selectionStart).toBe(2);
    const hide = screen.getByRole("button", { name: "Hide password" });
    expect(hide.getAttribute("aria-pressed")).toBe("true");

    fireEvent.click(hide);
    expect(field().type).toBe("password");
    expect(document.activeElement).toBe(field());
  });

  it("hides the value again once the field is cleared", () => {
    render(<Harness initial="s3cret" />);
    fireEvent.click(screen.getByRole("button", { name: "Show password" }));
    expect(field().type).toBe("text");

    fireEvent.change(field(), { target: { value: "" } });

    expect(field().type).toBe("password");
    fireEvent.change(field(), { target: { value: "n" } });
    expect(field().type).toBe("password");
    expect(screen.getByRole("button", { name: "Show password" })).toBeTruthy();
  });

  it("is hidden again when the dialog reopens", () => {
    const { unmount } = render(<Harness initial="s3cret" />);
    fireEvent.click(screen.getByRole("button", { name: "Show password" }));
    unmount();
    render(<Harness initial="s3cret" />);
    expect(field().type).toBe("password");
  });

  it("names the button after the field", () => {
    expect(inSentence("Password")).toBe("password");
    expect(inSentence("Backup password")).toBe("backup password");
    expect(inSentence("TOTP Secret")).toBe("TOTP Secret");
    expect(inSentence("PIN")).toBe("PIN");
  });
});
