//! Signed broker-manifest updates published with each GitHub release.
//!
//! Release CI publishes two assets:
//! - `brokers-manifest.json`: `{"files": {"index.toml": "...", "zerodha.toml": "...", ...}}`
//! - `brokers-manifest.json.sig`: base64url (no padding) ed25519 signature over
//!   the exact bytes of `brokers-manifest.json`.
//!
//! A download is used only if it is signed by a key in `TRUSTED_MANIFEST_KEYS`
//! and the files pass the same validation as the bundled copy
//! (`ManifestBundle::from_files`). Verified downloads are cached verbatim, and
//! the cache is re-verified on every load, so a tampered cache is ignored.
//! The app runs with whichever valid bundle has the highest `manifest_version`.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use thiserror::Error;

use super::registry::{ManifestBundle, RegistryError};

/// Hex-encoded ed25519 public keys allowed to sign broker manifests.
/// The private key lives only in the release CI secrets (and the maintainer's
/// .secrets/manifest.key). To rotate, add the new key here in a release
/// before switching the secret (docs/releasing.md).
pub const TRUSTED_MANIFEST_KEYS: &[&str] = &[
    // Created 2026-10-07.
    "8c67f0d320a5ba624de7095c650d44868774085c95c06a83f655265c48034cad",
];

/// Latest release's manifest. The signature is at the same URL plus `.sig`.
pub const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/pratikbhadane24/autologin/releases/latest/download/brokers-manifest.json";

pub const MANIFEST_FILE: &str = "brokers-manifest.json";
pub const SIGNATURE_FILE: &str = "brokers-manifest.json.sig";
const SIGNATURE_SUFFIX: &str = ".sig";

const MAX_MANIFEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_SIGNATURE_BYTES: usize = 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Error)]
pub enum RemoteError {
    #[error("Broker update is larger than allowed ({limit} bytes).")]
    TooLarge { limit: usize },
    #[error("No trusted signing key is configured for broker updates.")]
    NoTrustedKeys,
    #[error("Broker update signature is malformed.")]
    MalformedSignature,
    #[error("Broker update is not signed by a trusted key.")]
    BadSignature,
    #[error("Broker update is not in the expected format.")]
    Malformed,
    #[error("Broker update failed validation: {0}")]
    Invalid(#[from] RegistryError),
    #[error("Could not download broker update: {0}")]
    Network(String),
    #[error("Broker update server returned HTTP {0}.")]
    Status(u16),
    #[error("Could not save broker update: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Deserialize)]
struct ManifestFiles {
    files: BTreeMap<String, String>,
}

/// Verify `sig_b64` over `json_bytes` against any trusted key, then parse and
/// validate the bundle.
pub fn verify_and_parse(json_bytes: &[u8], sig_b64: &str, trusted_keys: &[&str]) -> Result<ManifestBundle, RemoteError> {
    if json_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(RemoteError::TooLarge { limit: MAX_MANIFEST_BYTES });
    }
    verify_signature(json_bytes, sig_b64, trusted_keys)?;
    let parsed: ManifestFiles = serde_json::from_slice(json_bytes).map_err(|_| RemoteError::Malformed)?;
    let bundle = ManifestBundle::from_files(parsed.files.iter().map(|(name, text)| (name.as_str(), text.as_str())))?;
    Ok(bundle)
}

fn verify_signature(message: &[u8], sig_b64: &str, trusted_keys: &[&str]) -> Result<(), RemoteError> {
    let keys: Vec<VerifyingKey> = trusted_keys.iter().filter_map(|hex_key| parse_key(hex_key)).collect();
    if keys.is_empty() {
        return Err(RemoteError::NoTrustedKeys);
    }
    let sig_bytes: [u8; 64] = URL_SAFE_NO_PAD
        .decode(sig_b64.trim().trim_end_matches('='))
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(RemoteError::MalformedSignature)?;
    let signature = Signature::from_bytes(&sig_bytes);
    if keys.iter().any(|key| key.verify_strict(message, &signature).is_ok()) {
        Ok(())
    } else {
        Err(RemoteError::BadSignature)
    }
}

fn parse_key(hex_key: &str) -> Option<VerifyingKey> {
    let bytes: Option<[u8; 32]> = hex::decode(hex_key).ok().and_then(|b| b.try_into().ok());
    let key = bytes.and_then(|b| VerifyingKey::from_bytes(&b).ok());
    if key.is_none() {
        tracing::warn!(key = hex_key, "ignoring malformed trusted manifest key");
    }
    key
}

/// The verified bundle cached in `cache_dir`, if any. Missing, tampered or
/// invalid caches yield `None` (logged), so callers fall back to the bundled copy.
pub fn load_cached(cache_dir: &Path, trusted_keys: &[&str]) -> Option<ManifestBundle> {
    let json = match std::fs::read(cache_dir.join(MANIFEST_FILE)) {
        Ok(json) => json,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(%error, "could not read cached broker manifest");
            return None;
        }
    };
    let sig = match std::fs::read_to_string(cache_dir.join(SIGNATURE_FILE)) {
        Ok(sig) => sig,
        Err(error) => {
            tracing::warn!(%error, "could not read cached broker manifest signature");
            return None;
        }
    };
    match verify_and_parse(&json, &sig, trusted_keys) {
        Ok(bundle) => Some(bundle),
        Err(error) => {
            tracing::warn!(%error, "ignoring cached broker manifest");
            None
        }
    }
}

