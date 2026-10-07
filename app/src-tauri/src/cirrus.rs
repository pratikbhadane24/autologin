//! Client for the Cirrus broker-auth API (`/api/sessions/*`).
//!
//! Only one-time broker authorization codes are sent here, never passwords,
//! PINs or TOTP secrets (see PRIVACY.md).

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::broker::context::TemplateContext;
use crate::broker::manifest::CallbackSpec;
use crate::broker::vars::TemplateError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ERROR_CHARS: usize = 300;

#[derive(Debug, Error)]
pub enum CirrusError {
    #[error(transparent)]
    Template(#[from] TemplateError),
    #[error("could not reach Cirrus: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Cirrus sign-in required or expired")]
    Unauthorized,
    #[error("Cirrus rejected the login: {0}")]
    Rejected(String),
}

/// `StandardResponse{success, message, data}` from the backend.
#[derive(Debug, Deserialize)]
struct StandardResponse {
    success: bool,
    #[serde(default)]
    message: Option<String>,
}

pub struct CirrusClient {
    http: reqwest::Client,
    base_url: String,
}

impl CirrusClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("static reqwest configuration is valid");
        Self { http, base_url: base_url.into().trim_end_matches('/').to_string() }
    }

    /// Render `spec.body` and POST it. Returns Cirrus's success message.
    pub async fn submit_callback(
        &self,
        spec: &CallbackSpec,
        ctx: &TemplateContext<'_>,
        bearer_token: Option<&str>,
    ) -> Result<String, CirrusError> {
        let body = spec
            .body
            .iter()
            .map(|(key, template)| Ok((key.clone(), Value::String(ctx.render(template)?))))
            .collect::<Result<Map<_, _>, TemplateError>>()?;

        let mut request = self.http.post(format!("{}{}", self.base_url, spec.endpoint)).json(&body);
        if let Some(token) = bearer_token {
            request = request.bearer_auth(token);
        }

        let response = request.send().await?;
        let status = response.status();
        let raw = response.text().await?;

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CirrusError::Unauthorized);
        }
        match serde_json::from_str::<StandardResponse>(&raw) {
            Ok(parsed) if status.is_success() && parsed.success => {
                Ok(parsed.message.unwrap_or_else(|| "Account saved".to_string()))
            }
            Ok(parsed) if parsed.message.is_some() => Err(CirrusError::Rejected(parsed.message.unwrap_or_default())),
            _ => Err(CirrusError::Rejected(error_detail(&raw))),
        }
    }
}

/// FastAPI errors are `{"detail": "..."}` or a list of validation errors.
fn error_detail(raw: &str) -> String {
    let parsed: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let detail = match parsed.get("detail") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("msg").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("; "),
        _ => raw.to_string(),
    };
    detail.chars().take(MAX_ERROR_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::broker::context::AccountValues;
    use crate::broker::registry::ManifestBundle;

    async fn submit_upstox(server: &MockServer, token: Option<&str>) -> Result<String, CirrusError> {
        let bundle = ManifestBundle::bundled().unwrap();
        let upstox = bundle.get("upstox").unwrap();
        let account: AccountValues = [("client_id".to_string(), "UP1".to_string())].into();
        let mut ctx = TemplateContext::new(upstox, &bundle, &account);
        ctx.set_query(HashMap::from([("code".to_string(), "abc".to_string())]));
        CirrusClient::new(server.uri())
            .submit_callback(upstox.callback.as_ref().unwrap(), &ctx, token)
            .await
    }

    #[tokio::test]
    async fn posts_rendered_body_with_bearer_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/sessions/upstox/callback"))
            .and(header("authorization", "Bearer jwt"))
            .and(body_json(json!({ "client_id": "UP1", "code": "abc" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "message": "Saved" })))
            .mount(&server)
            .await;
        assert_eq!(submit_upstox(&server, Some("jwt")).await.unwrap(), "Saved");
    }

    #[tokio::test]
    async fn maps_401_and_fastapi_detail_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "detail": "Not authenticated" })))
            .mount(&server)
            .await;
        assert!(matches!(submit_upstox(&server, None).await, Err(CirrusError::Unauthorized)));

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "detail": "Invalid code" })))
            .mount(&server)
            .await;
        let err = submit_upstox(&server, None).await.unwrap_err();
        assert_eq!(err.to_string(), "Cirrus rejected the login: Invalid code");
    }

    #[tokio::test]
    async fn success_false_is_a_rejection() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "success": false, "message": "Account not linked" })),
            )
            .mount(&server)
            .await;
        let err = submit_upstox(&server, None).await.unwrap_err();
        assert_eq!(err.to_string(), "Cirrus rejected the login: Account not linked");
    }

    #[test]
    fn joins_validation_error_messages() {
        let raw = r#"{"detail":[{"msg":"field required"},{"msg":"bad type"}]}"#;
        assert_eq!(error_detail(raw), "field required; bad type");
    }
}
