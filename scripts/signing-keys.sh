#!/usr/bin/env bash
# Create AutoLogin's release signing keys in .secrets/ and upload them as
# GitHub Actions secrets. Run it yourself, in a terminal; it prompts for the
# passwords and only ever prints public keys. Existing keys are never
# replaced: delete a file on purpose first if you really mean to rotate it.
#
#   scripts/signing-keys.sh android|updater|manifest|all [--no-upload]
#
# See .secrets/README.md for what each key signs and where its public half
# goes, and docs/releasing.md for how the release workflow uses them.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SECRETS="$ROOT/.secrets"
ANDROID_ALIAS="autologin"
MIN_PASSWORD_LENGTH=12
UPLOAD=true

die() { echo "error: $*" >&2; exit 1; }

ask_password() { # ask_password <label> -> stdout
  local first second
  read -r -s -p "$1 (min $MIN_PASSWORD_LENGTH chars): " first; echo >&2
  [ "${#first}" -ge "$MIN_PASSWORD_LENGTH" ] || die "password too short"
  read -r -s -p "Repeat it: " second; echo >&2
  [ "$first" = "$second" ] || die "passwords don't match"
  printf '%s' "$first"
}

upload() { # upload <secret-name>   (value on stdin, never in argv)
  if [ "$UPLOAD" = true ]; then
    gh secret set "$1" --repo "$REPO" >/dev/null
    echo "  uploaded secret $1"
  else
    cat >/dev/null
  fi
}

refuse_existing() {
  [ ! -e "$1" ] || die "$1 already exists; not replacing a signing key (see .secrets/README.md)"
}

android_key() {
  local file="$SECRETS/android-release.jks"
  refuse_existing "$file"
  command -v keytool >/dev/null || die "keytool not found (install a JDK)"
  AUTOLOGIN_KS_PASSWORD="$(ask_password "Android keystore password")"
  export AUTOLOGIN_KS_PASSWORD
  # One password for store and key (PKCS12 keystores use the same one anyway).
  keytool -genkeypair -keystore "$file" -storetype PKCS12 -alias "$ANDROID_ALIAS" \
    -keyalg RSA -keysize 4096 -validity 10000 \
    -dname "CN=AutoLogin, O=Quartgen Solutions Private Limited, C=IN" \
    -storepass:env AUTOLOGIN_KS_PASSWORD -keypass:env AUTOLOGIN_KS_PASSWORD
  chmod 600 "$file"
  echo "created $file"
  base64 < "$file" | tr -d '\n' | upload ANDROID_KEYSTORE_BASE64
  printf '%s' "$AUTOLOGIN_KS_PASSWORD" | upload ANDROID_KEYSTORE_PASSWORD
  printf '%s' "$AUTOLOGIN_KS_PASSWORD" | upload ANDROID_KEY_PASSWORD
  printf '%s' "$ANDROID_ALIAS" | upload ANDROID_KEY_ALIAS
  unset AUTOLOGIN_KS_PASSWORD
}

updater_key() {
  local file="$SECRETS/updater.key" password
  refuse_existing "$file"
  password="$(ask_password "Desktop updater key password")"
  # The Tauri CLI takes the password only as an argument; it's visible to
  # your own user's processes for the second it runs.
  (cd "$ROOT/app" && pnpm -s tauri signer generate --ci -w "$file" -p "$password" >/dev/null)
  chmod 600 "$file"
  echo "created $file"
  upload TAURI_SIGNING_PRIVATE_KEY < "$file"
  printf '%s' "$password" | upload TAURI_SIGNING_PRIVATE_KEY_PASSWORD
  echo "  public key for plugins.updater.pubkey in app/src-tauri/tauri.conf.json:"
  echo "  $(cat "$file.pub")"
}

manifest_key() {
  local file="$SECRETS/manifest.key" public
  refuse_existing "$file"
  public="$(node "$ROOT/scripts/sign-manifest.mjs" --keygen "$file" | sed -n 's/^public key (embed in app): //p')"
  [ -n "$public" ] || die "manifest key generation printed no public key"
  chmod 600 "$file"
  echo "created $file"
  tr -d '\n' < "$file" | upload MANIFEST_SIGNING_KEY
  if [ "$UPLOAD" = true ]; then
    gh variable set MANIFEST_PUBLIC_KEY --repo "$REPO" --body "$public" >/dev/null
    echo "  set variable MANIFEST_PUBLIC_KEY"
  fi
  echo "  public key for TRUSTED_MANIFEST_KEYS in app/src-tauri/src/broker/remote.rs:"
  echo "  $public"
}

main() {
  local what="${1:-}"
  [ "${2:-}" = "--no-upload" ] && UPLOAD=false
  case "$what" in android|updater|manifest|all) ;; *) die "usage: $0 android|updater|manifest|all [--no-upload]";; esac

  umask 077
  mkdir -p "$SECRETS"
  chmod 700 "$SECRETS"
  git -C "$ROOT" check-ignore -q "$SECRETS/probe.key" || die ".secrets/ is not git-ignored; refusing to write keys there"
  if [ "$UPLOAD" = true ]; then
    command -v gh >/dev/null || die "gh not found (or pass --no-upload)"
    REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
    echo "Uploading secrets to $REPO"
  fi

  case "$what" in
    android) android_key ;;
    updater) updater_key ;;
    manifest) manifest_key ;;
    all) android_key; updater_key; manifest_key ;;
  esac
  echo "Back up the new files in .secrets/ (and their passwords) to your password manager now."
}

main "$@"
