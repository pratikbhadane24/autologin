//! Brokers whose login is a sequence of API calls rather than a web form.
//! Each flow reads endpoints and app ids from the manifest `[consts]` and
//! returns the `{vars.*}` its manifest steps need (see `http_flow_outputs`).

mod motilal;

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;
use thiserror::Error;

use super::context::TemplateContext;
use super::vars::TemplateError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Error)]
pub enum FlowError {
    #[error("unknown http flow {0:?}")]
    UnknownFlow(String),
    #[error("missing account field {0:?}")]
    MissingField(String),
    #[error("TOTP secret is invalid")]
    InvalidTotp,
    #[error(transparent)]
    Template(#[from] TemplateError),
    #[error("network error during {step}: {source}")]
    Network { step: &'static str, source: reqwest::Error },
    /// The broker answered but refused; `message` is the broker's own text.
    #[error("{step} rejected: {message}")]
    Rejected { step: &'static str, message: String },
}

/// Redirects are not followed: a flow reads each broker answer as sent.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("static reqwest configuration is valid")
}

pub async fn run(
    flow: &str,
    ctx: &TemplateContext<'_>,
    client: &reqwest::Client,
) -> Result<HashMap<String, String>, FlowError> {
    match flow {
        "motilal" => motilal::run(ctx, client).await,
        other => Err(FlowError::UnknownFlow(other.to_string())),
    }
}

fn required<'c>(ctx: &'c TemplateContext<'_>, key: &str) -> Result<&'c str, FlowError> {
    ctx.field(key)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| FlowError::MissingField(key.to_string()))
}

fn totp(ctx: &TemplateContext<'_>) -> Result<String, FlowError> {
    ctx.totp().ok_or(FlowError::InvalidTotp)
}

/// The broker's error text, from whichever field it uses, else the raw body.
fn broker_message(body: &Value, raw: &str) -> String {
    ["message", "Message", "msg", "error"]
        .iter()
        .find_map(|key| body.get(key).and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| raw.chars().take(200).collect())
}

/// Send `request`, and return `(status, parsed JSON, raw text)`.
async fn send_json(
    step: &'static str,
    request: reqwest::RequestBuilder,
) -> Result<(reqwest::StatusCode, Value, String), FlowError> {
    let response = request.send().await.map_err(|source| FlowError::Network { step, source })?;
    let status = response.status();
    let raw = response.text().await.map_err(|source| FlowError::Network { step, source })?;
    let body = serde_json::from_str(&raw).unwrap_or(Value::Null);
    Ok((status, body, raw))
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::broker::context::AccountValues;
    use crate::broker::registry::ManifestBundle;

    /// The bundled manifests with every `*_base`/`*_url` const of `broker`
    /// pointed at `server`.
    pub fn bundle_against(server: &str, broker: &str) -> ManifestBundle {
        let mut bundle = ManifestBundle::bundled().unwrap();
        let manifest = bundle.brokers.get_mut(broker).unwrap();
        for (key, value) in manifest.consts.iter_mut() {
            if key.ends_with("_base") || key.ends_with("_url") {
                let path = url::Url::parse(value).map(|u| u.path().to_string()).unwrap_or_default();
                *value = format!("{server}{path}");
            }
        }
        bundle
    }

    pub const TOTP_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    pub fn account(pairs: &[(&str, &str)]) -> AccountValues {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }
}
