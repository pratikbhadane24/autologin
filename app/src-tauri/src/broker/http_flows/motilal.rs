//! Motilal Oswal: one `authdirectapi` (v7) call returning an AuthToken.
//! The password is sent as sha256(password + api_key), per Motilal's API.
//! v7 also needs the app's secret key in the `apisecretkey` header, which the
//! manifest's `[headers]` sends (see PythonSDK `MOFSLOPENAPI.validate`).

use std::collections::HashMap;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{broker_message, required, send_json, totp, FlowError};
use crate::broker::context::TemplateContext;

/// Header naming this "installation"; PythonSDK sends a fresh `uuid1()` per
/// session, so a fresh random id per login matches it.
const INSTALLED_APP_ID_HEADER: &str = "installedappid";

fn password_checksum(password: &str, api_key: &str) -> String {
    hex::encode(Sha256::digest(format!("{password}{api_key}").as_bytes()))
}

/// A random id in UUID form (version 4 layout).
fn installed_app_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("OS random number generator is available");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex::encode(bytes);
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

pub async fn run(ctx: &TemplateContext<'_>, client: &reqwest::Client) -> Result<HashMap<String, String>, FlowError> {
    let client_id = required(ctx, "client_id")?;
    let api_key = required(ctx, "api_key")?;
    let password = required(ctx, "password")?;
    // Sent as the apisecretkey header; v7 rejects logins without it.
    required(ctx, "api_secret")?;
    // DOB (dd/mm/yyyy) or PAN; PANs are upper case.
    let second_factor = ctx.field("dob").unwrap_or_default().to_ascii_uppercase();

    let payload = json!({
        "userid": client_id,
        "password": password_checksum(password, api_key),
        "2FA": second_factor,
        "totp": totp(ctx)?,
    });

    let mut request = client
        .post(ctx.konst("login_url")?)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header(INSTALLED_APP_ID_HEADER, installed_app_id())
        .json(&payload);
    for (name, value) in ctx.headers()? {
        request = request.header(name, value);
    }

    let (_, body, raw) = send_json("authdirectapi", request).await?;
    let succeeded = body.get("status").and_then(Value::as_str) == Some("SUCCESS");
    body.get("AuthToken")
        .and_then(Value::as_str)
        .filter(|token| succeeded && !token.is_empty())
        .map(|token| HashMap::from([("auth_token".to_string(), token.to_string())]))
        .ok_or_else(|| FlowError::Rejected { step: "authdirectapi", message: broker_message(&body, &raw) })
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::test_support::{account, bundle_against, TOTP_SECRET};
    use super::*;

    fn motilal_account() -> crate::broker::context::AccountValues {
        account(&[
            ("client_id", "EMUM1"),
            ("api_key", "KEY"),
            ("api_secret", "SECRET"),
            ("password", "pw"),
            ("dob", "abcde1234f"),
            ("totp_key", TOTP_SECRET),
        ])
    }

    #[test]
    fn checksum_matches_v1_hashlib() {
        // python3 -c 'import hashlib;print(hashlib.sha256(b"pwKEY").hexdigest())'
        assert_eq!(password_checksum("pw", "KEY"), "812bda846c64ffcd451ee3f31c9299e3526bd82b0f8467aa48138574d9768224");
    }

    #[test]
    fn installed_app_id_is_a_fresh_uuid() {
        let id = installed_app_id();
        let shape = regex::Regex::new("^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$").unwrap();
        assert!(shape.is_match(&id), "{id}");
        assert_ne!(id, installed_app_id());
    }

    #[tokio::test]
    async fn returns_auth_token_and_sends_v7_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/rest/login/v7/authdirectapi"))
            .and(header("vendorinfo", "EMUM1"))
            .and(header("ApiKey", "KEY"))
            .and(header("apisecretkey", "SECRET"))
            .and(header("accesstoken", ""))
            .and(header("sdkversion", "Python 5.0"))
            .and(header_exists(INSTALLED_APP_ID_HEADER))
            .and(body_partial_json(json!({
                "userid": "EMUM1",
                "password": password_checksum("pw", "KEY"),
                "2FA": "ABCDE1234F",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "SUCCESS", "AuthToken": "tok" })))
            .mount(&server)
            .await;

        let bundle = bundle_against(&server.uri(), "motilal");
        let acct = motilal_account();
        let ctx = TemplateContext::new(bundle.get("motilal").unwrap(), &bundle, &acct);
        let vars = run(&ctx, &super::super::client()).await.unwrap();
        assert_eq!(vars["auth_token"], "tok");
    }

    #[tokio::test]
    async fn reports_motilal_error_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "status": "ERROR", "message": "Invalid TOTP" })),
            )
            .mount(&server)
            .await;

        let bundle = bundle_against(&server.uri(), "motilal");
        let acct = motilal_account();
        let ctx = TemplateContext::new(bundle.get("motilal").unwrap(), &bundle, &acct);
        let err = run(&ctx, &super::super::client()).await.unwrap_err();
        assert_eq!(err.to_string(), "authdirectapi rejected: Invalid TOTP");
    }

    #[tokio::test]
    async fn missing_api_secret_fails_before_any_request() {
        let bundle = crate::broker::registry::ManifestBundle::bundled().unwrap();
        let mut acct = motilal_account();
        acct.remove("api_secret");
        let ctx = TemplateContext::new(bundle.get("motilal").unwrap(), &bundle, &acct);
        let err = run(&ctx, &super::super::client()).await.unwrap_err();
        assert!(matches!(err, FlowError::MissingField(ref f) if f == "api_secret"));
    }
}
