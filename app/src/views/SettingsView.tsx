import { useState } from "react";
import { ExportDialog } from "../components/ExportDialog";
import { ImportDialog } from "../components/ImportDialog";
import { PhoneScheduleSetup } from "../components/PhoneScheduleSetup";
import { api } from "../lib/api";
import { DEVICE, IS_MOBILE } from "../lib/platform";
import type { AppInfo, AppSettings, CommandError, SettingsView as View, Weekday } from "../lib/types";
import "./SettingsView.css";

const DAYS: Weekday[] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const RECOMMENDED_TIME = "08:45";
const RETRY_CHOICES = [null, 5, 10, 15, 30];

interface Props {
  view: View;
  info: AppInfo;
  onSaved: (view: View) => void;
  onAccountsChanged: () => void;
  onShowPrivacy: () => void;
  onToast: (message: string) => void;
}

export function SettingsView({ view, info, onSaved, onAccountsChanged, onShowPrivacy, onToast }: Props) {
  const { settings } = view;
  const [dialog, setDialog] = useState<"export" | "import" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const time = settings.schedule.time.slice(0, 5);

  async function checkUpdates() {
    setChecking(true);
    try {
      const status = await api.checkForUpdate();
      // An available update installs and restarts the app; otherwise say so.
      if (status.state === "up_to_date") onToast(`You have the latest version (${info.version}).`);
    } catch (e) {
      onToast((e as CommandError).message);
    } finally {
      setChecking(false);
    }
  }

  async function update(change: (s: AppSettings) => AppSettings) {
    setError(null);
    try {
      onSaved(await api.saveSettings(change(settings)));
    } catch (e) {
      setError((e as CommandError).message);
    }
  }

  const setSchedule = (patch: Partial<AppSettings["schedule"]>) =>
    update((s) => ({ ...s, schedule: { ...s.schedule, ...patch } }));

  const toggleDay = (day: Weekday) =>
    setSchedule({
      days: settings.schedule.days.includes(day)
        ? settings.schedule.days.filter((d) => d !== day)
        : DAYS.filter((d) => d === day || settings.schedule.days.includes(d)),
    });

  return (
    <div className="settings">
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}

      <section className="settings-section">
        <h2>Daily login</h2>
        <label className="toggle-row">
          <input
            type="checkbox"
            checked={settings.schedule.enabled}
            onChange={(e) => setSchedule({ enabled: e.target.checked })}
          />
          Log in all accounts {IS_MOBILE ? "every day" : "automatically every day"}
        </label>
        <div className="schedule-row" aria-disabled={!settings.schedule.enabled}>
          <div className="field">
            <label htmlFor="schedule-time">Time</label>
            <input
              id="schedule-time"
              className="input time-input"
              type="time"
              value={time}
              disabled={!settings.schedule.enabled}
              onChange={(e) => e.target.value && setSchedule({ time: `${e.target.value}:00` })}
            />
          </div>
          <fieldset className="days" disabled={!settings.schedule.enabled}>
            <legend>Days</legend>
            {DAYS.map((day) => (
              <label key={day} className="day">
                <input type="checkbox" checked={settings.schedule.days.includes(day)} onChange={() => toggleDay(day)} />
                {day}
              </label>
            ))}
          </fieldset>
        </div>
        <p className="recommendation">
          {view.recommendation}
          {time !== RECOMMENDED_TIME && (
            <>
              {" "}
              <button className="button quiet link" onClick={() => setSchedule({ time: `${RECOMMENDED_TIME}:00` })}>
                Use 8:45 AM
              </button>
            </>
          )}
        </p>
        {IS_MOBILE && <PhoneScheduleSetup schedule={settings.schedule} onChange={setSchedule} />}
        {view.next_scheduled_run && (
          <p className="muted">Next automatic login: {formatNextRun(view.next_scheduled_run)}.</p>
        )}
        <label className="toggle-row">
          <input
            type="checkbox"
            checked={!settings.schedule.headless}
            onChange={(e) => setSchedule({ headless: !e.target.checked })}
          />
          Show the browser during automatic logins
        </label>
        <div className="field narrow">
          <label htmlFor="retry-failed">If some accounts fail, try them again</label>
          <select
            id="retry-failed"
            className="input"
            value={settings.schedule.retry_failed_after_minutes ?? ""}
            onChange={(e) =>
              setSchedule({
                retry_failed_after_minutes: e.target.value ? Number(e.target.value) : null,
              })
            }
          >
            {RETRY_CHOICES.map((m) => (
              <option key={m ?? "off"} value={m ?? ""}>
                {m ? `After ${m} minutes` : "Don't retry"}
              </option>
            ))}
          </select>
        </div>
        {!IS_MOBILE && (
          <label className="toggle-row">
            <input
              type="checkbox"
              checked={settings.start_with_computer}
              onChange={(e) => update((s) => ({ ...s, start_with_computer: e.target.checked }))}
            />
            Start AutoLogin when I sign in to my computer (needed for automatic logins)
          </label>
        )}
      </section>

      <section className="settings-section">
        <h2>Logging in</h2>
        <label className="toggle-row">
          <input
            type="checkbox"
            checked={!settings.manual_headless}
            onChange={(e) => update((s) => ({ ...s, manual_headless: !e.target.checked }))}
          />
          Show the browser when I log in manually
        </label>
        <div className="field narrow">
          <label htmlFor="retries">Extra attempts when a broker page doesn't load</label>
          <select
            id="retries"
            className="input"
            value={settings.retries}
            onChange={(e) => update((s) => ({ ...s, retries: Number(e.target.value) }))}
          >
            {[0, 1, 2, 3].map((n) => (
              <option key={n} value={n}>
                {n === 0 ? "None" : n}
              </option>
            ))}
          </select>
          <span className="hint">
            AutoLogin never retries after a broker rejects your password, so your account won't get locked.
          </span>
        </div>
        <div className="field narrow">
          <label htmlFor="concurrency">Accounts to log in at the same time</label>
          <select
            id="concurrency"
            className="input"
            value={settings.concurrency}
            onChange={(e) => update((s) => ({ ...s, concurrency: Number(e.target.value) }))}
          >
            {[1, 2, 3, 4, 5, 6, 8, 10].map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </div>
      </section>

      <section className="settings-section">
        <h2>Move to another {DEVICE}</h2>
        <p className="muted">Export your accounts here, then import the file in AutoLogin on the new {DEVICE}.</p>
        <div className="button-row">
          <button className="button" onClick={() => setDialog("export")}>
            Export accounts
          </button>
          <button className="button" onClick={() => setDialog("import")}>
            Import accounts
          </button>
        </div>
      </section>

      {!IS_MOBILE && (
        <section className="settings-section">
          <h2>Updates</h2>
          <label className="toggle-row">
            <input
              type="checkbox"
              checked={settings.auto_update}
              onChange={(e) => update((s) => ({ ...s, auto_update: e.target.checked }))}
            />
            Install updates automatically (never during a login or in the 30 minutes before your daily login)
          </label>
          <div className="button-row">
            <button className="button" disabled={checking} onClick={checkUpdates}>
              {checking ? "Checking…" : "Check for updates"}
            </button>
          </div>
        </section>
      )}

      <section className="settings-section">
        <h2>About</h2>
        <p>
          AutoLogin {info.version}, broker definitions version {info.manifest_version}. Free and open source, built by
          Cirrus.
        </p>
        <div className="button-row">
          <button className="button" onClick={onShowPrivacy}>
            How AutoLogin handles your data
          </button>
          {!IS_MOBILE && (
            <button className="button" onClick={() => api.openFolder("data")}>
              Open data folder
            </button>
          )}
        </div>
      </section>

      {dialog === "export" && <ExportDialog onClose={() => setDialog(null)} onDone={onToast} />}
      {dialog === "import" && <ImportDialog onClose={() => setDialog(null)} onImported={onAccountsChanged} />}
    </div>
  );
}

export function formatNextRun(iso: string): string {
  return new Date(iso).toLocaleString([], {
    weekday: "long",
    hour: "numeric",
    minute: "2-digit",
  });
}
