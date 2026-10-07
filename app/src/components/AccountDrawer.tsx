import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { ask } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useState } from "react";
import { api } from "../lib/api";
import { DEVICE } from "../lib/platform";
import type { Account, Catalog, CommandError, FieldSpec, PastedAccount } from "../lib/types";
import "./AccountDrawer.css";

interface Props {
  catalog: Catalog;
  /** Account being edited, or null to add a new one. */
  account: Account | null;
  onClose: () => void;
  onSaved: (message: string) => void;
}

const TOTP_HINT = "The text key shown under the QR code when you turned on TOTP (letters A–Z and digits 2–7).";

export function AccountDrawer({ catalog, account, onClose, onSaved }: Props) {
  const editing = account !== null;
  const firstBroker = catalog.brokers.find((b) => !b.coming_soon)?.id ?? "";
  const [brokerId, setBrokerId] = useState(account?.broker_id ?? firstBroker);
  const [tenantId, setTenantId] = useState(account?.tenant_id ?? catalog.default_tenant);
  const [values, setValues] = useState<Record<string, string>>(() => initialValues(account));
  const [fromCirrus, setFromCirrus] = useState<Set<string>>(new Set());
  const [queue, setQueue] = useState<PastedAccount[]>([]);
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const broker = useMemo(() => catalog.brokers.find((b) => b.id === brokerId), [catalog, brokerId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  function applyPasted(pasted: PastedAccount) {
    setBrokerId(pasted.broker_id);
    setTenantId(pasted.tenant_id);
    setValues(pasted.fields);
    setFromCirrus(new Set(Object.keys(pasted.fields)));
    setError(null);
  }

  async function pasteFromCirrus() {
    setNotice(null);
    try {
      const result = await api.parsePaste(await readText());
      const [first, ...rest] = result.accounts;
      if (!first) {
        setError({ message: result.problems[0]?.reason ?? "No accounts found in the copied text.", fields: null });
        return;
      }
      applyPasted(first);
      setQueue(rest);
      const ignored = first.ignored.length ? ` Ignored: ${first.ignored.join(", ")}.` : "";
      const more = rest.length ? ` ${rest.length} more account${rest.length > 1 ? "s" : ""} will follow after you save.` : "";
      setNotice(`Filled in from Cirrus. Add your password, PIN and TOTP secret below.${ignored}${more}`);
    } catch (e) {
      setError(e as CommandError);
    }
  }

  async function save() {
    setSaving(true);
    setError(null);
    const input = { tenant_id: tenantId, broker_id: brokerId, values };
    try {
      const saved = editing ? await api.updateAccount(account.id, input) : await api.createAccount(input);
      const label = `${saved.broker_name} ${saved.client_id}`;
      const [next, ...rest] = queue;
      if (next) {
        applyPasted(next);
        setQueue(rest);
        setNotice(`Saved ${label}. Next from your Cirrus paste:`);
        onSaved(`Added ${label}`);
      } else {
        onSaved(editing ? `Saved ${label}` : `Added ${label}`);
        onClose();
      }
    } catch (e) {
      setError(e as CommandError);
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!account) return;
    const confirmed = await ask(`Delete ${account.broker_name} ${account.client_id}? Its saved password, PIN and TOTP secret are removed from this ${DEVICE}.`, {
      title: "Delete account",
      kind: "warning",
      okLabel: "Delete account",
    });
    if (!confirmed) return;
    try {
      await api.deleteAccounts([account.id]);
      onSaved(`Deleted ${account.broker_name} ${account.client_id}`);
      onClose();
    } catch (e) {
      setError(e as CommandError);
    }
  }

  return (
    <div className="drawer-backdrop" onClick={onClose}>
      <aside className="drawer" role="dialog" aria-modal="true" aria-labelledby="drawer-title" onClick={(e) => e.stopPropagation()}>
        <header className="drawer-header">
          <h2 id="drawer-title">{editing ? `Edit ${account.broker_name} ${account.client_id}` : "Add account"}</h2>
          <button className="button quiet" onClick={onClose} aria-label="Close">
            Close
          </button>
        </header>

        <div className="drawer-body">
          {!editing && (
            <div className="paste-box">
              <p>Copy the account from your Cirrus dashboard, then:</p>
              <button className="button" onClick={pasteFromCirrus}>
                Paste from Cirrus
              </button>
            </div>
          )}
          {notice && <p className="notice">{notice}</p>}
          {error && !error.fields && <p className="form-error" role="alert">{error.message}</p>}

          {!editing && (
            <div className="field">
              <label htmlFor="broker">Broker</label>
              <select id="broker" className="input" value={brokerId} onChange={(e) => {
                setBrokerId(e.target.value);
                setValues({});
                setFromCirrus(new Set());
              }}>
                {catalog.brokers.map((b) => (
                  <option key={b.id} value={b.id} disabled={b.coming_soon}>
                    {b.coming_soon ? `${b.name} (coming soon)` : b.name}
                  </option>
                ))}
              </select>
            </div>
          )}

          {catalog.tenants.length > 1 && (
            <div className="field">
              <label htmlFor="tenant">Cirrus workspace</label>
              <select id="tenant" className="input" value={tenantId} onChange={(e) => setTenantId(e.target.value)}>
                {catalog.tenants.map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.name}
                  </option>
                ))}
              </select>
            </div>
          )}

          {broker?.fields.map((field) => (
            <FieldInput
              key={field.key}
              field={field}
              value={values[field.key] ?? ""}
              saved={editing && field.secret && account.secret_keys.includes(field.key)}
              filledFromCirrus={fromCirrus.has(field.key)}
              error={error?.fields?.[field.key] ?? null}
              onChange={(value) => setValues((current) => ({ ...current, [field.key]: value }))}
            />
          ))}
        </div>

        <footer className="drawer-footer">
          {editing && (
            <button className="button danger" onClick={remove}>
              Delete account
            </button>
          )}
          <span className="spacer" />
          <button className="button" onClick={onClose}>
            Cancel
          </button>
          <button className="button primary" onClick={save} disabled={saving || !broker}>
            {editing ? "Save changes" : "Add account"}
          </button>
        </footer>
      </aside>
    </div>
  );
}

function initialValues(account: Account | null): Record<string, string> {
  if (!account) return {};
  return { ...account.fields, client_id: account.client_id };
}

interface FieldProps {
  field: FieldSpec;
  value: string;
  saved: boolean;
  filledFromCirrus: boolean;
  error: string | null;
  onChange: (value: string) => void;
}

function FieldInput({ field, value, saved, filledFromCirrus, error, onChange }: FieldProps) {
  const id = `field-${field.key}`;
  const hint = field.totp ? TOTP_HINT : field.help;
  const placeholder = saved ? "Saved. Leave blank to keep it." : field.placeholder ?? undefined;
  return (
    <div className="field">
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
        value={value}
        placeholder={placeholder}
        aria-invalid={error ? true : undefined}
        aria-describedby={error ? `${id}-error` : hint ? `${id}-hint` : undefined}
        onChange={(e) => onChange(e.target.value)}
      />
      {filledFromCirrus && <span className="hint">Filled in from Cirrus.</span>}
      {hint && <span className="hint" id={`${id}-hint`}>{hint}</span>}
      {error && <span className="error" id={`${id}-error`}>{error}</span>}
    </div>
  );
}
