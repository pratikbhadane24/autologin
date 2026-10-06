//! "Copy for AutoLogin" paste from the Cirrus dashboard.
//! Format and signing are specified in `docs/cirrus-paste-format.md`.
//!
//! The clipboard holds a signed envelope `{autologin, kid, payload, sig}`;
//! `payload` is base64url JSON with one account object or an array of them.
//! Only pastes signed by a key in `TRUSTED_PASTE_KEYS` are accepted.
//!
//! Only fields marked `from_cirrus` in the broker manifest are kept. Anything
//! else, including any secret that slipped into the copy, is dropped and only
//! its *name* is reported back. `tenant` must be a tenant id from the broker
//! manifest, so a paste can never point logins at an unlisted server. Users
//! add passwords, PINs and TOTP secrets in AutoLogin itself.

use std::collections::BTreeMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::broker::manifest::Availability;
use crate::broker::registry::ManifestBundle;

pub const FORMAT_VERSION: u64 = 1;

/// Cirrus paste-signing public keys as `(kid, hex-encoded ed25519 key)`.
/// The private keys live only on the Cirrus backend. Empty until Cirrus
/// generates its first key; until then every paste is rejected.
pub const TRUSTED_PASTE_KEYS: &[(&str, &str)] = &[];
const MAX_PASTE_BYTES: usize = 256 * 1024;
const MAX_ACCOUNTS: usize = 500;
const MAX_VALUE_CHARS: usize = 256;
/// Keys that describe the paste itself, not an account field.
const ENVELOPE_KEYS: &[&str] = &["autologin", "broker", "tenant"];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PasteError {
    #[error("Clipboard is empty.")]
    Empty,
    #[error("Clipboard text is too large to be a Cirrus account copy.")]
    TooLarge,
    #[error("This doesn't look like a Cirrus account copy (expected JSON).")]
    NotJson,
    #[error("This copy is from a newer Cirrus format ({0}); update AutoLogin.")]
    UnsupportedVersion(u64),
    #[error("Too many accounts in one paste (max {MAX_ACCOUNTS}).")]
    TooManyAccounts,
    #[error("This copy isn't signed by Cirrus. Use the \"Copy for AutoLogin\" button in Cirrus.")]
    Unsigned,
    #[error("This copy was signed with an unknown key; update AutoLogin.")]
    UnknownKey,
    #[error("This copy was changed after Cirrus created it, so it can't be trusted.")]
    BadSignature,
    #[error("This copy has expired. Copy the accounts from Cirrus again.")]
    Expired,
}

#[derive(Deserialize)]
struct Envelope {
    autologin: u64,
    kid: String,
    payload: String,
    sig: String,
}

/// One account parsed from the paste, ready to prefill the Add form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PastedAccount {
    pub tenant_id: String,
    pub broker_id: String,
    pub fields: BTreeMap<String, String>,
    /// Names of keys that were present but not accepted.
    pub ignored: Vec<String>,
    pub coming_soon: bool,
}

/// An entry that could not be used, with a user-facing reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PasteProblem {
    pub position: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PasteResult {
    pub accounts: Vec<PastedAccount>,
    pub problems: Vec<PasteProblem>,
    /// Cirrus username the copy was made for (signed copies only). Shown
    /// before adding, so a copy someone else made is obvious.
    pub issued_to: Option<String>,
}

/// Signed payload: who the copy is for, when it was made and when it expires.
#[derive(Deserialize)]
struct SignedPayload {
    issued_to: String,
    iat: u64,
    exp: u64,
    accounts: Vec<Value>,
}

/// Tolerated clock difference between Cirrus and this computer.
const CLOCK_SKEW_SECONDS: u64 = 5 * 60;

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Development builds also accept an unsigned paste (the plain payload JSON)
/// so the flow can be tested before Cirrus signs copies. Release builds never do.
pub const ALLOW_UNSIGNED: bool = cfg!(debug_assertions);

