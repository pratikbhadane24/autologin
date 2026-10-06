import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { useMemo, useState } from "react";
import { BulkAddDialog } from "../components/BulkAddDialog";
import { api } from "../lib/api";
import { AccountDrawer } from "../components/AccountDrawer";
import { AccountTable } from "../components/AccountTable";
import { ReadinessBoard } from "../components/ReadinessBoard";
import { RunControls } from "../components/RunControls";
import { readiness } from "../lib/readiness";
import type { RunState } from "../lib/runState";
import type { Account, Catalog, CommandError, PasteResult, Selection } from "../lib/types";
import "./AccountsView.css";

interface Props {
  catalog: Catalog;
  accounts: Account[];
  run: RunState;
  nextRun: string | null;
  showBrowser: boolean;
  onShowBrowserChange: (show: boolean) => void;
  onRun: (selection: Selection, showBrowser: boolean) => void;
  onStop: () => void;
  onChanged: (message: string) => void;
  /** Accounts sent from Cirrus via an autologin:// link. */
  incomingImport: PasteResult | null;
  onIncomingImportDone: () => void;
}

type Filter = "all" | "attention" | "ready";

export function AccountsView(props: Props) {
  const { catalog, accounts, run, nextRun, showBrowser, onShowBrowserChange, onRun, onStop, onChanged } = props;
  const { incomingImport, onIncomingImportDone } = props;
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [filter, setFilter] = useState<Filter>("all");
  const [drawer, setDrawer] = useState<{ account: Account | null } | null>(null);
  const [bulk, setBulk] = useState<PasteResult | null>(null);

  async function pasteFromCirrus() {
    try {
      const result = await api.parsePaste(await readText());
      if (result.accounts.length === 0) {
        onChanged(result.problems[0]?.reason ?? "No accounts found in the copied text.");
        return;
      }
      setBulk(result);
    } catch (e) {
      onChanged((e as CommandError).message);
    }
  }

  const board = useMemo(() => readiness(accounts, run.accounts), [accounts, run.accounts]);
  const stateById = useMemo(() => new Map(board.segments.map((s) => [s.id, s.state])), [board]);
  const visible = accounts.filter((a) => {
    const state = stateById.get(a.id);
    if (filter === "ready") return state === "ready";
    if (filter === "attention") return state !== "ready";
    return true;
  });
  const failedCount = accounts.filter((a) => a.effective_status === "failed").length;
  const tenantsInUse = new Set(accounts.map((a) => a.tenant_id));
  const tenantName = (id: string) => catalog.tenants.find((t) => t.id === id)?.name ?? id;

  function toggle(id: number) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  return (
    <div className="accounts-view">
      <ReadinessBoard readiness={board} nextRun={nextRun} />
      <RunControls
        running={run.running}
        selectedIds={[...selected]}
        failedCount={failedCount}
        showBrowser={showBrowser}
        onShowBrowserChange={onShowBrowserChange}
        onRun={(selection) => onRun(selection, showBrowser)}
        onStop={onStop}
      />

      <div className="list-toolbar">
        <div className="segmented" role="radiogroup" aria-label="Show accounts">
          {(
            [
              ["all", `All (${accounts.length})`],
              ["attention", "Not ready"],
              ["ready", "Logged in"],
            ] as const
          ).map(([value, label]) => (
            <button key={value} role="radio" aria-checked={filter === value} onClick={() => setFilter(value)}>
              {label}
            </button>
          ))}
        </div>
        <div className="button-row">
          <button className="button" onClick={pasteFromCirrus}>
            Paste from Cirrus
          </button>
          <button className="button" onClick={() => setDrawer({ account: null })}>
            Add account
          </button>
        </div>
      </div>

      {accounts.length === 0 ? (
        <div className="empty">
          <h2>No accounts yet</h2>
          <p className="muted">
            Add your broker accounts once. In Cirrus, use “Copy for AutoLogin” (one account or all of them), then choose
            Paste from Cirrus here.
          </p>
          <div className="button-row">
            <button className="button primary" onClick={pasteFromCirrus}>
              Paste from Cirrus
            </button>
            <button className="button" onClick={() => setDrawer({ account: null })}>
              Add account
            </button>
          </div>
        </div>
      ) : (
        <AccountTable
          accounts={visible}
          live={run.accounts}
          selected={selected}
          showWorkspace={tenantsInUse.size > 1}
          tenantName={tenantName}
          onToggle={toggle}
          onToggleAll={(all) => setSelected(all ? new Set(visible.map((a) => a.id)) : new Set())}
          onEdit={(account) => setDrawer({ account })}
        />
      )}

      {bulk && <BulkAddDialog catalog={catalog} paste={bulk} onClose={() => setBulk(null)} onDone={onChanged} />}
      {!bulk && incomingImport && (
        <BulkAddDialog catalog={catalog} paste={incomingImport} onClose={onIncomingImportDone} onDone={onChanged} />
      )}
      {drawer && (
        <AccountDrawer catalog={catalog} account={drawer.account} onClose={() => setDrawer(null)} onSaved={onChanged} />
      )}
    </div>
  );
}
