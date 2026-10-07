//! Fyers: send_login_otp -> verify_otp -> verify_pin -> token (308 + auth code).
//! Ported from v1 `utils/api.py::get_auto_code_fyers`.

use std::collections::HashMap;

use serde_json::{json, Value};

use super::{broker_message, required, send_json, totp, FlowError};
use crate::broker::context::TemplateContext;

pub async fn run(ctx: &TemplateContext<'_>, client: &reqwest::Client) -> Result<HashMap<String, String>, FlowError> {
    let fy_id = required(ctx, "client_id")?;
    let pin = required(ctx, "mpin")?;
    let vagator = ctx.konst("vagator_base")?;
    let api = ctx.konst("api_base")?;
    let app_id = ctx.konst("app_id")?;

    let request_key = post_for_key(
        client,
        "send_login_otp",
        &format!("{vagator}/send_login_otp"),
        json!({ "fy_id": fy_id, "app_id": app_id }),
    )
    .await?;

    let request_key = post_for_key(
        client,
        "verify_otp",
        &format!("{vagator}/verify_otp"),
        json!({ "request_key": request_key, "otp": totp(ctx)? }),
    )
    .await?;

    let (status, body, raw) = send_json(
        "verify_pin",
        client.post(format!("{vagator}/verify_pin")).json(&json!({
            "request_key": request_key, "identity_type": "pin", "identifier": pin,
        })),
    )
    .await?;
    let access_token = body
        .pointer("/data/access_token")
        .and_then(Value::as_str)
        .filter(|_| status.is_success())
        .ok_or_else(|| FlowError::Rejected { step: "verify_pin", message: broker_message(&body, &raw) })?;

    let auth_code = token(ctx, client, &api, fy_id, &app_id, access_token).await?;
    Ok(HashMap::from([("auth_code".to_string(), auth_code)]))
}

async fn post_for_key(
    client: &reqwest::Client,
    step: &'static str,
    url: &str,
    payload: Value,
) -> Result<String, FlowError> {
    let (status, body, raw) = send_json(step, client.post(url).json(&payload)).await?;
    body.get("request_key")
        .and_then(Value::as_str)
        .filter(|_| status.is_success())
        .map(str::to_string)
        .ok_or_else(|| FlowError::Rejected { step, message: broker_message(&body, &raw) })
}

async fn token(
    ctx: &TemplateContext<'_>,
    client: &reqwest::Client,
    api: &str,
    fy_id: &str,
    app_id: &str,
    access_token: &str,
) -> Result<String, FlowError> {
    let payload = json!({
        "fyers_id": fy_id,
        "app_id": app_id,
        "redirect_uri": ctx.konst("redirect_uri")?,
        "appType": ctx.konst("app_type")?,
        "code_challenge": "",
        "state": ctx.konst("state")?,
        "scope": "",
        "nonce": "",
        "response_type": "code",
        "create_cookie": true,
    });
    let (status, body, raw) =
        send_json("token", client.post(format!("{api}/token")).bearer_auth(access_token).json(&payload)).await?;

    let rejected = || FlowError::Rejected { step: "token", message: broker_message(&body, &raw) };
    if status != reqwest::StatusCode::PERMANENT_REDIRECT {
        return Err(rejected());
    }
    let redirect = body.get("Url").and_then(Value::as_str).ok_or_else(rejected)?;
    url::Url::parse(redirect)
        .ok()
        .and_then(|u| u.query_pairs().find(|(k, _)| k == "auth_code").map(|(_, v)| v.into_owned()))
        .ok_or_else(rejected)
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::test_support::{account, bundle_against, TOTP_SECRET};
    use super::*;

    fn fyers_account() -> crate::broker::context::AccountValues {
        account(&[("client_id", "XA123"), ("mpin", "1234"), ("totp_key", TOTP_SECRET)])
    }

    async fn mount_happy_path(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/vagator/v2/send_login_otp"))
            .and(body_partial_json(json!({ "fy_id": "XA123", "app_id": "YPUVOWAXFE-100" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "request_key": "rk1" })))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/vagator/v2/verify_otp"))
            .and(body_partial_json(json!({ "request_key": "rk1" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "request_key": "rk2" })))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/vagator/v2/verify_pin"))
            .and(body_partial_json(json!({ "request_key": "rk2", "identifier": "1234" })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "data": { "access_token": "at" } })),
            )
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn returns_auth_code_from_308_body() {
        let server = MockServer::start().await;
        mount_happy_path(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/v3/token"))
            .and(header("authorization", "Bearer at"))
            .respond_with(ResponseTemplate::new(308).set_body_json(
                json!({ "Url": "https://app.tradinx.in/broker-login/fyers-login?s=ok&auth_code=CODE123&state=x" }),
            ))
            .mount(&server)
            .await;

        let bundle = bundle_against(&server.uri(), "fyers");
        let acct = fyers_account();
        let ctx = TemplateContext::new(bundle.get("fyers").unwrap(), &bundle, &acct);
        let vars = run(&ctx, &super::super::client()).await.unwrap();
        assert_eq!(vars["auth_code"], "CODE123");
    }

    #[tokio::test]
    async fn surfaces_broker_message_on_rejection() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/vagator/v2/send_login_otp"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "message": "Invalid Fyers ID" })))
            .mount(&server)
            .await;

        let bundle = bundle_against(&server.uri(), "fyers");
        let acct = fyers_account();
        let ctx = TemplateContext::new(bundle.get("fyers").unwrap(), &bundle, &acct);
        let err = run(&ctx, &super::super::client()).await.unwrap_err();
        assert_eq!(err.to_string(), "send_login_otp rejected: Invalid Fyers ID");
    }

    #[tokio::test]
    async fn missing_pin_fails_before_any_request() {
        let bundle = crate::broker::registry::ManifestBundle::bundled().unwrap();
        let acct = account(&[("client_id", "XA123")]);
        let ctx = TemplateContext::new(bundle.get("fyers").unwrap(), &bundle, &acct);
        let err = run(&ctx, &super::super::client()).await.unwrap_err();
        assert!(matches!(err, FlowError::MissingField(ref f) if f == "mpin"));
    }

}
