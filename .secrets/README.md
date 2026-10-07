# .secrets

Signing keys for AutoLogin releases. Git ignores everything in this folder
except this file. Create the keys with `scripts/signing-keys.sh`, which puts
them here with owner-only permissions and uploads them as GitHub secrets.

| File | What it signs | GitHub secret(s) | Public half goes to |
| --- | --- | --- | --- |
| `android-release.jks` | Android APK | `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` | (none; Android pins the certificate on first install) |
| `updater.key` (+ `updater.key.pub`) | Desktop auto-updates | `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | `plugins.updater.pubkey` in `app/src-tauri/tauri.conf.json` |
| `manifest.key` | Broker manifest updates | `MANIFEST_SIGNING_KEY` | `TRUSTED_MANIFEST_KEYS` in `app/src-tauri/src/broker/remote.rs`, and the `MANIFEST_PUBLIC_KEY` repository variable |

| `paste.key` | "Send to AutoLogin" / "Copy for AutoLogin" from Cirrus | none: set `AUTOLOGIN_SIGNING_KEY` (+ `AUTOLOGIN_SIGNING_KEY_ID`) on the broker-auth backend | `TRUSTED_PASTE_KEYS` in `app/src-tauri/src/paste.rs` |

| `apple-developer-id.key` (+ `.csr`, `.cer`, `.p12`) | macOS app (Developer ID + notarization) | `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_TEAM_ID`, `APPLE_ID`, `APPLE_PASSWORD` | (none; Apple issues the certificate) |

macOS signing takes two runs: `scripts/signing-keys.sh apple-csr` makes the
private key and a request to upload to Apple (Developer ID Application); save
Apple's certificate as `.secrets/apple-developer-id.cer`, then run
`scripts/signing-keys.sh apple`.

Create the paste key with `scripts/signing-keys.sh paste`. It is never
uploaded to GitHub; its private half goes only into the Cirrus backend's
environment.

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
