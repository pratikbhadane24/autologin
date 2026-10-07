import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useReducer, useState } from "react";
import { api } from "./api";
import { idleRun, reduceRun, type RunState } from "./runState";
import type { Account, CommandError, RunEvent } from "./types";

/** Live run progress; `onFinished` fires once per completed run. */
export function useRun(onFinished: () => void): RunState {
  const [state, dispatch] = useReducer(reduceRun, idleRun);

  useEffect(() => {
    const unlisten = listen<RunEvent>("run-event", ({ payload }) => {
      dispatch(payload);
      if (payload.type === "account_finished" || payload.type === "run_finished") onFinished();
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, [onFinished]);

  // A run may already be going (started by the scheduler before the window opened).
  useEffect(() => {
    api.isRunning().then((running) => {
      if (running) dispatch({ type: "run_started", run_id: 0, total: 0 });
    });
  }, []);

  return state;
}

export function useAccounts() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [error, setError] = useState<CommandError | null>(null);

  const refresh = useCallback(() => {
    api
      .listAccounts()
      .then((list) => {
        setAccounts(list);
        setError(null);
      })
      .catch(setError);
  }, []);

  useEffect(refresh, [refresh]);
  return { accounts, error, refresh };
}

/** "8 min ago", "Yesterday 08:46", or a date. */
export function relativeTime(iso: string | null, now = new Date()): string {
  if (!iso) return "Never";
  const then = new Date(iso);
  const minutes = Math.round((now.getTime() - then.getTime()) / 60000);
  const time = then.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (minutes < 1) return "Just now";
  if (minutes < 60) return `${minutes} min ago`;
  if (then.toDateString() === now.toDateString()) return `Today ${time}`;
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (then.toDateString() === yesterday.toDateString()) return `Yesterday ${time}`;
  return then.toLocaleDateString([], { day: "numeric", month: "short" }) + ` ${time}`;
}
