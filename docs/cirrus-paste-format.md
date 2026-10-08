# "Copy for AutoLogin" format (Cirrus dashboard → AutoLogin)

The Cirrus dashboard puts JSON on the clipboard. The user pastes it into
AutoLogin with **Add → Paste from Cirrus**. The paste fills in the account
details. The user then enters their password, PIN and TOTP secret in AutoLogin.
Those never come from Cirrus.

## Payload (format version 1)

The Cirrus backend builds this (`POST /api/accounts/autologin-export` in
broker-auth-backend) and signs it; AutoLogin rejects unsigned, edited or
expired pastes.

```json
{
  "issued_to": "cirrus_username",
  "iat": 1791270000,
  "exp": 1791270900,
  "accounts": [
    { "autologin": 1, "tenant": "cirrus", "broker": "zerodha", "client_id": "AB1234", "api_key": "kite_api_key" }
  ]
}
```

| Key | Notes |
|---|---|
| `issued_to` | Cirrus user the copy was made for. AutoLogin shows it before adding accounts, so a copy someone else made is obvious. |
| `iat` / `exp` | Unix seconds. Copies expire 15 minutes after `iat`; AutoLogin allows 5 minutes of clock difference. |
| `accounts[].tenant` | Tenant id: `cirrus` or `pocketful` (backend `APP_TENANT`). Must match an id in AutoLogin's signed broker manifest. Never URLs. |
| `accounts[].broker` | AutoLogin broker id (`zerodha`, `upstox`, `pocketful`, `motilal`, `fyers`, `tradejini`, `aliceblue`, …). |
| `accounts[].client_id` | Broker client / user id. |
| other account fields | Only those listed below are kept; anything else is dropped and only its *name* is reported. |

### Fields AutoLogin accepts per broker

| Broker | Fields accepted from Cirrus |
|---|---|
| Pocketful | `client_id` |
| Zerodha | `client_id`, `api_key` |
| Upstox | `client_id`, `api_key` |
| Motilal Oswal | `client_id`, `api_key` |
| Fyers | `client_id` (Fyers ID), `api_key` (the user's Fyers App ID, e.g. `XB12345-100`) |
| Tradejini | `client_id` (User ID), `api_key` (the user's Tradejini API key) |
| AliceBlue | `client_id` (User ID), `api_key` (the App Code of the user's AliceBlue API app) |

**Do not include** `api_secret`, passwords, PINs, TOTP secrets or access
tokens. AutoLogin discards them even if present, but they should never reach
the clipboard.

## The same account in two tenants

AutoLogin identifies an account by **tenant + broker + client_id**. The same
Zerodha client can therefore be added once for `cirrus` and once for
`pocketful`. Each entry sends its login to its own tenant.

## Signed envelope (required)

The clipboard holds this envelope, not the raw payload (implemented in
broker-auth-backend `app/services/autologin_export.py`):

```json
{ "autologin": 1, "kid": "cirrus-2026-1", "payload": "<base64url(payload JSON), no padding>", "sig": "<base64url(ed25519 signature over the payload STRING bytes), no padding>" }
```

- The signature covers the **base64url payload string** exactly as it appears
  in the envelope. It does not cover the decoded JSON, so JSON key order and
  whitespace don't matter.
- `kid` names the key. AutoLogin ships the public key for each `kid` in
  `TRUSTED_PASTE_KEYS` (`app/src-tauri/src/paste.rs`). New keys arrive with an
  app release, so start a new key well before the old one is retired.
- Sign on the **backend**. The private key must never reach the browser.

### Backend signing (Python)

```python
import base64, json, time
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()

def autologin_copy(accounts: list[dict], username: str, key: Ed25519PrivateKey, kid: str) -> str:
    now = int(time.time())
    body = {"issued_to": username, "iat": now, "exp": now + 15 * 60, "accounts": accounts}
    payload = b64url(json.dumps(body, separators=(",", ":")).encode())
    sig = b64url(key.sign(payload.encode()))
    return json.dumps({"autologin": 1, "kid": kid, "payload": payload, "sig": sig})
```

### One-time key setup

```python
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization
key = Ed25519PrivateKey.generate()
print(key.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw,
      serialization.NoEncryption()).hex())   # -> backend secret store only
print(key.public_key().public_bytes(serialization.Encoding.Raw,
      serialization.PublicFormat.Raw).hex())  # -> send to AutoLogin (public)
```

Encrypting the paste would add nothing. AutoLogin is open source, so any key
inside it is public, and the paste contains no secrets. The signature is what
proves the data came from Cirrus.
