import { accountName, allSelected, statusOf } from "../lib/accountStatus";
import { relativeTime } from "../lib/hooks";
import type { LiveAccount } from "../lib/runState";
import type { Account } from "../lib/types";
import { PHONE_QUERY, useMediaQuery } from "../lib/useMediaQuery";
import { AccountCards } from "./AccountCards";
import "./AccountStatus.css";
import "./AccountTable.css";

export interface AccountListProps {
  accounts: Account[];
  live: Record<number, LiveAccount>;
  selected: Set<number>;
  showWorkspace: boolean;
  tenantName: (id: string) => string;
  onToggle: (id: number) => void;
  onToggleAll: (select: boolean) => void;
  onEdit: (account: Account) => void;
}

/** The account list: a table on wide screens, cards on phones. */
export function AccountTable(props: AccountListProps) {
  const isPhone = useMediaQuery(PHONE_QUERY);
  return isPhone ? <AccountCards {...props} /> : <AccountRows {...props} />;
}

function AccountRows({ accounts, live, selected, showWorkspace, tenantName, onToggle, onToggleAll, onEdit }: AccountListProps) {
  return (
    <table className="accounts">
      <thead>
        <tr>
          <th className="col-check">
            <input
              type="checkbox"
              aria-label="Select all accounts"
              checked={allSelected(accounts, selected)}
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
                  aria-label={`Select ${accountName(account)}`}
                  checked={selected.has(account.id)}
                  onChange={() => onToggle(account.id)}
                />
              </td>
              <td className="broker">{account.broker_name}</td>
              <td>
                {account.client_id}
                {account.tag && <span className="account-tag">{account.tag}</span>}
              </td>
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
