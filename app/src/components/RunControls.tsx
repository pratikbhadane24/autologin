import type { Selection } from "../lib/types";

interface Props {
  running: boolean;
  selectedIds: number[];
  failedCount: number;
  showBrowser: boolean;
  onShowBrowserChange: (show: boolean) => void;
  onRun: (selection: Selection) => void;
  onStop: () => void;
}

export function RunControls({ running, selectedIds, failedCount, showBrowser, onShowBrowserChange, onRun, onStop }: Props) {
  if (running) {
    return (
      <div className="run-controls">
        <button className="button danger" onClick={onStop}>
          Stop logging in
        </button>
        <span className="muted">Logging in… you can keep using your computer.</span>
      </div>
    );
  }
  return (
    <div className="run-controls">
      {selectedIds.length > 0 ? (
        <button className="button primary" onClick={() => onRun({ kind: "ids", ids: selectedIds })}>
          Log in {selectedIds.length} selected
        </button>
      ) : (
        <button className="button primary" onClick={() => onRun({ kind: "all" })}>
          Log in all accounts
        </button>
      )}
      {failedCount > 0 && (
        <button className="button" onClick={() => onRun({ kind: "failed" })}>
          Retry {failedCount} failed
        </button>
      )}
      <label className="show-browser">
        <input type="checkbox" checked={showBrowser} onChange={(e) => onShowBrowserChange(e.target.checked)} />
        Show browser while logging in
      </label>
    </div>
  );
}
