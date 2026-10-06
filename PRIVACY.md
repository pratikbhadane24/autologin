# How AutoLogin handles your data

AutoLogin is a free, open-source desktop app built by Cirrus. It automates the
daily broker login you would otherwise do by hand on cirrus.trade. It is a
**runner on your computer**, not a service: it has no server, no account
database and no analytics.

## What stays on your computer

- **Passwords, PINs, TOTP secrets.** Encrypted (XChaCha20-Poly1305) in
  AutoLogin's local database with a random key that is kept in your operating
  system's secure store (Keychain on macOS, Credential Manager on Windows,
  Secret Service on Linux). Using one key for all accounts means the system
  asks for permission once, not once per account. Secrets are never written to
  a plain file, never shown back in the app, and never sent to Cirrus.
  AutoLogin's own developers cannot see them.
- **Account list and login history** (broker, client ID, last login time,
  status). Stored in a local database in your user data folder.
- **Failure screenshots.** When a login fails, a screenshot of the broker page
  is saved locally so you can see what went wrong. They are deleted after 7
  days and never uploaded.

## Where AutoLogin connects, and what it sends

| Destination | Why | What is sent |
|---|---|---|
| Your broker's official login page or API (e.g. kite.zerodha.com, api-t2.fyers.in) | To log in | Your credentials and a one-time TOTP code, exactly as if you typed them |
| Cirrus (broker-auth-api.cirrus.trade, app.cirrus.trade) | To link the resulting broker session to your Cirrus account | The one-time authorization code the broker returns after login, never your password, PIN or TOTP secret |
| GitHub (github.com/pratikbhadane24/autologin) | To check for app updates and broker-flow fixes | Nothing personal, only a version check |
| Google Chrome for Testing (only if no Chrome or Edge is installed) | To download a browser to run logins in | Nothing personal |

**Exception: Kotak Neo and Firstock.** These two brokers are logged in by
Cirrus's server on your behalf. For them, your PIN or password and a one-time
TOTP code pass through Cirrus to the broker. Cirrus does not store them. The
app marks these brokers and asks before using them.

## What Cirrus stores

AutoLogin does not store anything on any server. **Cirrus**, the trading
platform you are logging in to, does store the broker **session token** your
broker issues after login, plus any broker API key you registered on Cirrus.
That is how Cirrus places orders for you, and it is the same thing that happens
when you log in on cirrus.trade in a browser. See the
[Cirrus privacy policy](https://cirrus.trade/privacy-policy).

## Verify it yourself

AutoLogin is open source under the MIT license. Every broker login step is a
readable file in [`app/src-tauri/brokers/`](app/src-tauri/brokers/), and every
network call is in the source. Logs never contain your secrets.

Questions or concerns: open an issue on GitHub.
