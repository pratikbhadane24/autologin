# Releasing AutoLogin v2

Releases are built by `.github/workflows/release-v2.yml`. CI for pull requests and pushes is in `ci-v2.yml`. The v1 workflows (`build.yml`, `release.yml`) belong to the Python app.

## Cut a release

1. Bump `version` in `app/src-tauri/Cargo.toml`. This is the only place the app version is set, because `tauri.conf.json` has no version. Keep `app/package.json` in step for tidiness.
2. Add an entry at the top of `RELEASES` in `app/src/content/whatsNew.ts`. Write it in the user's words. The GitHub release notes are generated separately from commits by git-cliff (`cliff.toml`).
3. Commit with a conventional message, e.g. `chore(release): prepare for v2.0.1`. git-cliff skips commits in this form.
4. Tag and push:
   ```sh
   git tag v2.0.1 && git push origin v2.0.1
   ```
5. The workflow runs these steps:
   - It checks that the Cargo.toml version equals the tag.
   - It creates a **draft** release with the git-cliff notes. Tags with a `-` suffix (`v2.1.0-beta.1`) are marked as pre-releases.
   - It builds Windows (MSI and NSIS; pre-releases get NSIS only, because WiX rejects non-numeric pre-release versions), macOS universal (`.dmg` and `.app.tar.gz`) and Linux (AppImage and `.deb`). It uploads these together with the updater `.sig` files and `latest.json`.
   - It builds a signed Android APK (`AutoLogin_<version>_android.apk`, arm64 and armv7), if the Android signing secrets are set. Without them the job warns and skips, and the rest of the release goes ahead.
   - It signs `brokers-manifest.json` and uploads it with `brokers-manifest.json.sig`.
   - For non-pre-releases, it checks that the release contains exactly one `.msi`, one `.dmg` and one `.AppImage`. The v1 updater depends on this (see below).
6. Check the draft, edit the notes if needed, then **Publish**. Installed apps see nothing until you publish. The v1 updater, the v2 broker-manifest fetch and the Tauri updater all read `releases/latest`, which excludes drafts and pre-releases.

To rebuild an existing tag, use **Actions → Release (v2) → Run workflow** with `tag` set. It reuses the existing release and overwrites its assets.

Commit groups in the notes:

| Commit | Heading |
| --- | --- |
| `feat:` | New |
| `fix:` | Fixed |
| any type with scope `brokers` (e.g. `fix(brokers): ...`, `feat(app,brokers): ...`) | Brokers |

git-cliff can't group commits by the paths they touch. Use the `brokers` scope for every change under `app/src-tauri/brokers/`.

## First beta checklist (v2.0.0-beta.1)

A pre-release tag (`v2.0.0-beta.1`) builds everything but stays invisible to installed apps and to the v1 updater, which only read `releases/latest`. That makes it the safe place to test.

Before tagging:

- [ ] Signing keys created and uploaded: `scripts/signing-keys.sh all` (Android, updater, manifest).
- [ ] Public keys in the app: `plugins.updater.pubkey` in `app/src-tauri/tauri.conf.json`, and the manifest key in `TRUSTED_MANIFEST_KEYS` (`app/src-tauri/src/broker/remote.rs`). Commit both.
- [ ] Apple secrets set if the macOS build should be signed and notarized (optional for a beta).
- [ ] Cirrus side live: broker-auth-backend (signed export, Dhan consent; `TRUSTED_PROXY_HOPS` set) and app-cirrus ("Send to AutoLogin", `/autologin`).
- [ ] CI green on `v2`, including the Android lint job.

After the draft is built, before publishing it as a pre-release:

- [ ] Windows: install v1.0.24, add a test account, then install the beta's NSIS installer. Check the accounts migrated, v1 was removed, and What's New showed.
- [ ] macOS (`.dmg`) and Linux (AppImage): install, add an account, run one login, check one keychain prompt at most.
- [ ] Android APK: install, add an account, log in once (shown and hidden), then install the same APK again over it and check the accounts stayed.
- [ ] Send to AutoLogin from Cirrus opens the installed app (`autologin://`) on each platform.
- [ ] Live logins for each broker you can test (Pocketful, Zerodha, Upstox, Motilal Oswal, Dhan).
- [ ] Schedule a run two minutes ahead on desktop and on the phone (both phone modes).
- [ ] Self-update: publish `v2.0.0-beta.2` later and check a `beta.1` desktop install updates itself.

## Secrets and variables

Keep every signing key in the git-ignored `.secrets/` folder at the repository root ([`.secrets/README.md`](../.secrets/README.md) lists what goes there). Create the keys and upload them as GitHub secrets in one go, in your own terminal:

