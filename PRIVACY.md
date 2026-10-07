# How AutoLogin handles your data

AutoLogin is a free, open-source app for desktop and Android, built by Cirrus.
It automates the daily broker login you would otherwise do by hand on
cirrus.trade. It is a **runner on your own computer or phone**, not a service:
it has no server, no account database and no analytics.

## What stays on your device

- **Passwords, PINs, TOTP secrets** (and, for Dhan, your mobile number).
  Encrypted (XChaCha20-Poly1305) in AutoLogin's local database with a random
  key that your device's secure key storage protects: Keychain on macOS,
  Credential Manager on Windows, Secret Service on Linux, and the Android
  Keystore on phones, where the key can't be copied off the device. Using one
  key for all accounts means the system asks for permission once, not once
  per account. Secrets are never written to a plain file, never shown back in
  the app, and never sent to Cirrus. AutoLogin's own developers cannot see
  them.
- **Account list and login history** (broker, client ID, last login time,
  status). Stored in a local database in your user data folder.
- **Failure screenshots.** When a login fails, a screenshot of the broker page
  is saved locally so you can see what went wrong. They are deleted after 7
  days and never uploaded.

## Why Cirrus doesn't keep your credentials

Logging in every morning needs your password, PIN and TOTP secret in a usable
form. If Cirrus stored them on its servers, those servers would have to be
able to decrypt them each day, so the key would have to live right next to the
data. However many times the credentials were encrypted, a server breach or a
bad insider could then expose every user's broker logins at once.

So AutoLogin keeps them on your own device, locked with a key only that device
holds. There is no central place holding everyone's broker credentials, and
nothing for a Cirrus breach to leak. Your device still needs ordinary care:
keep it updated and locked, as you would for your banking apps.

## Where AutoLogin connects, and what it sends

| Destination | Why | What is sent |
|---|---|---|
| Your broker's official login page or API (e.g. kite.zerodha.com, auth.dhan.co) | To log in | Your credentials and a one-time TOTP code, exactly as if you typed them |
| Cirrus (broker-auth-api.cirrus.trade, app.cirrus.trade) | To link the resulting broker session to your Cirrus account | The one-time authorization code the broker returns after login, never your password, PIN or TOTP secret |
| Cirrus, for Dhan only (broker-auth-api.cirrus.trade) | Dhan logins start from a single-use consent that Cirrus requests from Dhan | Your Dhan client ID only |
| GitHub (github.com/pratikbhadane24/autologin) | To check for app updates and broker-flow fixes | Nothing personal, only a version check |

**Not supported yet: Kotak Neo and Firstock.** Their logins run on Cirrus's
server, so your PIN or password and a one-time TOTP code would pass through
Cirrus to the broker (Cirrus wouldn't store them). If AutoLogin adds them, the
app will mark these brokers and ask before using them.

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