pub fn parse(text: &str, bundle: &ManifestBundle) -> Result<PasteResult, PasteError> {
    parse_with(text, bundle, TRUSTED_PASTE_KEYS, ALLOW_UNSIGNED)
}

/// Strict parse: only correctly signed pastes.
pub fn parse_with_keys(
    text: &str,
    bundle: &ManifestBundle,
    trusted: &[(&str, &str)],
) -> Result<PasteResult, PasteError> {
    parse_with(text, bundle, trusted, false)
}

fn parse_with(
    text: &str,
    bundle: &ManifestBundle,
    trusted: &[(&str, &str)],
    allow_unsigned: bool,
) -> Result<PasteResult, PasteError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(PasteError::Empty);
    }
    if text.len() > MAX_PASTE_BYTES {
        return Err(PasteError::TooLarge);
    }
    let outer: Value = serde_json::from_str(text).map_err(|_| PasteError::NotJson)?;
    let is_envelope = outer.get("payload").is_some();
    let (entries, issued_to) = match (is_envelope, outer) {
        (true, outer) => {
            let envelope: Envelope = serde_json::from_value(outer).map_err(|_| PasteError::Unsigned)?;
            if envelope.autologin > FORMAT_VERSION {
                return Err(PasteError::UnsupportedVersion(envelope.autologin));
            }
            let payload = verify(&envelope, trusted)?;
            let signed: SignedPayload = serde_json::from_slice(&payload).map_err(|_| PasteError::NotJson)?;
            let now = now_seconds();
            if signed.exp + CLOCK_SKEW_SECONDS < now || signed.iat > now + CLOCK_SKEW_SECONDS {
                return Err(PasteError::Expired);
            }
            (signed.accounts, Some(signed.issued_to))
        }
        (false, plain @ (Value::Object(_) | Value::Array(_))) if allow_unsigned => {
            tracing::warn!("accepting an unsigned paste (development build)");
            let entries = match plain {
                Value::Array(items) => items,
                single => vec![single],
            };
            (entries, None)
        }
        (false, Value::Object(_) | Value::Array(_)) => return Err(PasteError::Unsigned),
        (false, _) => return Err(PasteError::NotJson),
    };
    if entries.len() > MAX_ACCOUNTS {
        return Err(PasteError::TooManyAccounts);
    }

    let mut result = PasteResult { issued_to, ..PasteResult::default() };
    for (index, entry) in entries.iter().enumerate() {
        let position = index + 1;
        match parse_entry(entry, bundle)? {
            Ok(account) => result.accounts.push(account),
            Err(reason) => result.problems.push(PasteProblem { position, reason }),
        }
    }
    Ok(result)
}

/// Check the signature over the raw payload bytes and return them decoded.
fn verify(envelope: &Envelope, trusted: &[(&str, &str)]) -> Result<Vec<u8>, PasteError> {
    let key_hex = trusted.iter().find(|(kid, _)| *kid == envelope.kid).map(|(_, key)| *key).ok_or(PasteError::UnknownKey)?;
    let key_bytes: [u8; 32] = hex::decode(key_hex)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(PasteError::UnknownKey)?;
    let key = VerifyingKey::from_bytes(&key_bytes).map_err(|_| PasteError::UnknownKey)?;
    let sig_bytes: [u8; 64] = URL_SAFE_NO_PAD
        .decode(envelope.sig.trim_end_matches('='))
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(PasteError::BadSignature)?;
    key.verify_strict(envelope.payload.as_bytes(), &Signature::from_bytes(&sig_bytes))
        .map_err(|_| PasteError::BadSignature)?;
    URL_SAFE_NO_PAD.decode(envelope.payload.trim_end_matches('=')).map_err(|_| PasteError::NotJson)
}

