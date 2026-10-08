import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { api } from "../lib/api";
import type { CommandError, FileKind, ImportReport } from "../lib/types";
import { Modal } from "./Modal";
import { SecretInput } from "./SecretInput";

interface Props {
  onClose: () => void;
  onImported: () => void;
}

export function ImportDialog({ onClose, onImported }: Props) {
  const [path, setPath] = useState<string | null>(null);
  const [kind, setKind] = useState<FileKind | null>(null);
  const [password, setPassword] = useState("");
  const [report, setReport] = useState<ImportReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function chooseFile() {
    setError(null);
    const picked = await open({
      multiple: false,
      filters: [{ name: "AutoLogin backup or CSV", extensions: ["autologin", "json", "csv"] }],
    });
    if (typeof picked !== "string") return;
    try {
      setKind(await api.inspectImport(picked));
      setPath(picked);
    } catch (e) {
      setError((e as CommandError).message);
    }
  }

  async function importNow() {
    if (!path) return;
    setError(null);
    try {
      setReport(await api.importAccounts(path, kind === "encrypted" ? password : undefined));
      onImported();
    } catch (e) {
      setError((e as CommandError).message);
    }
  }

  if (report) {
    return (
      <Modal title="Import finished" onClose={onClose} footer={<button className="button primary" onClick={onClose}>Done</button>}>
        <p>
          Added {report.added}, updated {report.updated}.
          {report.needs_setup > 0 && ` ${report.needs_setup} need their password, PIN or TOTP secret added before they can log in.`}
        </p>
        {report.problems.length > 0 && (
          <>
            <p>These rows were not imported:</p>
            <ul>
              {report.problems.map((p) => (
                <li key={p.row}>
                  Row {p.row}: {p.reason}
                </li>
              ))}
            </ul>
          </>
        )}
      </Modal>
    );
  }

  return (
    <Modal
      title="Import accounts"
      onClose={onClose}
      footer={
        <>
          <button className="button" onClick={onClose}>
            Cancel
          </button>
          <button className="button primary" disabled={!path || (kind === "encrypted" && !password)} onClick={importNow}>
            Import
          </button>
        </>
      }
    >
      <p className="muted">
        Use a backup exported from AutoLogin, or a CSV file (including CSV exports from AutoLogin 1.x). Accounts that already
        exist are updated, not duplicated.
      </p>
      <div>
        <button className="button" onClick={chooseFile}>
          {path ? "Choose a different file" : "Choose file"}
        </button>
        {path && <p className="muted">{path.split(/[\\/]/).pop()}</p>}
      </div>
      {kind === "encrypted" && (
        <div className="field">
          <label htmlFor="import-password">Backup password</label>
          <SecretInput id="import-password" label="Backup password" value={password} onChange={setPassword} />
        </div>
      )}
      {error && <p className="form-error" role="alert">{error}</p>}
    </Modal>
  );
}