/// Write both files via temp file + rename so a crash never leaves a partial
/// file. A crash between the two renames leaves a mismatched pair, which fails
/// verification on load and is ignored.
fn save_cache(cache_dir: &Path, json_bytes: &[u8], sig_b64: &str) -> Result<(), RemoteError> {
    std::fs::create_dir_all(cache_dir)?;
    write_atomic(cache_dir, MANIFEST_FILE, json_bytes)?;
    write_atomic(cache_dir, SIGNATURE_FILE, sig_b64.as_bytes())?;
    Ok(())
}

fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = dir.join(format!("{name}.tmp"));
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, dir.join(name))
}

/// The newest bundle by `manifest_version`; ties keep the bundled copy.
pub fn choose(bundled: ManifestBundle, candidates: impl IntoIterator<Item = ManifestBundle>) -> ManifestBundle {
    candidates.into_iter().fold(bundled, |best, candidate| {
        if candidate.index.manifest_version > best.index.manifest_version {
            candidate
        } else {
            best
        }
    })
}

/// Download and verify the manifest at `url` (signature at `url` + `.sig`).
/// Returns the bundle plus the raw bytes and signature for caching.
pub async fn fetch(
    client: &reqwest::Client,
    url: &str,
    trusted_keys: &[&str],
) -> Result<(ManifestBundle, Vec<u8>, String), RemoteError> {
    let json = download(client, url, MAX_MANIFEST_BYTES).await?;
    let sig_bytes = download(client, &format!("{url}{SIGNATURE_SUFFIX}"), MAX_SIGNATURE_BYTES).await?;
    let sig = String::from_utf8(sig_bytes).map_err(|_| RemoteError::MalformedSignature)?;
    let sig = sig.trim().to_string();
    let bundle = verify_and_parse(&json, &sig, trusted_keys)?;
    Ok((bundle, json, sig))
}