/// Outer `Err` aborts the whole paste; inner `Err` skips just this entry.
fn parse_entry(entry: &Value, bundle: &ManifestBundle) -> Result<Result<PastedAccount, String>, PasteError> {
    let Some(object) = entry.as_object() else {
        return Ok(Err("not an account object".to_string()));
    };
    if let Some(version) = object.get("autologin").and_then(Value::as_u64) {
        if version > FORMAT_VERSION {
            return Err(PasteError::UnsupportedVersion(version));
        }
    }
    let tenant_id = match object.get("tenant").and_then(Value::as_str) {
        Some(id) if bundle.has_tenant(id) => id.to_string(),
        Some(id) => return Ok(Err(format!("unknown Cirrus workspace \"{id}\""))),
        None => bundle.default_tenant().to_string(),
    };
    let Some(broker_name) = object.get("broker").and_then(Value::as_str) else {
        return Ok(Err("missing \"broker\"".to_string()));
    };
    let Some(manifest) = bundle.resolve_alias(broker_name) else {
        return Ok(Err(format!("AutoLogin doesn't support \"{broker_name}\" yet")));
    };

    let mut fields = BTreeMap::new();
    let mut ignored = Vec::new();
    for (key, value) in object {
        if ENVELOPE_KEYS.contains(&key.as_str()) {
            continue;
        }
        let accepted = manifest.field(key).filter(|f| f.from_cirrus);
        match (accepted, scalar_text(value)) {
            (Some(_), Some(text)) if !text.is_empty() && text.chars().count() <= MAX_VALUE_CHARS => {
                fields.insert(key.clone(), text);
            }
            _ => ignored.push(key.clone()),
        }
    }
    if !fields.contains_key("client_id") {
        return Ok(Err(format!("{}: missing client_id", manifest.name)));
    }
    Ok(Ok(PastedAccount {
        tenant_id,
        broker_id: manifest.id.clone(),
        fields,
        ignored,
        coming_soon: manifest.availability == Availability::ComingSoon,
    }))
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const KID: &str = "test-1";

    fn bundle() -> ManifestBundle {
        ManifestBundle::bundled().unwrap()
    }

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn trusted() -> Vec<(&'static str, &'static str)> {
        let public = hex::encode(signing_key().verifying_key().to_bytes());
        vec![(KID, Box::leak(public.into_boxed_str()))]
    }

    /// Sign a payload object exactly as broker-auth-backend does.
    fn sign_payload(payload: &Value) -> String {
        let payload = URL_SAFE_NO_PAD.encode(payload.to_string());
        let sig = URL_SAFE_NO_PAD.encode(signing_key().sign(payload.as_bytes()).to_bytes());
        serde_json::json!({ "autologin": 1, "kid": KID, "payload": payload, "sig": sig }).to_string()
    }

    /// Wrap account JSON (one object or a list) the way the Cirrus backend will.
    fn signed(json: &str) -> String {
        let accounts = match serde_json::from_str::<Value>(json).unwrap() {
            Value::Array(items) => items,
            single => vec![single],
        };
        let now = now_seconds();
        sign_payload(&serde_json::json!({
            "issued_to": "test_user", "iat": now, "exp": now + 900, "accounts": accounts
        }))
    }

    fn parse_signed(json: &str) -> Result<PasteResult, PasteError> {
        parse_with_keys(&signed(json), &bundle(), &trusted())
    }

    #[test]
    fn keeps_only_fields_allowed_from_cirrus() {
        let paste = r#"{"autologin":1,"broker":"Zerodha","client_id":" AB1234 ","api_key":"kite123",
                        "api_secret":"SHOULD-NOT-BE-KEPT","password":"nope"}"#;
        let result = parse_signed(paste).unwrap();
        let account = &result.accounts[0];
        assert_eq!(account.broker_id, "zerodha");
        assert_eq!(
            account.fields,
            BTreeMap::from([("api_key".into(), "kite123".into()), ("client_id".into(), "AB1234".into())])
        );
        assert_eq!(account.ignored, vec!["api_secret".to_string(), "password".to_string()]);
    }

    #[test]
    fn ignored_report_never_contains_values() {
        let paste = r#"{"broker":"upstox","client_id":"U1","api_secret":"TOPSECRET"}"#;
        let result = parse_signed(paste).unwrap();
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("TOPSECRET"), "{json}");
    }

    #[test]
    fn accepts_array_and_reports_bad_entries_separately() {
        let paste = r#"[{"broker":"pocketful","client_id":"P1"},
                        {"broker":"unknownbroker","client_id":"X"},
                        {"broker":"upstox"},
                        "oops"]"#;
        let result = parse_signed(paste).unwrap();
        assert_eq!(result.accounts.len(), 1);
        let positions: Vec<usize> = result.problems.iter().map(|p| p.position).collect();
        assert_eq!(positions, vec![2, 3, 4]);
        assert!(result.problems[0].reason.contains("unknownbroker"));
    }

    #[test]
    fn tenant_defaults_to_cirrus_and_must_be_known() {
        let result = parse_signed(
            r#"[{"broker":"zerodha","client_id":"A"},
                {"tenant":"pocketful","broker":"zerodha","client_id":"A"},
                {"tenant":"evil","broker":"zerodha","client_id":"A"}]"#,
        )
        .unwrap();
        let tenants: Vec<&str> = result.accounts.iter().map(|a| a.tenant_id.as_str()).collect();
        assert_eq!(tenants, vec!["cirrus", "pocketful"]);
        assert!(result.problems[0].reason.contains("unknown Cirrus workspace"));
    }

    #[test]
    fn flags_coming_soon_brokers() {
        let result = parse_signed(r#"{"broker":"fyers","client_id":"XA1"}"#).unwrap();
        assert!(result.accounts[0].coming_soon);
    }

    #[test]
    fn rejects_non_json_newer_version_and_oversized_input() {
        assert_eq!(parse("  ", &bundle()), Err(PasteError::Empty));
        assert_eq!(parse("hello", &bundle()), Err(PasteError::NotJson));
        assert_eq!(parse("42", &bundle()), Err(PasteError::NotJson));
        assert_eq!(parse(&"x".repeat(MAX_PASTE_BYTES + 1), &bundle()), Err(PasteError::TooLarge));
        let newer = signed("{}").replace("\"autologin\":1", "\"autologin\":2");
        assert_eq!(parse_with_keys(&newer, &bundle(), &trusted()), Err(PasteError::UnsupportedVersion(2)));
    }

    /// Signed by Python exactly as broker-auth-backend's autologin_export.py
    /// does, with the same test key (`[7u8; 32]`): the formats interoperate.
    const PYTHON_SIGNED: &str = r#"{"autologin":1,"kid":"test-1","payload":"eyJpc3N1ZWRfdG8iOiJ0ZXN0X3VzZXIiLCJpYXQiOjE3MDAwMDAwMDAsImV4cCI6NDEwMjQ0NDgwMCwiYWNjb3VudHMiOlt7ImF1dG9sb2dpbiI6MSwidGVuYW50IjoiY2lycnVzIiwiYnJva2VyIjoiemVyb2RoYSIsImNsaWVudF9pZCI6IkFCMTIzNCIsImFwaV9rZXkiOiJraXRlMTIzIiwiYXBpX3NlY3JldCI6IngifV19","sig":"QRrYrKScAWNizIq1Y3oJcNLne8OM-bTJyXqu-JFF8b1tlikRRy6M_-tDkM2v0E1xqaXPiSJdxHv1x_1bW_qyCA"}"#;

    #[test]
    fn accepts_paste_signed_by_python_backend_recipe() {
        let result = parse_with_keys(PYTHON_SIGNED, &bundle(), &trusted()).unwrap();
        let account = &result.accounts[0];
        assert_eq!(account.fields["client_id"], "AB1234");
        assert_eq!(account.fields["api_key"], "kite123");
        assert_eq!(account.ignored, vec!["api_secret".to_string()]);
    }

    #[test]
    fn development_builds_may_accept_unsigned_but_release_logic_does_not() {
        let plain = r#"{"autologin":1,"tenant":"cirrus","broker":"pocketful","client_id":"PK1"}"#;
        let accepted = parse_with(plain, &bundle(), &[], true).unwrap();
        assert_eq!(accepted.accounts[0].fields["client_id"], "PK1");
        assert_eq!(parse_with(plain, &bundle(), &[], false), Err(PasteError::Unsigned));
        // A forged "signed" envelope is still verified even when unsigned is allowed.
        let forged = r#"{"autologin":1,"kid":"x","payload":"e30","sig":"AAAA"}"#;
        assert_eq!(parse_with(forged, &bundle(), &trusted(), true), Err(PasteError::UnknownKey));
    }

    #[test]
    fn release_builds_reject_unsigned() {
        // Guards the cfg: this constant must be false whenever tests run in release mode.
        assert_eq!(ALLOW_UNSIGNED, cfg!(debug_assertions));
    }

    #[test]
    fn shows_who_a_signed_copy_is_from() {
        let result = parse_signed(r#"{"broker":"pocketful","client_id":"P1"}"#).unwrap();
        assert_eq!(result.issued_to.as_deref(), Some("test_user"));
    }

    #[test]
    fn rejects_expired_and_future_dated_copies() {
        let now = now_seconds();
        let account = serde_json::json!([{"broker": "pocketful", "client_id": "P1"}]);
        let expired = sign_payload(&serde_json::json!({
            "issued_to": "u", "iat": now - 7_200, "exp": now - 3_600, "accounts": account
        }));
        assert_eq!(parse_with_keys(&expired, &bundle(), &trusted()), Err(PasteError::Expired));
        let future = sign_payload(&serde_json::json!({
            "issued_to": "u", "iat": now + 7_200, "exp": now + 8_100, "accounts": account
        }));
        assert_eq!(parse_with_keys(&future, &bundle(), &trusted()), Err(PasteError::Expired));
    }

    #[test]
    fn signed_copy_without_expiry_fields_is_rejected() {
        // The old list-only payload is no longer accepted once signed.
        let legacy = sign_payload(&serde_json::json!([{"broker": "pocketful", "client_id": "P1"}]));
        assert_eq!(parse_with_keys(&legacy, &bundle(), &trusted()), Err(PasteError::NotJson));
    }

    #[test]
    fn rejects_unsigned_paste() {
        let plain = r#"{"autologin":1,"tenant":"cirrus","broker":"zerodha","client_id":"AB1234"}"#;
        assert_eq!(parse_with_keys(plain, &bundle(), &trusted()), Err(PasteError::Unsigned));
    }

    #[test]
    fn rejects_edited_payload() {
        let original = signed(r#"{"tenant":"cirrus","broker":"zerodha","client_id":"AB1234"}"#);
        let mut envelope: Value = serde_json::from_str(&original).unwrap();
        let forged = URL_SAFE_NO_PAD.encode(r#"{"tenant":"pocketful","broker":"zerodha","client_id":"AB1234"}"#);
        envelope["payload"] = Value::String(forged);
        let result = parse_with_keys(&envelope.to_string(), &bundle(), &trusted());
        assert_eq!(result, Err(PasteError::BadSignature));
    }

    #[test]
    fn rejects_unknown_key_and_default_build_trusts_nothing_yet() {
        let paste = signed(r#"{"broker":"zerodha","client_id":"A"}"#);
        assert_eq!(parse_with_keys(&paste, &bundle(), &[]), Err(PasteError::UnknownKey));
        assert_eq!(parse(&paste, &bundle()), Err(PasteError::UnknownKey));
    }

    #[test]
    fn numeric_client_ids_are_accepted() {
        let result = parse_signed(r#"{"broker":"motilal","client_id":12345}"#).unwrap();
        assert_eq!(result.accounts[0].fields["client_id"], "12345");
    }
}
