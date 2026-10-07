#!/usr/bin/env bash
# Create AutoLogin's release signing keys in .secrets/ and upload them as
# GitHub Actions secrets. Run it yourself, in a terminal; it prompts for the
# passwords and only ever prints public keys. Existing keys are never
# replaced: delete a file on purpose first if you really mean to rotate it.
#
#   scripts/signing-keys.sh android|updater|manifest|all [--no-upload]
#   scripts/signing-keys.sh paste      Cirrus paste key: never uploaded to
#                                      GitHub; it goes into the Cirrus backend
#   scripts/signing-keys.sh apple-csr  macOS step 1: private key + certificate
#                                      request to upload to Apple
#   scripts/signing-keys.sh apple      macOS step 2: after saving Apple's
#                                      certificate as .secrets/apple-developer-id.cer
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

paste_key() {
  local file="$SECRETS/paste.key" public kid
  refuse_existing "$file"
  kid="cirrus-$(date +%Y%m)"
  public="$(node "$ROOT/scripts/sign-manifest.mjs" --keygen "$file" | sed -n 's/^public key (embed in app): //p')"
  [ -n "$public" ] || die "paste key generation printed no public key"
  chmod 600 "$file"
  echo "created $file"
  echo "  On the broker-auth backend (Dokploy env, not GitHub):"
  echo "    AUTOLOGIN_SIGNING_KEY=<the 64 hex characters in $file>"
  echo "    AUTOLOGIN_SIGNING_KEY_ID=$kid"
  echo "  For TRUSTED_PASTE_KEYS in app/src-tauri/src/paste.rs:"
  echo "    (\"$kid\", \"$public\")"
}

APPLE_KEY="apple-developer-id.key"
APPLE_CSR="apple-developer-id.csr"
APPLE_CER="apple-developer-id.cer"
APPLE_P12="apple-developer-id.p12"
# Apple's intermediate for Developer ID certificates; codesign needs the chain.
APPLE_INTERMEDIATE_URL="https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer"

apple_csr() {
  local key="$SECRETS/$APPLE_KEY" csr="$SECRETS/$APPLE_CSR" email
  refuse_existing "$key"
  command -v openssl >/dev/null || die "openssl not found"
  read -r -p "Apple Developer account email: " email
  [ -n "$email" ] || die "email is required"
  openssl req -new -newkey rsa:2048 -nodes -keyout "$key" -out "$csr" \
    -subj "/emailAddress=$email/CN=Quartgen Solutions Private Limited/C=IN" 2>/dev/null
  chmod 600 "$key" "$csr"
  echo "created $key (private, stays here) and $csr"
  cat <<STEPS

Next, as the Apple Developer team's Account Holder:
  1. developer.apple.com -> Certificates, IDs & Profiles -> Certificates -> +
  2. Choose "Developer ID Application" (G2 Sub-CA), upload $csr
  3. Download the certificate and save it as .secrets/$APPLE_CER
  4. Create an app-specific password at account.apple.com -> Sign-In and Security
  5. Run: scripts/signing-keys.sh apple
STEPS
}

apple_cert() {
  local key="$SECRETS/$APPLE_KEY" cer="$SECRETS/$APPLE_CER" p12="$SECRETS/$APPLE_P12"
  local pem chain subject identity team password apple_id app_password
  [ -f "$key" ] || die "run 'scripts/signing-keys.sh apple-csr' first"
  # Accept the file under the name Apple downloads it as.
  [ -f "$cer" ] || [ ! -f "$SECRETS/developerID_application.cer" ] || cer="$SECRETS/developerID_application.cer"
  [ -f "$cer" ] || die "save Apple's certificate as .secrets/$APPLE_CER first"
  refuse_existing "$p12"
  # Scratch copies of public certificates only (nothing secret).
  pem="$(mktemp)"; chain="$(mktemp)"
  # Apple hands out DER; accept PEM too.
  openssl x509 -inform der -in "$cer" -out "$pem" 2>/dev/null || openssl x509 -in "$cer" -out "$pem"
  openssl x509 -noout -modulus -in "$pem" | cmp -s - <(openssl rsa -noout -modulus -in "$key" 2>/dev/null) \
    || die "the certificate doesn't match .secrets/$APPLE_KEY (was it made from this request?)"
  subject="$(openssl x509 -noout -subject -nameopt multiline -in "$pem")"
  identity="$(printf '%s\n' "$subject" | sed -n 's/^ *commonName *= *//p')"
  team="$(printf '%s\n' "$subject" | sed -n 's/^ *organizationalUnitName *= *//p')"
  case "$identity" in "Developer ID Application:"*) ;; *) die "not a Developer ID Application certificate: $identity";; esac
  [ -n "$team" ] || die "no Team ID in the certificate"
  if ! curl -fsSL "$APPLE_INTERMEDIATE_URL" | openssl x509 -inform der -out "$chain" 2>/dev/null; then
    die "couldn't download Apple's intermediate certificate"
  fi
  password="$(ask_password "Password for the .p12 bundle")"
  # SHA1/3DES: the encryption macOS's 'security import' (used in CI) accepts.
  AUTOLOGIN_P12_PASSWORD="$password" openssl pkcs12 -export -inkey "$key" -in "$pem" -certfile "$chain" \
    -name "$identity" -out "$p12" -passout env:AUTOLOGIN_P12_PASSWORD \
    -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1
  chmod 600 "$p12"
  rm -f "$pem" "$chain"
  echo "created $p12 for: $identity"
  read -r -p "Apple ID (developer account email): " apple_id
  read -r -s -p "App-specific password (from account.apple.com): " app_password; echo >&2
  [ -n "$apple_id" ] && [ -n "$app_password" ] || die "Apple ID and app-specific password are required"
  base64 < "$p12" | tr -d '\n' | upload APPLE_CERTIFICATE
  printf '%s' "$password" | upload APPLE_CERTIFICATE_PASSWORD
  printf '%s' "$identity" | upload APPLE_SIGNING_IDENTITY
  printf '%s' "$team" | upload APPLE_TEAM_ID
  printf '%s' "$apple_id" | upload APPLE_ID
  printf '%s' "$app_password" | upload APPLE_PASSWORD
  echo "  macOS builds will now be signed and notarized as: $identity"
}

main() {
  local what="${1:-}"
  [ "${2:-}" = "--no-upload" ] && UPLOAD=false
  case "$what" in android|updater|manifest|paste|apple-csr|apple|all) ;;
    *) die "usage: $0 android|updater|manifest|paste|apple-csr|apple|all [--no-upload]";; esac
  case "$what" in paste|apple-csr) UPLOAD=false ;; esac # nothing for GitHub yet

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
    paste) paste_key ;;
    apple-csr) apple_csr ;;
    apple) apple_cert ;;
    all) android_key; updater_key; manifest_key ;;
  esac
  echo "Back up the new files in .secrets/ (and their passwords) to your password manager now."
}

main "$@"
