import { openUrl } from "@tauri-apps/plugin-opener";
import { DEVICE, IS_MOBILE } from "../lib/platform";
import "./PrivacyScreen.css";

const FULL_POLICY = "https://github.com/pratikbhadane24/autologin/blob/main/PRIVACY.md";

interface Props {
  /** First run requires an explicit "continue"; from Settings it just closes. */
  firstRun: boolean;
  onDone: () => void;
}

export function PrivacyScreen({ firstRun, onDone }: Props) {
  return (
    <div className="privacy-backdrop">
      <article className="privacy" aria-labelledby="privacy-title">
        <p className="privacy-from">AutoLogin by Cirrus</p>
        <h1 id="privacy-title">AutoLogin runs on your {DEVICE}. Your credentials stay here.</h1>

        <div className="privacy-points">
          <section>
            <h2>What it does</h2>
            <p>
              Every morning it opens your broker's own login page and signs in for you, exactly as you would by hand.
              Then it tells Cirrus the login is done.
            </p>
          </section>
          <section>
            <h2>Where your details are kept</h2>
            <p>
              Passwords, PINs and TOTP secrets are encrypted on this {DEVICE} with a key held in{" "}
              {IS_MOBILE ? "AutoLogin's private storage, which other apps can't read" : "your system keychain"}. They
              are sent only to your broker's official login page, never to Cirrus, and AutoLogin has no server of its
              own.
            </p>
          </section>
          <section>
            <h2>What Cirrus receives</h2>
            <p>
              The one-time code your broker gives after you log in. Cirrus keeps the broker session it gets from that
              code so it can place your orders, the same as when you log in on cirrus.trade.
            </p>
          </section>
          <section>
            <h2>Check it yourself</h2>
            <p>
              AutoLogin is open source. Every broker login step is a readable file, and logs never contain your secrets.
            </p>
          </section>
        </div>

        <div className="privacy-actions">
          <button className="button primary" onClick={onDone}>
            {firstRun ? "Continue" : "Close"}
          </button>
          <button className="button quiet" onClick={() => openUrl(FULL_POLICY)}>
            Read the full privacy details
          </button>
        </div>
      </article>
    </div>
  );
}
