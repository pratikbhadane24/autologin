import { relativeTime } from "../lib/hooks";
import type { LiveAccount } from "../lib/runState";
import type { Account } from "../lib/types";
import "./AccountTable.css";

interface Props {
  accounts: Account[];
  live: Record<number, LiveAccount>;
  selected: Set<number>;
  showWorkspace: boolean;
  tenantName: (id: string) => string;
  onToggle: (id: number) => void;
  onToggleAll: (select: boolean) => void;
  onEdit: (account: Account) => void;
}

interface StatusView {
  tone: "ok" | "fail" | "warn" | "busy" | "idle";
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

export function AccountTable({ accounts, live, selected, showWorkspace, tenantName, onToggle, onToggleAll, onEdit }: Props) {
  const allSelected = accounts.length > 0 && accounts.every((a) => selected.has(a.id));
  return (
    <table className="accounts">
      <thead>
        <tr>
          <th className="col-check">
            <input
              type="checkbox"
              aria-label="Select all accounts"
              checked={allSelected}
              onChange={(e) => onToggleAll(e.target.checked)}
            />
          </th>
          <th>Broker</th>
          <th>Client ID</th>
          {showWorkspace && <th>Workspace</th>}
          <th>Status</th>
          <th>Last login</th>
          <th className="col-actions">
            <span className="visually-hidden">Actions</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {accounts.map((account) => {
          const status = statusOf(account, live[account.id]);
          return (
            <tr key={account.id} className={selected.has(account.id) ? "selected" : undefined}>
              <td className="col-check">
                <input
                  type="checkbox"
                  aria-label={`Select ${account.broker_name} ${account.client_id}`}
                  checked={selected.has(account.id)}
                  onChange={() => onToggle(account.id)}
                />
              </td>
              <td className="broker">{account.broker_name}</td>
              <td>{account.client_id}</td>
              {showWorkspace && <td>{tenantName(account.tenant_id)}</td>}
              <td>
                <span className={`status status-${status.tone}`}>{status.text}</span>
                {status.detail && <span className="status-detail">{status.detail}</span>}
              </td>
              <td className="muted">{relativeTime(account.last_login)}</td>
              <td className="col-actions">
                <button className="button quiet" onClick={() => onEdit(account)}>
                  Edit
                </button>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
