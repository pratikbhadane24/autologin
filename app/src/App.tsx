import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { PrivacyScreen } from "./components/PrivacyScreen";
import { WhatsNew } from "./components/WhatsNew";
import { api } from "./lib/api";
import { IS_MOBILE } from "./lib/platform";
import { useAccounts, useRun } from "./lib/hooks";
import type {
  AppInfo,
  Catalog,
  CommandError,
  MigrationOutcome,
  PasteResult,
  Selection,
  SettingsView as SettingsData,
  UpdateStatus,
} from "./lib/types";
import { AccountsView } from "./views/AccountsView";
import { LogsView } from "./views/LogsView";
import { formatNextRun, SettingsView } from "./views/SettingsView";
import "./App.css";

type Tab = "accounts" | "settings" | "activity";
const TABS: { id: Tab; label: string }[] = [
  { id: "accounts", label: "Accounts" },
  { id: "settings", label: "Schedule & settings" },
  { id: "activity", label: "Activity log" },
];
const TOAST_MS = 4000;

export default function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [tab, setTab] = useState<Tab>("accounts");
  const [toast, setToast] = useState<string | null>(null);
  const [migration, setMigration] = useState<MigrationOutcome | null>(null);
  const [privacyOpen, setPrivacyOpen] = useState(false);
  const [whatsNewOpen, setWhatsNewOpen] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [incomingImport, setIncomingImport] = useState<PasteResult | null>(null);

  // Accounts sent from Cirrus with an autologin://import link: parsed with the
  // same signature/expiry checks as a paste, then confirmed in bulk add.
  const collectImport = useCallback(async () => {
    try {
      const text = await api.takePendingImport();
      if (!text) return;
      const result = await api.parsePaste(text);
      setTab("accounts");
      setIncomingImport(result);
    } catch (e) {
      setToast((e as CommandError).message);
    }
  }, []);

  useEffect(() => {
    void collectImport();
    const unlisten = listen("cirrus-import", () => void collectImport());
    return () => {
      unlisten.then((stop) => stop());
    };
  }, [collectImport]);

  const { accounts, refresh } = useAccounts();
  const refreshAfterRun = useCallback(() => refresh(), [refresh]);
  const run = useRun(refreshAfterRun);

  useEffect(() => {
    Promise.all([api.appInfo(), api.catalog(), api.settingsView(), api.takeMigrationOutcome()])
      .then(([appInfo, cat, view, outcome]) => {
        setInfo(appInfo);
        setCatalog(cat);
        setSettings(view);
        setMigration(outcome);
        setPrivacyOpen(!appInfo.privacy_acknowledged);
        setWhatsNewOpen(appInfo.show_whats_new);
      })
      .catch((e: CommandError) => setLoadError(e.message));
  }, []);

  // Broker definitions were updated in the background (signed remote update).
  useEffect(() => {
    const unlisten = listen<number>("catalog-updated", () => {
      api.catalog().then(setCatalog).catch(() => undefined);
      refresh();
      setToast("Broker updates installed.");
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, [refresh]);

  useEffect(() => {
    const unlisten = listen<UpdateStatus>("update-status", ({ payload }) => setUpdate(payload));
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), TOAST_MS);
    return () => window.clearTimeout(timer);
  }, [toast]);

  const showMessage = useCallback(
    (message: string) => {
      setToast(message);
      refresh();
    },
    [refresh],
  );

  async function startRun(selection: Selection, showBrowser: boolean) {
    try {
      await api.runAccounts(selection, !showBrowser);
    } catch (e) {
      setToast((e as CommandError).message);
    }
  }

  async function setShowBrowser(show: boolean) {
    if (!settings) return;
    try {
      setSettings(await api.saveSettings({ ...settings.settings, manual_headless: !show }));
    } catch (e) {
      setToast((e as CommandError).message);
    }
  }

  async function installUpdate() {
    try {
      await api.checkForUpdate();
    } catch (e) {
      setToast((e as CommandError).message);
    }
  }

  async function closePrivacy() {
    if (info && !info.privacy_acknowledged) {
      await api.acknowledgePrivacy();
      setInfo({ ...info, privacy_acknowledged: true });
    }
    setPrivacyOpen(false);
  }

  async function closeWhatsNew() {
    await api.markWhatsNewSeen();
    setWhatsNewOpen(false);
  }

  if (loadError) {
    return (
      <main className="fatal">
        <h1>AutoLogin couldn't start</h1>
        <p>{loadError}</p>
        <p className="muted">Restart the app. If this keeps happening, share the latest log from {IS_MOBILE ? "the Activity log tab" : "the log folder in the tray menu"}.</p>
      </main>
    );
  }
  if (!info || !catalog || !settings) return null;

  const nextRun = settings.next_scheduled_run ? `Next automatic login: ${formatNextRun(settings.next_scheduled_run)}.` : null;

  return (
    <div className="shell">
      <header className="topbar">
        <span className="wordmark">AutoLogin</span>
        <nav className="tabs" aria-label="Sections">
          {TABS.map((t) => (
            <button key={t.id} className="tab" aria-current={tab === t.id ? "page" : undefined} onClick={() => setTab(t.id)}>
              {t.label}
            </button>
          ))}
        </nav>
      </header>

      <main className="content">
        {update && update.state !== "up_to_date" && <UpdateBanner status={update} onInstall={installUpdate} />}
        {migration && (
          <MigrationBanner outcome={migration} onDismiss={() => setMigration(null)} />
        )}
        {tab === "accounts" && (
          <AccountsView
            catalog={catalog}
            accounts={accounts}
            run={run}
            nextRun={nextRun}
            showBrowser={!settings.settings.manual_headless}
            onShowBrowserChange={setShowBrowser}
            onRun={startRun}
            onStop={() => api.stopRun()}
            onChanged={showMessage}
            incomingImport={incomingImport}
            onIncomingImportDone={() => setIncomingImport(null)}
          />
        )}
        {tab === "settings" && (
          <SettingsView
            view={settings}
            info={info}
            onSaved={setSettings}
            onAccountsChanged={refresh}
            onShowPrivacy={() => setPrivacyOpen(true)}
            onToast={setToast}
          />
        )}
        {tab === "activity" && <LogsView running={run.running} />}
      </main>

      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
      {privacyOpen && <PrivacyScreen firstRun={!info.privacy_acknowledged} onDone={closePrivacy} />}
      {!privacyOpen && whatsNewOpen && <WhatsNew version={info.version} onClose={closeWhatsNew} />}
    </div>
  );
}

