import { save } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { api } from "../lib/api";
import type { CommandError, ExportFormat } from "../lib/types";
import { Modal } from "./Modal";
import { SecretInput } from "./SecretInput";

const MIN_PASSWORD = 8;

const FORMATS: { value: ExportFormat; title: string; detail: string; extension: string }[] = [
  {
    value: "encrypted",
    title: "Password-protected backup (recommended)",
    detail: "Everything, including passwords, PINs and TOTP secrets, locked with a password you choose.",
    extension: "autologin",
  },
  {
    value: "plain",
    title: "Unprotected backup",
    detail: "Everything, including secrets, in a readable file. Anyone who gets the file can read them.",
    extension: "json",
  },
  {
    value: "csv",
    title: "Spreadsheet (CSV) without secrets",
    detail: "Broker, client ID, name and API keys only. You'll re-enter passwords, PINs and TOTP secrets after importing.",
    extension: "csv",
  },
];

interface Props {
  onClose: () => void;
  onDone: (message: string) => void;
}

export function ExportDialog({ onClose, onDone }: Props) {
  const [format, setFormat] = useState<ExportFormat>("encrypted");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [understood, setUnderstood] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const chosen = FORMATS.find((f) => f.value === format)!;
  const passwordProblem =
    format !== "encrypted"
      ? null
      : password.length < MIN_PASSWORD
        ? `Use at least ${MIN_PASSWORD} characters.`
        : password !== confirm
          ? "The two passwords don't match."
          : null;
  const ready = format === "encrypted" ? passwordProblem === null : format === "plain" ? understood : true;

  async function exportNow() {
    setError(null);
    const date = new Date().toISOString().slice(0, 10);
    const path = await save({
      defaultPath: `autologin-backup-${date}.${chosen.extension}`,
      filters: [{ name: chosen.title, extensions: [chosen.extension] }],
    });
    if (!path) return;
    try {
      const count = await api.exportAccounts(path, format, format === "encrypted" ? password : undefined);
      onDone(`Exported ${count} account${count === 1 ? "" : "s"}`);
      onClose();
    } catch (e) {
      setError((e as CommandError).message);
    }
  }

  return (
    <Modal
      title="Export accounts"
      onClose={onClose}
      footer={
        <>
          <button className="button" onClick={onClose}>
            Cancel
          </button>
          <button className="button primary" disabled={!ready} onClick={exportNow}>
            Export
          </button>
        </>
      }
    >
      {FORMATS.map((f) => (
        <label key={f.value} className="choice">
          <input type="radio" name="format" checked={format === f.value} onChange={() => setFormat(f.value)} />
          <strong>{f.title}</strong>
          <span>{f.detail}</span>
        </label>
      ))}

      {format === "encrypted" && (
        <>
          <div className="field">
            <label htmlFor="export-password">Backup password</label>
            <SecretInput id="export-password" label="Backup password" value={password} onChange={setPassword} />
          </div>
          <div className="field">
            <label htmlFor="export-confirm">Type it again</label>
            <SecretInput id="export-confirm" label="Backup password again" value={confirm} onChange={setConfirm} />
            {password && passwordProblem && <span className="error">{passwordProblem}</span>}
            <span className="hint">There is no way to recover this password. Without it the backup can't be opened.</span>
          </div>
        </>
      )}
      {format === "plain" && (
        <label className="warning">
          <input type="checkbox" checked={understood} onChange={(e) => setUnderstood(e.target.checked)} /> I understand this file
          shows my passwords, PINs and TOTP secrets to anyone who opens it.
        </label>
      )}
      {error && <p className="form-error" role="alert">{error}</p>}
    </Modal>
  );
}
