//! Motilal Oswal: one `authdirectapi` call returning an AuthToken.
//! The password is sent as sha256(password + api_key), per Motilal's API.

use std::collections::HashMap;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{broker_message, required, send_json, totp, FlowError};
use crate::broker::context::TemplateContext;

fn password_checksum(password: &str, api_key: &str) -> String {
    hex::encode(Sha256::digest(format!("{password}{api_key}").as_bytes()))
}

pub async fn run(ctx: &TemplateContext<'_>, client: &reqwest::Client) -> Result<HashMap<String, String>, FlowError> {
    let client_id = required(ctx, "client_id")?;
    let api_key = required(ctx, "api_key")?;
    let password = required(ctx, "password")?;
    let dob = ctx.field("dob").unwrap_or_default();

    let payload = json!({
        "userid": client_id,
        "password": password_checksum(password, api_key),
        "2FA": dob,
        "totp": totp(ctx)?,
    });

    let mut request = client
        .post(ctx.konst("login_url")?)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
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
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::test_support::{account, bundle_against, TOTP_SECRET};
    use super::*;

    fn motilal_account() -> crate::broker::context::AccountValues {
        account(&[
            ("client_id", "EMUM1"),
            ("api_key", "KEY"),
            ("password", "pw"),
            ("dob", "01/01/1990"),
            ("totp_key", TOTP_SECRET),
        ])
    }

    #[test]
    fn checksum_matches_v1_hashlib() {
        // python3 -c 'import hashlib;print(hashlib.sha256(b"pwKEY").hexdigest())'
        assert_eq!(password_checksum("pw", "KEY"), "812bda846c64ffcd451ee3f31c9299e3526bd82b0f8467aa48138574d9768224");
    }

    #[tokio::test]
    async fn returns_auth_token_and_sends_manifest_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/rest/login/v3/authdirectapi"))
            .and(header("vendorinfo", "EMUM1"))
            .and(header("ApiKey", "KEY"))
            .and(body_partial_json(json!({
                "userid": "EMUM1",
                "password": password_checksum("pw", "KEY"),
                "2FA": "01/01/1990",
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
}