function UpdateBanner({ status, onInstall }: { status: UpdateStatus; onInstall: () => void }) {
  if (status.state === "downloading") {
    return (
      <div className="banner banner-info" role="status">
        <p>Updating AutoLogin{status.percent !== null ? ` (${status.percent}%)` : ""}. It will restart by itself.</p>
      </div>
    );
  }
  if (status.state === "failed") {
    return (
      <div className="banner banner-fail" role="alert">
        <p>The update couldn't be installed: {status.message}. AutoLogin will try again later.</p>
      </div>
    );
  }
  if (status.state !== "available") return null;
  return (
    <div className="banner banner-info" role="status">
      <p>
        AutoLogin {status.version} is ready. It installs automatically when no login is running, or you can install it now.
      </p>
      <button className="button" onClick={onInstall}>
        Install now
      </button>
    </div>
  );
}

function MigrationBanner({ outcome, onDismiss }: { outcome: MigrationOutcome; onDismiss: () => void }) {
  const { added, updated, needs_setup } = outcome.report;
  const brought = added + updated;
  return (
    <div className="banner" role="status">
      <p>
        Brought over {brought} account{brought === 1 ? "" : "s"} from AutoLogin 1.x. Your passwords are now encrypted, no longer in a plain file.
        {needs_setup > 0 && ` ${needs_setup} need a password, PIN or TOTP secret added.`}
        {outcome.waiting_brokers.length > 0 &&
          ` Accounts for ${outcome.waiting_brokers.join(", ")} will appear once AutoLogin supports those brokers.`}
      </p>
      <button className="button quiet" onClick={onDismiss}>
        Dismiss
      </button>
    </div>
  );
}
