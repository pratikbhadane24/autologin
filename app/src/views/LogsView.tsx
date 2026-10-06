import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import type { CommandError } from "../lib/types";
import "./LogsView.css";

const LINES = 400;
const REFRESH_MS = 2000;

/** Recent activity from the log file. Secrets are masked before logging. */
export function LogsView({ running }: { running: boolean }) {
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const bottom = useRef<HTMLDivElement>(null);

  const load = useCallback(() => {
    api
      .recentLogs(LINES)
      .then((logs) => {
        setText(logs);
        setError(null);
      })
      .catch((e: CommandError) => setError(e.message));
  }, []);

  useEffect(load, [load]);
  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(load, REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [running, load]);
  useEffect(() => bottom.current?.scrollIntoView({ block: "end" }), [text]);

  return (
    <div className="logs">
      <div className="button-row">
        <button className="button" onClick={load}>
          Refresh
        </button>
        <button className="button" onClick={() => api.openFolder("logs")}>
          Open log folder
        </button>
        <button className="button" onClick={() => api.openFolder("failures")}>
          Open failure screenshots
        </button>
      </div>
      <p className="muted">
        Logs never contain your passwords, PINs or TOTP secrets. If you report a problem, you can attach the log file.
      </p>
      {error && <p className="form-error">{error}</p>}
      <pre className="log-text" tabIndex={0} aria-label="Recent activity">
        {text || "No activity yet."}
        <div ref={bottom} />
      </pre>
    </div>
  );
}
