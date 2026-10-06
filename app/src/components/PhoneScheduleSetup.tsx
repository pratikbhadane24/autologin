import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type { PhoneSetting, PhoneStatus, Schedule } from "../lib/types";
import "./PhoneScheduleSetup.css";

interface Props {
  schedule: Schedule;
  onChange: (patch: Partial<Schedule>) => void;
}

interface Need {
  key: keyof PhoneStatus;
  setting: PhoneSetting;
  label: string;
  why: string;
}

const NOTIFICATIONS: Need = {
  key: "notifications",
  setting: "notifications",
  label: "Notifications",
  why: "So AutoLogin can tell you when it's time to log in, and how it went.",
};

// Everything automatic mode depends on, in the order users should grant it.
const AUTOMATIC_NEEDS: Need[] = [
  NOTIFICATIONS,
  {
    key: "exact_alarms",
    setting: "exactAlarms",
    label: "Alarms & reminders",
    why: "Starts the login at exactly the time you chose.",
  },
  {
    key: "overlay",
    setting: "overlay",
    label: "Display over other apps",
    why: "Android only lets an app open itself in the background with this.",
  },
  {
    key: "battery_unrestricted",
    setting: "battery",
    label: "Battery: unrestricted",
    why: "Stops battery saving from closing AutoLogin before it finishes.",
  },
];

/** Phone-only part of Daily login: how the run starts, and its permissions. */
export function PhoneScheduleSetup({ schedule, onChange }: Props) {
  const [status, setStatus] = useState<PhoneStatus | null>(null);

  const refresh = useCallback(() => {
    api.phoneScheduleStatus().then(setStatus, () => setStatus(null));
  }, []);

  useEffect(() => {
    refresh();
    // Permissions are granted in system screens; re-check on the way back.
    const onVisible = () => document.visibilityState === "visible" && refresh();
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  }, [refresh]);

  const needs = schedule.phone_automatic ? AUTOMATIC_NEEDS : [NOTIFICATIONS];
  const missing = status ? needs.filter((need) => !status[need.key]) : [];

  return (
    <div className="phone-schedule">
      <fieldset className="phone-mode" disabled={!schedule.enabled}>
        <legend>At the chosen time</legend>
        <label>
          <input
            type="radio"
            name="phone-mode"
            checked={!schedule.phone_automatic}
            onChange={() => onChange({ phone_automatic: false })}
          />
          <span>
            Remind me with a notification
            <small>One tap opens AutoLogin and logs in. Works on every phone.</small>
          </span>
        </label>
        <label>
          <input
            type="radio"
            name="phone-mode"
            checked={schedule.phone_automatic}
            onChange={() => onChange({ phone_automatic: true })}
          />
          <span>
            Log in without a tap
            <small>Needs the permissions below. Some phones still close apps to save battery.</small>
          </span>
        </label>
      </fieldset>

      {schedule.enabled && missing.length > 0 && (
        <ul className="phone-needs" aria-label="Still needed">
          {missing.map((need) => (
            <li key={need.key}>
              <div>
                <strong>{need.label}</strong>
                <small>{need.why}</small>
              </div>
              <button className="button" onClick={() => api.openPhoneSetting(need.setting).finally(refresh)}>
                Allow
              </button>
            </li>
          ))}
        </ul>
      )}
      {schedule.enabled && status && missing.length === 0 && schedule.phone_automatic && (
        <p className="muted">Everything is set up for logging in without a tap.</p>
      )}
    </div>
  );
}