async fn download(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>, RemoteError> {
    let network = |error: reqwest::Error| RemoteError::Network(error.without_url().to_string());
    let mut response = client.get(url).timeout(FETCH_TIMEOUT).send().await.map_err(network)?;
    let status = response.status();
    if !status.is_success() {
        return Err(RemoteError::Status(status.as_u16()));
    }
    if response.content_length().is_some_and(|len| len > limit as u64) {
        return Err(RemoteError::TooLarge { limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network)? {
        if body.len() + chunk.len() > limit {
            return Err(RemoteError::TooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Fetch the remote manifest; if it is newer than `current_version`, cache it
/// and return it. Returns `None` when the remote copy is not newer.
pub async fn refresh(
    client: &reqwest::Client,
    url: &str,
    cache_dir: &Path,
    trusted_keys: &[&str],
    current_version: u64,
) -> Result<Option<ManifestBundle>, RemoteError> {
    let (bundle, json, sig) = fetch(client, url, trusted_keys).await?;
    let remote_version = bundle.index.manifest_version;
    if remote_version <= current_version {
        tracing::debug!(remote_version, current_version, "broker manifests are up to date");
        return Ok(None);
    }
    save_cache(cache_dir, &json, &sig)?;
    tracing::info!(remote_version, current_version, "downloaded newer broker manifests");
    Ok(Some(bundle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[9u8; 32])
    }

    fn public_hex(key: &SigningKey) -> String {
        hex::encode(key.verifying_key().to_bytes())
    }

    fn sign(key: &SigningKey, bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(key.sign(bytes).to_bytes())
    }

    /// The real bundled brokers dir as signed-manifest JSON, with
    /// `manifest_version` set to `version`.
    fn manifest_json(version: u64) -> Vec<u8> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("brokers");
        let files: BTreeMap<String, String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name().into_string().unwrap();
                let text = std::fs::read_to_string(entry.path()).unwrap();
                let text = if name == "index.toml" {
                    let bundled = regex::Regex::new(r"(?m)^manifest_version = \d+$").unwrap();
                    bundled.replace(&text, format!("manifest_version = {version}")).into_owned()
                } else {
                    text
                };
                (name, text)
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({ "files": files })).unwrap()
    }

    fn signed(version: u64) -> (Vec<u8>, String, String) {
        let key = signing_key();
        let json = manifest_json(version);
        let sig = sign(&key, &json);
        (json, sig, public_hex(&key))
    }

    #[test]
    fn valid_signed_bundle_parses() {
        let (json, sig, key) = signed(5);
        let bundle = verify_and_parse(&json, &sig, &[key.as_str()]).unwrap();
        assert_eq!(bundle.index.manifest_version, 5);
        assert!(bundle.get("zerodha").is_some());
    }

    #[test]
    fn tampered_byte_is_rejected() {
        let (mut json, sig, key) = signed(5);
        let last = json.len() - 2;
        json[last] ^= 1;
        assert!(matches!(verify_and_parse(&json, &sig, &[key.as_str()]), Err(RemoteError::BadSignature)));
    }

    #[test]
    fn unknown_key_and_missing_keys_are_rejected() {
        let (json, sig, _) = signed(5);
        let other = public_hex(&SigningKey::from_bytes(&[3u8; 32]));
        assert!(matches!(verify_and_parse(&json, &sig, &[other.as_str()]), Err(RemoteError::BadSignature)));
        assert!(matches!(verify_and_parse(&json, &sig, &[]), Err(RemoteError::NoTrustedKeys)));
        assert!(matches!(verify_and_parse(&json, &sig, &["not-hex"]), Err(RemoteError::NoTrustedKeys)));
    }

    #[test]
    fn malformed_signature_is_rejected() {
        let (json, _, key) = signed(5);
        assert!(matches!(verify_and_parse(&json, "@@@", &[key.as_str()]), Err(RemoteError::MalformedSignature)));
    }

    #[test]
    fn invalid_toml_inside_signed_bundle_is_rejected() {
        let key = signing_key();
        let json = serde_json::to_vec(&serde_json::json!({ "files": { "index.toml": "not = [valid" } })).unwrap();
        let sig = sign(&key, &json);
        let result = verify_and_parse(&json, &sig, &[public_hex(&key).as_str()]);
        assert!(matches!(result, Err(RemoteError::Invalid(RegistryError::Index(_)))));
    }

    #[test]
    fn signed_non_manifest_json_is_rejected() {
        let key = signing_key();
        let json = br#"{"other": 1}"#;
        let result = verify_and_parse(json, &sign(&key, json), &[public_hex(&key).as_str()]);
        assert!(matches!(result, Err(RemoteError::Malformed)));
    }

    #[test]
    fn oversize_is_rejected_before_verification() {
        let json = vec![b' '; MAX_MANIFEST_BYTES + 1];
        assert!(matches!(verify_and_parse(&json, "", &[]), Err(RemoteError::TooLarge { .. })));
    }

    #[test]
    fn cache_round_trips_and_rejects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let (json, sig, key) = signed(7);
        assert!(load_cached(dir.path(), &[key.as_str()]).is_none());

        save_cache(dir.path(), &json, &sig).unwrap();
        let cached = load_cached(dir.path(), &[key.as_str()]).unwrap();
        assert_eq!(cached.index.manifest_version, 7);

        std::fs::write(dir.path().join(MANIFEST_FILE), manifest_json(8)).unwrap();
        assert!(load_cached(dir.path(), &[key.as_str()]).is_none());
    }

    fn bundle_with_version(version: u64) -> ManifestBundle {
        let (json, sig, key) = signed(version);
        verify_and_parse(&json, &sig, &[key.as_str()]).unwrap()
    }

    #[test]
    fn choose_prefers_newest_and_ties_keep_bundled() {
        let bundled = bundle_with_version(3);
        let chosen = choose(bundled.clone(), [bundle_with_version(2), bundle_with_version(5), bundle_with_version(4)]);
        assert_eq!(chosen.index.manifest_version, 5);

        let mut tie = bundle_with_version(3);
        tie.index.default_tenant = "pocketful".into();
        let chosen = choose(bundled, [tie]);
        assert_eq!(chosen.default_tenant(), "cirrus");
    }

    async fn serve(json: Vec<u8>, sig: String) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/brokers-manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(json))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/brokers-manifest.json.sig"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!("{sig}\n")))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn refresh_caches_newer_manifest() {
        let (json, sig, key) = signed(9);
        let server = serve(json, sig).await;
        let dir = tempfile::tempdir().unwrap();
        let url = format!("{}/brokers-manifest.json", server.uri());

        let updated = refresh(&reqwest::Client::new(), &url, dir.path(), &[key.as_str()], 1).await.unwrap();
        assert_eq!(updated.map(|b| b.index.manifest_version), Some(9));
        assert_eq!(load_cached(dir.path(), &[key.as_str()]).map(|b| b.index.manifest_version), Some(9));
    }

    #[tokio::test]
    async fn refresh_skips_same_version() {
        let (json, sig, key) = signed(4);
        let server = serve(json, sig).await;
        let dir = tempfile::tempdir().unwrap();
        let url = format!("{}/brokers-manifest.json", server.uri());

        let updated = refresh(&reqwest::Client::new(), &url, dir.path(), &[key.as_str()], 4).await.unwrap();
        assert!(updated.is_none());
        assert!(!dir.path().join(MANIFEST_FILE).exists());
    }

    #[tokio::test]
    async fn fetch_reports_http_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        let url = format!("{}/brokers-manifest.json", server.uri());
        let key = public_hex(&signing_key());

        let result = fetch(&reqwest::Client::new(), &url, &[key.as_str()]).await;
        assert!(matches!(result, Err(RemoteError::Status(404))));
    }

    #[tokio::test]
    async fn fetch_rejects_oversize_body() {
        let key = public_hex(&signing_key());
        let server = serve(vec![b' '; MAX_MANIFEST_BYTES + 1], String::new()).await;
        let url = format!("{}/brokers-manifest.json", server.uri());

        let result = fetch(&reqwest::Client::new(), &url, &[key.as_str()]).await;
        assert!(matches!(result, Err(RemoteError::TooLarge { .. })));
    }
}
