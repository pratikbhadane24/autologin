import { accountName, allSelected, statusOf } from "../lib/accountStatus";
import { relativeTime } from "../lib/hooks";
import type { AccountListProps } from "./AccountTable";
import "./AccountStatus.css";
import "./AccountCards.css";

/** Phone layout of the account list: one card per account. Tapping a card
 *  opens the account drawer; the checkbox selects it for "Log in N selected". */
export function AccountCards({ accounts, live, selected, showWorkspace, tenantName, onToggle, onToggleAll, onEdit }: AccountListProps) {
  return (
    <div className="account-cards">
      <label className="account-cards-all">
        <input
          type="checkbox"
          checked={allSelected(accounts, selected)}
          onChange={(e) => onToggleAll(e.target.checked)}
        />
        Select all
      </label>
      <ul className="account-card-list">
        {accounts.map((account) => {
          const status = statusOf(account, live[account.id]);
          const isSelected = selected.has(account.id);
          return (
            <li key={account.id} className={isSelected ? "account-card selected" : "account-card"}>
              <label className="account-card-check">
                <input
                  type="checkbox"
                  aria-label={`Select ${accountName(account)}`}
                  checked={isSelected}
                  onChange={() => onToggle(account.id)}
                />
              </label>
              <div className="account-card-main">
                <h3 className="account-card-title">
                  {/* Its ::after stretches over the whole card, so the card is the tap target. */}
                  <button className="account-card-open" aria-label={`Edit ${accountName(account)}`} onClick={() => onEdit(account)}>
                    {account.broker_name} <span className="account-card-client">{account.client_id}</span>
                  </button>
                </h3>
                {showWorkspace && <p className="account-card-meta">{tenantName(account.tenant_id)}</p>}
                <p className="account-card-status">
                  <span className={`status pill status-${status.tone}`}>{status.text}</span>
                </p>
                {status.detail && <p className="status-detail">{status.detail}</p>}
                <p className="account-card-meta">Last login: {relativeTime(account.last_login)}</p>
              </div>
              <span className="account-card-chevron" aria-hidden="true">
                ›
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