```sh
scripts/signing-keys.sh all            # or: android | updater | manifest
```

The script prompts for passwords, never overwrites an existing key, prints only public keys, and refuses to run if `.secrets/` isn't git-ignored. Pass `--no-upload` to only create the files. Back the files and passwords up to a password manager straight away.

| Name | Kind | Required | Purpose |
| --- | --- | --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | secret | yes, once the updater is enabled | Signs updater artifacts. Generate with `pnpm tauri signer generate`. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | secret | if the key has one | Password for the key above. |
| `MANIFEST_SIGNING_KEY` | secret | yes | 32-byte hex ed25519 seed that signs `brokers-manifest.json`. |
| `MANIFEST_PUBLIC_KEY` | variable | recommended | Hex public key that the app trusts. When set, CI checks the signature against it, which catches a mismatch between the secret and the app. |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` | secret | optional | Developer ID signing. `APPLE_CERTIFICATE` is the base64 of the `.p12` file. |
| `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), `APPLE_TEAM_ID` | secret | optional | Notarization. Used only together with the certificate. |
| `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` | secret | for an APK | Signs the Android APK. See [Android signing key](#android-signing-key). |

Create them with `scripts/signing-keys.sh apple-csr` (key and certificate request in `.secrets/`), upload the request at developer.apple.com as a **Developer ID Application** certificate (Account Holder only), save the certificate as `.secrets/apple-developer-id.cer`, then run `scripts/signing-keys.sh apple`. It builds the `.p12` with Apple's intermediate certificate and uploads all six secrets.

Set all six Apple secrets or none of them. If `APPLE_CERTIFICATE` is set and any of the others is missing, the workflow fails. With none set, the macOS build is ad-hoc signed (`APPLE_SIGNING_IDENTITY=-`), and users have to allow it once in System Settings → Privacy & Security → Open Anyway (recent macOS no longer offers right-click → Open).

Each secret reaches only the step that uses it. `TAURI_SIGNING_*` and `APPLE_*` go only to the tauri-action build step, `ANDROID_*` only to the APK build step (the keystore is written to the runner's temp folder and deleted afterwards), and `MANIFEST_SIGNING_KEY` goes only to the sign step. Workflows default to `contents: read`, and `contents: write` is granted only to the jobs that create the release or upload to it.

Windows code signing isn't wired up yet. To add it, set `bundle.windows.signCommand` (e.g. Azure Trusted Signing) or `certificateThumbprint` in `tauri.conf.json` and import the certificate in the Windows job. Unsigned installers trigger SmartScreen.

### Action pinning

Every action in `ci-v2.yml` and `release-v2.yml` is pinned to a full 40-character commit SHA, with the version in a trailing comment (`uses: owner/repo@<sha> # v1.2.3`). `.github/dependabot.yml` (github-actions, weekly) opens PRs that bump the SHA and the comment together.

To pin by hand, resolve the tag to its commit. Dereference annotated tags with `gh api repos/<o>/<r>/git/tags/<sha>`:

```sh
gh api repos/<owner>/<repo>/git/ref/tags/<tag> --jq .object
```

`dtolnay/rust-toolchain` is pinned to a commit on its `stable` branch. When it is pinned by SHA, the `toolchain: stable` input is required.

### Manifest signing key

```sh
scripts/signing-keys.sh manifest            # creates .secrets/manifest.key, uploads it, prints the public key
node scripts/sign-manifest.mjs --self-test   # keygen -> sign real brokers dir -> verify -> tamper checks
```

- The script stores the seed as the `MANIFEST_SIGNING_KEY` secret and sets the `MANIFEST_PUBLIC_KEY` variable.
- Add the public key to `TRUSTED_MANIFEST_KEYS` in `app/src-tauri/src/broker/remote.rs`.
- Until the key is in `TRUSTED_MANIFEST_KEYS`, apps reject every remote manifest and run on their bundled copy.
- To rotate the key, ship an app release that trusts both the old and new keys before you switch the secret.

To verify a published bundle locally:

```sh
gh release download v2.0.1 -p 'brokers-manifest.json*' -D /tmp/m
node scripts/sign-manifest.mjs --verify <pubkeyhex> --out /tmp/m [--dir app/src-tauri/brokers]
```

### Android signing key

Every AutoLogin APK must be signed with the same key forever. Android installs an update only when its signature matches the installed app. If the key is lost, users have to uninstall and reinstall, which deletes their saved accounts unless they exported a backup first. Keep a copy of the keystore file and its password in a password manager, not only in `.secrets/`.

Create it once with `scripts/signing-keys.sh android`. It writes `.secrets/android-release.jks` (RSA 4096, alias `autologin`, one password for store and key) and uploads the four `ANDROID_*` secrets.

To build a signed APK locally, set `ANDROID_KEYSTORE_PATH`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD`, then run `pnpm tauri android build --apk` in `app/`. Without them the release APK is unsigned and can't be installed.

Android has no self-updater: users install a new APK from the GitHub release over the old one, and their accounts stay.

## Broker-only fix (no app release)

Installed apps fetch `releases/latest/download/brokers-manifest.json` (and the `.sig` next to it). They use it only if it is signed by a trusted key, passes validation, and has a **higher `manifest_version`** than the copy they already have.

1. Fix the broker file(s) in `app/src-tauri/brokers/` and bump `manifest_version` in `brokers/index.toml`. Commit as `fix(brokers): ...` and merge to the release branch.
2. Go to **Actions → Release (v2) → Run workflow**, choose that branch, tick `manifest_only`, and leave `tag` empty. The workflow then:
   - signs the brokers directory from the branch head;
   - refuses to run unless `manifest_version` is higher than the one on the release;
   - replaces the two manifest assets on the **latest published release**.
3. Apps pick up the fix on their next manifest check. The next full release bundles the fix as well.

## How v1 users are upgraded to v2

v1 (its updater lives in `src/autologin/utils/updater.py` at tag `v1.0.24`; the v1 code was removed from this branch) does the following:

- It calls `GET /repos/pratikbhadane24/autologin/releases/latest`.
- It compares `tag_name` with its own version (`2.x` > `1.0.24`).
- It downloads the **first** asset whose lower-cased name ends in `.msi` (Windows), `.dmg` (macOS) or `.appimage` (Linux, skipping names containing `arm` or `aarch64` on x86).
- It opens that file: the MSI runs, the DMG is mounted for drag-to-Applications, and the folder containing the AppImage is opened.

A published, non-pre-release v2 release is therefore offered to v1 users automatically. Asset names produced by Tauri:

| Platform | v1 picks | Ignored by v1 |
| --- | --- | --- |
| Windows | `AutoLogin_<ver>_x64_en-US.msi` | `*-setup.exe`, `*.msi.sig` |
| macOS | `AutoLogin_<ver>_universal.dmg` (one file for both architectures) | `AutoLogin_universal.app.tar.gz(.sig)` |
| Linux | `AutoLogin_<ver>_amd64.AppImage` | `*.AppImage.sig`, `.deb` |

`brokers-manifest.json(.sig)` and `latest.json` match none of these.

Gotchas:

- **v1 build/release workflows are gone.** The Python app and its Briefcase workflows were removed in v2; only `ci-v2.yml` and `release-v2.yml` run. v1 sources remain at tag `v1.0.24`.
- **Pre-releases never reach v1**, because `releases/latest` skips them. Betas have to be installed manually.
- **Keep exactly one installer per extension.** v1 takes the first match, so an extra `.msi`, `.dmg` or `.AppImage` (for example, a separate arm64 dmg) makes its choice depend on asset order. The `check-assets` job enforces this rule.
- **Windows: v1 is removed automatically.** The real v1.0.24 MSI has UpgradeCode `{A6D3467D-D77D-5786-AA4F-92D15AB50522}` and installs per-user (`ALLUSERS=2`, `MSIINSTALLPERUSER=1`). Tauri's MSI installs per-machine. Windows Installer can't upgrade across scopes, so matching the UpgradeCode would not help. Instead, on every launch v2 looks for a `DisplayName = AutoLogin`, version `1.*`, MSI entry under `HKCU\...\Uninstall` and runs `msiexec /x {ProductCode} /qn /norestart` (`app/src-tauri/src/v1_uninstall.rs`). This happens after v1's data has been migrated. The per-user uninstall needs no admin prompt. If v1 is still running, the removal is retried on the next launch.
- **macOS**: the bundle is named `AutoLogin.app`, the same as v1, so dragging it into Applications replaces v1.
- **Linux**: v1 only opens the download folder. The user has to replace the AppImage by hand.
- **A later v1.x release would become "latest"** and hide v2 from the v2 manifest and updater fetch. Don't publish v1 releases after v2 ships, or mark them as not-latest.
- **Updater artifacts are built only in CI.** The release workflow passes `--config src-tauri/tauri.release.conf.json`, which sets `createUpdaterArtifacts: true`, so `TAURI_SIGNING_PRIVATE_KEY` is required there but not for local builds. The app checks `releases/latest/download/latest.json` and verifies it against the `pubkey` in `tauri.conf.json`.
