# AutoLogin by Cirrus

Log in to all your broker accounts every morning with one click, or on a
schedule. Free and open source.

> **Your credentials never leave your computer.** AutoLogin runs locally and
> encrypts passwords, PINs and TOTP secrets with a key kept in your OS keychain. It sends them
> only to your broker's official login page. Read exactly what goes where in
> [PRIVACY.md](PRIVACY.md).

## Before you start

Add each broker account on [app.cirrus.trade](https://app.cirrus.trade) and log
in to it there once by hand. After that, AutoLogin can refresh the login every
day. AutoLogin does not ask you to sign in to Cirrus.

## Supported brokers

| Broker | Status |
|---|---|
| Pocketful, Zerodha, Upstox, Motilal Oswal, Dhan, 5Paisa (with your own API app) | v2.0 |
| Fyers (with your own API app), Angel One, Nuvama, Sharekhan, Jainam Lite, Kotak Neo, Firstock | Coming in v2.1 |
| Groww, AliceBlue, Tradejini | Planned |

Broker login steps are plain data files in
[`app/src-tauri/brokers/`](app/src-tauri/brokers/). When a broker changes its
login page, the fix ships as a signed broker update without a new app release.

## Features

- Logs in many accounts in parallel, each in an isolated private browser session
- Daily schedule, system tray, start with your computer
- Desktop notification with the success and failure summary
- Automatic retry, plus a screenshot of the page when a login fails
- Automatic updates with a "What's New" summary
- Upgrades from AutoLogin 1.x with no setup: accounts are imported on first launch

## Download

Get the installer for Windows, macOS or Linux from
[Releases](https://github.com/pratikbhadane24/autologin/releases/latest).

## Build from source

Requirements: Rust (stable), Node 20+, pnpm.

```sh
cd app
pnpm install
pnpm tauri dev                     # run in development
pnpm tauri build                   # build installers
cd src-tauri && cargo test         # run tests
```

## Contributing a broker fix

1. Edit or add `app/src-tauri/brokers/<broker>.toml`.
2. Run `cargo test` (it validates every broker file).
3. Open a pull request with a note on what changed on the broker's page.

## License

MIT. Built and maintained by Cirrus. Not affiliated with any broker listed.
