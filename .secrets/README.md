# .secrets

Signing keys for AutoLogin releases. Git ignores everything in this folder
except this file. Create the keys with `scripts/signing-keys.sh`, which puts
them here with owner-only permissions and uploads them as GitHub secrets.

| File | What it signs | GitHub secret(s) | Public half goes to |
| --- | --- | --- | --- |
| `android-release.jks` | Android APK | `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` | (none; Android pins the certificate on first install) |
| `updater.key` (+ `updater.key.pub`) | Desktop auto-updates | `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | `plugins.updater.pubkey` in `app/src-tauri/tauri.conf.json` |
| `manifest.key` | Broker manifest updates | `MANIFEST_SIGNING_KEY` | `TRUSTED_MANIFEST_KEYS` in `app/src-tauri/src/broker/remote.rs`, and the `MANIFEST_PUBLIC_KEY` repository variable |

The Cirrus paste-signing key belongs to the broker-auth backend
(`AUTOLOGIN_SIGNING_KEY`), not to this repository.

Rules:

- Back up every file here, with its password, to a password manager. Losing
  `android-release.jks` means phones can't install updates (users would have
  to uninstall, which wipes their saved accounts). Losing `updater.key` means
  desktop apps stop auto-updating.
- Never commit, paste or share these files. Only the public halves go into
  the code.
- This folder is inside the repository, so anything that copies or uploads
  the whole working tree (zips, cloud sync, AI tools) would include it. The
  repository's `.claude/settings.json` stops Claude from reading it.
