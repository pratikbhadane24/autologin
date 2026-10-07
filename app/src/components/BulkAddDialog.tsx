import { useState } from "react";
import { api } from "../lib/api";
import type { Catalog, CommandError, FieldSpec, PasteResult, PastedAccount } from "../lib/types";
import "./BulkAddDialog.css";

interface Props {
  catalog: Catalog;
  paste: PasteResult;
  onClose: () => void;
  onDone: (message: string) => void;
}

interface Row {
  pasted: PastedAccount;
  values: Record<string, string>;
  error: CommandError | null;
}

/** Fields the user still has to type for a pasted account. */
function fieldsToFill(catalog: Catalog, pasted: PastedAccount): FieldSpec[] {
  const broker = catalog.brokers.find((b) => b.id === pasted.broker_id);
  return (broker?.fields ?? []).filter((f) => !(f.key in pasted.fields));
}

/** Add many accounts from one Cirrus paste: one row each, one button. */
export function BulkAddDialog({ catalog, paste, onClose, onDone }: Props) {
  const [rows, setRows] = useState<Row[]>(() =>
    paste.accounts.filter((a) => !a.coming_soon).map((pasted) => ({ pasted, values: {}, error: null })),
  );
  const [saving, setSaving] = useState(false);
  const skippedComingSoon = paste.accounts.filter((a) => a.coming_soon);
  const brokerName = (id: string) => catalog.brokers.find((b) => b.id === id)?.name ?? id;

  function setValue(index: number, key: string, value: string) {
    setRows((current) => current.map((row, i) => (i === index ? { ...row, values: { ...row.values, [key]: value } } : row)));
  }

  async function addAll() {
    setSaving(true);
    try {
      const inputs = rows.map(({ pasted, values }) => ({
        tenant_id: pasted.tenant_id,
        broker_id: pasted.broker_id,
        values: { ...pasted.fields, ...values },
      }));
      const result = await api.addAccounts(inputs);
      const failed = new Map(result.errors.map((e) => [e.index, e.error]));
      const needsSetup = result.added.filter((a) => a.missing.length > 0).length;
      if (failed.size === 0) {
        onDone(`Added ${result.added.length} account${result.added.length === 1 ? "" : "s"}` + (needsSetup ? `, ${needsSetup} need setup` : ""));
        onClose();
        return;
      }
      // Keep only the rows that failed, with their errors, so they can be fixed.
      setRows((current) => current.flatMap((row, i) => (failed.has(i) ? [{ ...row, error: failed.get(i)! }] : [])));
      if (result.added.length) onDone(`Added ${result.added.length}. Fix the rows below and try again.`);
    } catch (e) {
      setRows((current) => current.map((row) => ({ ...row, error: e as CommandError })));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="drawer-backdrop" onClick={onClose}>
      <aside className="drawer bulk" role="dialog" aria-modal="true" aria-labelledby="bulk-title" onClick={(e) => e.stopPropagation()}>
        <header className="drawer-header">
          <h2 id="bulk-title">Add {rows.length} account{rows.length === 1 ? "" : "s"} from Cirrus</h2>
          <button className="button quiet" onClick={onClose}>
            Close
          </button>
        </header>

        <div className="drawer-body">
          {paste.issued_to && (
            <p className="notice">
              Copied from Cirrus account <strong>{paste.issued_to}</strong>. If that isn't you, close this and don't add
              these accounts.
            </p>
          )}
          <p className="muted">
            Enter each account's password, PIN and TOTP secret. Anything you leave blank can be added later; the account shows
            "Needs setup" until then.
          </p>
          {skippedComingSoon.length > 0 && (
            <p className="notice">
              Not added yet (support coming soon): {skippedComingSoon.map((a) => `${brokerName(a.broker_id)} ${a.fields.client_id}`).join(", ")}.
            </p>
          )}
          {paste.problems.length > 0 && (
            <p className="notice">
              Skipped from the paste: {paste.problems.map((p) => `#${p.position} ${p.reason}`).join("; ")}.
            </p>
          )}

          <ol className="bulk-rows">
            {rows.map((row, index) => (
              <li key={`${row.pasted.tenant_id}:${row.pasted.broker_id}:${row.pasted.fields.client_id}`} className="bulk-row">
                <div className="bulk-row-title">
                  <strong>{brokerName(row.pasted.broker_id)}</strong> {row.pasted.fields.client_id}
                  {catalog.tenants.length > 1 && (
                    <span className="muted"> ({catalog.tenants.find((t) => t.id === row.pasted.tenant_id)?.name})</span>
                  )}
                </div>
                <div className="bulk-fields">
                  {fieldsToFill(catalog, row.pasted).map((field) => {
                    const id = `bulk-${index}-${field.key}`;
                    const fieldError = row.error?.fields?.[field.key];
                    return (
                      <div className="field" key={field.key}>
                        <label htmlFor={id}>
                          {field.label}
                          {!field.required && <span className="muted"> (optional)</span>}
                        </label>
                        <input
                          id={id}
                          className="input"
                          type={field.secret ? "password" : "text"}
                          autoComplete="off"
                          spellCheck={false}
                          value={row.values[field.key] ?? ""}
                          aria-invalid={fieldError ? true : undefined}
                          onChange={(e) => setValue(index, field.key, e.target.value)}
                        />
                        {fieldError && <span className="error">{fieldError}</span>}
                      </div>
                    );
                  })}
                </div>
                {row.error && !row.error.fields && <p className="form-error">{row.error.message}</p>}
              </li>
            ))}
          </ol>
        </div>

        <footer className="drawer-footer">
          <span className="spacer" />
          <button className="button" onClick={onClose}>
            Cancel
          </button>
          <button className="button primary" onClick={addAll} disabled={saving || rows.length === 0}>
            Add {rows.length} account{rows.length === 1 ? "" : "s"}
          </button>
        </footer>
      </aside>
    </div>
  );
}
