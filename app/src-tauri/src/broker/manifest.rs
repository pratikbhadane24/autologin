//! Declarative broker definitions.
//!
//! Everything v1 hardcoded in `broker_logins.py` (URLs, client IDs, selectors,
//! timeouts, success markers) lives in one TOML file per broker. The Rust side
//! only interprets these definitions, so a broker DOM change is a data change.

use std::collections::{BTreeMap, HashSet};

use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::vars;
use crate::session::SessionPolicy;

pub const SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_STEP_TIMEOUT_MS: u64 = 10_000;
pub const DEFAULT_SUCCESS_TIMEOUT_MS: u64 = 30_000;

/// Placeholder that is generated at the moment it is used, not stored.
pub const TOTP_KEY: &str = "totp";

/// Prefix for query parameters of an intercepted callback URL.
pub const QUERY_PREFIX: &str = "query.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerKind {
    /// Drive the broker login page with a headless browser.
    Browser,
    /// Run a named Rust HTTP flow (`flow = "..."`) whose outputs become
    /// `{vars.*}` for the `[callback]` body. No browser is involved.
    Http,
    /// Open a single Cirrus URL that performs the login server-side.
    Redirect,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    #[default]
    Ready,
    /// Listed and importable, but skipped by the runner.
    ComingSoon,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSpec {
    pub key: String,
    pub label: String,
    #[serde(default = "default_true")]
    pub required: bool,
    /// Stored in the OS keyring, never returned to the UI.
    #[serde(default)]
    pub secret: bool,
    /// Holds a base32 TOTP secret; enables the `{totp}` placeholder.
    #[serde(default)]
    pub totp: bool,
    /// May be filled from the Cirrus dashboard's "Copy for AutoLogin" paste.
    /// Only non-secret values qualify.
    #[serde(default)]
    pub from_cirrus: bool,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub help: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSpec {
    /// `HH:MM`, local to `tz`.
    pub reset: String,
    pub tz: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Step {
    Goto {
        url: String,
    },
    /// Set the value of the first matching input.
    Fill {
        sel: Vec<String>,
        value: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Type key by key, for inputs that ignore programmatic value changes.
    Type {
        sel: Vec<String>,
        value: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    Click {
        sel: Vec<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Spread one value over N single-character boxes (OTP/PIN inputs).
    FillSplit {
        sel: Vec<String>,
        value: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    WaitFor {
        #[serde(default)]
        sel: Vec<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Run `steps` only if one of `when` appears within `timeout_ms`.
    Optional {
        when: Vec<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
        steps: Vec<Step>,
    },
    /// HTTP request whose JSON response feeds `{vars.*}` placeholders.
    Http {
        #[serde(default = "default_method")]
        method: String,
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        body: Option<String>,
        /// var name -> JSON pointer, e.g. `consent_id = "/consent_id"`.
        extract: BTreeMap<String, String>,
    },
    Eval {
        js: String,
    },
}

fn default_method() -> String {
    "GET".to_string()
}

/// A login succeeded when every configured condition holds: the page URL
/// matches `url_matches` (if set) and the page contains `text` (if set).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessSpec {
    #[serde(default)]
    pub url_matches: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default = "default_success_timeout")]
    pub timeout_ms: u64,
}

fn default_success_timeout() -> u64 {
    DEFAULT_SUCCESS_TIMEOUT_MS
}

/// How a finished broker login is handed to the Cirrus broker-auth API.
///
/// Browser brokers set `url_matches`: navigation to the broker's registered
/// redirect URL is intercepted (and blocked, since auth codes are single-use)
/// and its query string becomes `{query.*}`. HTTP brokers omit it and build
/// the body from `{vars.*}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallbackSpec {
    #[serde(default)]
    pub url_matches: Option<String>,
    /// Path on `{tenant.broker_auth_api}`, e.g. `/api/sessions/upstox/callback`.
    pub endpoint: String,
    /// JSON body: field name -> template.
    pub body: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureSpec {
    #[serde(default)]
    pub text_any: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerManifest {
    pub id: String,
    pub name: String,
    pub kind: BrokerKind,
    #[serde(default)]
    pub availability: Availability,
    /// Names accepted for this broker in CSV imports and v1 data.
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub help: Option<String>,
    /// Name of the HTTP flow for `kind = "http"`.
    #[serde(default)]
    pub flow: Option<String>,
    pub fields: Vec<FieldSpec>,
    #[serde(default)]
    pub consts: BTreeMap<String, String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub session: Option<SessionSpec>,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub success: Option<SuccessSpec>,
    #[serde(default)]
    pub failure: FailureSpec,
    #[serde(default)]
    pub callback: Option<CallbackSpec>,
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("{broker}: invalid TOML: {source}")]
    Parse { broker: String, source: toml::de::Error },
    #[error("{broker}: {message}")]
    Invalid { broker: String, message: String },
}

/// One tenant's `{tenant.*}` values.
pub type TenantValues = BTreeMap<String, String>;

impl BrokerManifest {
    pub fn from_toml(source_name: &str, text: &str) -> Result<Self, ManifestError> {
        toml::from_str(text).map_err(|source| ManifestError::Parse {
            broker: source_name.to_string(),
            source,
        })
    }

    pub fn field(&self, key: &str) -> Option<&FieldSpec> {
        self.fields.iter().find(|f| f.key == key)
    }

    pub fn secret_keys(&self) -> impl Iterator<Item = &str> {
        self.fields.iter().filter(|f| f.secret).map(|f| f.key.as_str())
    }

    pub fn session_policy(&self) -> SessionPolicy {
        self.session
            .as_ref()
            .and_then(|spec| {
                let reset = NaiveTime::parse_from_str(&spec.reset, "%H:%M").ok()?;
                let tz = spec.tz.parse().ok()?;
                Some(SessionPolicy { reset, tz })
            })
            .unwrap_or_default()
    }

    fn invalid(&self, message: impl Into<String>) -> ManifestError {
        ManifestError::Invalid { broker: self.id.clone(), message: message.into() }
    }

    /// Check internal consistency against every tenant the bundle defines.
    /// All tenants share one key set (the registry enforces this).
    pub fn validate(&self, tenants: &[&TenantValues]) -> Result<(), ManifestError> {
        let tenant_keys: HashSet<String> = tenants.first().map(|t| t.keys().cloned().collect()).unwrap_or_default();
        let tenant_keys = &tenant_keys;
        self.validate_fields()?;
        self.validate_session()?;
        if self.kind != BrokerKind::Http && self.steps.is_empty() {
            return Err(self.invalid("browser/redirect brokers need at least one step"));
        }
        if self.callback.is_none() && self.success.is_none() {
            return Err(self.invalid("a broker needs a [callback] or a [success] check"));
        }
        if let Some(success) = &self.success {
            if success.url_matches.is_none() && success.text.is_none() {
                return Err(self.invalid("[success] needs url_matches or text"));
            }
            if let Some(pattern) = &success.url_matches {
                self.check_url_pattern(pattern, tenants)?;
            }
        }

        let mut scope = self.base_scope(tenant_keys);
        self.add_flow_outputs(&mut scope)?;
        self.validate_steps(&self.steps, &mut scope, tenants)?;
        for template in self.consts.values().chain(self.headers.values()) {
            self.check_template(template, &scope)?;
        }
        self.validate_callback(scope, tenants)
    }

    /// URL patterns may use `{tenant.*}` (and only that), so each tenant's
    /// rendered pattern must be a valid regex. Use `{{`/`}}` for quantifiers.
    fn check_url_pattern(&self, pattern: &str, tenants: &[&TenantValues]) -> Result<(), ManifestError> {
        let keys = vars::keys(pattern).map_err(|e| self.invalid(e.to_string()))?;
        if let Some(bad) = keys.iter().find(|k| !k.starts_with("tenant.")) {
            return Err(self.invalid(format!("URL patterns may only use {{tenant.*}}, found {{{bad}}}")));
        }
        for tenant in tenants {
            let rendered = vars::render(pattern, |k| k.strip_prefix("tenant.").and_then(|k| tenant.get(k)).cloned())
                .map_err(|e| self.invalid(e.to_string()))?;
            compile_regex(&rendered).map_err(|e| self.invalid(e))?;
        }
        Ok(())
    }

    fn validate_callback(&self, mut scope: HashSet<String>, tenants: &[&TenantValues]) -> Result<(), ManifestError> {
        let Some(callback) = &self.callback else { return Ok(()) };
        if !callback.endpoint.starts_with("/api/") {
            return Err(self.invalid("callback.endpoint must start with /api/"));
        }
        if callback.body.is_empty() {
            return Err(self.invalid("callback.body must not be empty"));
        }
        match &callback.url_matches {
            Some(pattern) => {
                self.check_url_pattern(pattern, tenants)?;
                for template in callback.body.values() {
                    scope.extend(
                        vars::keys(template)
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|k| k.starts_with(QUERY_PREFIX)),
                    );
                }
            }
            None if self.kind == BrokerKind::Browser => {
                return Err(self.invalid("browser callbacks need url_matches"));
            }
            None => {}
        }
        for template in callback.body.values() {
            self.check_template(template, &scope)?;
        }
        Ok(())
    }

    fn validate_fields(&self) -> Result<(), ManifestError> {
        let mut seen = HashSet::new();
        for field in &self.fields {
            if !seen.insert(field.key.as_str()) {
                return Err(self.invalid(format!("duplicate field {:?}", field.key)));
            }
            if let Some(pattern) = &field.pattern {
                compile_regex(pattern).map_err(|e| self.invalid(e))?;
            }
            if field.from_cirrus && (field.secret || field.totp) {
                return Err(self.invalid(format!("secret field {:?} cannot be from_cirrus", field.key)));
            }
        }
        if self.field("client_id").is_none() {
            return Err(self.invalid("every broker needs a client_id field"));
        }
        Ok(())
    }

    fn validate_session(&self) -> Result<(), ManifestError> {
        let Some(spec) = &self.session else { return Ok(()) };
        NaiveTime::parse_from_str(&spec.reset, "%H:%M")
            .map_err(|_| self.invalid(format!("session.reset {:?} is not HH:MM", spec.reset)))?;
        spec.tz
            .parse::<chrono_tz::Tz>()
            .map_err(|_| self.invalid(format!("session.tz {:?} is not a timezone", spec.tz)))?;
        Ok(())
    }

    fn base_scope(&self, tenant_keys: &HashSet<String>) -> HashSet<String> {
        let fields = self.fields.iter().map(|f| f.key.clone());
        let consts = self.consts.keys().map(|k| format!("consts.{k}"));
        let tenant = tenant_keys.iter().map(|k| format!("tenant.{k}"));
        let mut scope: HashSet<String> = fields.chain(consts).chain(tenant).collect();
        if self.fields.iter().any(|f| f.totp) {
            scope.insert(TOTP_KEY.to_string());
        }
        scope
    }

    fn add_flow_outputs(&self, scope: &mut HashSet<String>) -> Result<(), ManifestError> {
        match (self.kind, self.flow.as_deref()) {
            (BrokerKind::Http, Some(name)) => {
                let outputs = http_flow_outputs(name)
                    .ok_or_else(|| self.invalid(format!("unknown http flow {name:?}")))?;
                scope.extend(outputs.iter().map(|k| format!("vars.{k}")));
                Ok(())
            }
            (BrokerKind::Http, None) => Err(self.invalid("kind = \"http\" needs a flow")),
            (_, Some(_)) => Err(self.invalid("flow is only valid with kind = \"http\"")),
            (_, None) => Ok(()),
        }
    }

    fn check_template(&self, template: &str, scope: &HashSet<String>) -> Result<(), ManifestError> {
        let keys = vars::keys(template).map_err(|e| self.invalid(e.to_string()))?;
        match keys.into_iter().find(|key| !scope.contains(key)) {
            Some(missing) => Err(self.invalid(format!("unknown placeholder {{{missing}}}"))),
            None => Ok(()),
        }
    }

    fn validate_steps(
        &self,
        steps: &[Step],
        scope: &mut HashSet<String>,
        tenants: &[&TenantValues],
    ) -> Result<(), ManifestError> {
        for step in steps {
            match step {
                Step::Goto { url } => self.check_template(url, scope)?,
                Step::Fill { sel, value, .. }
                | Step::Type { sel, value, .. }
                | Step::FillSplit { sel, value, .. } => {
                    self.require_selectors(sel)?;
                    self.check_template(value, scope)?;
                }
                Step::Click { sel, .. } => self.require_selectors(sel)?,
                Step::WaitFor { sel, url, .. } => {
                    if sel.is_empty() && url.is_none() {
                        return Err(self.invalid("wait_for needs sel or url"));
                    }
                    if let Some(pattern) = url {
                        self.check_url_pattern(pattern, tenants)?;
                    }
                }
                Step::Optional { when, steps, .. } => {
                    self.require_selectors(when)?;
                    self.validate_steps(steps, scope, tenants)?;
                }
                Step::Http { url, headers, body, extract, .. } => {
                    self.check_template(url, scope)?;
                    for template in headers.values().chain(body.iter()) {
                        self.check_template(template, scope)?;
                    }
                    scope.extend(extract.keys().map(|k| format!("vars.{k}")));
                }
                Step::Eval { js } => self.check_template(js, scope)?,
            }
        }
        Ok(())
    }

    fn require_selectors(&self, sel: &[String]) -> Result<(), ManifestError> {
        if sel.is_empty() {
            return Err(self.invalid("step needs at least one selector"));
        }
        Ok(())
    }
}

/// The `{vars.*}` each Rust HTTP flow produces.
pub fn http_flow_outputs(flow: &str) -> Option<&'static [&'static str]> {
    match flow {
        "motilal" => Some(&["auth_token"]),
        _ => None,
    }
}

fn compile_regex(pattern: &str) -> Result<regex::Regex, String> {
    regex::Regex::new(pattern).map_err(|e| format!("invalid regex {pattern:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANGEL: &str = r##"
id = "angel_one"
name = "Angel One"
kind = "browser"
aliases = ["angel", "angelone"]

[[fields]]
key = "client_id"
label = "Client ID"

[[fields]]
key = "mpin"
label = "MPIN"
secret = true
pattern = "^\\d{4,6}$"

[[fields]]
key = "totp_key"
label = "TOTP Secret"
secret = true
totp = true

[consts]
api_key = "oS35ILQ1"

[session]
reset = "05:00"
tz = "Asia/Kolkata"

[[steps]]
action = "goto"
url = "https://smartapi.angelbroking.com/publisher-login?api_key={consts.api_key}"

[[steps]]
action = "fill"
sel = ["#tot-totp"]
value = "{totp}"

[[steps]]
action = "optional"
when = ["text=Switch to TOTP"]
steps = [{ action = "click", sel = ["text=Switch to TOTP"] }]

[success]
url_matches = "^{tenant}"
text = "Account Saved!"
"##;

    fn tenant_values() -> TenantValues {
        [("cirrus_base".to_string(), "https://cirrus.trade".to_string())].into()
    }

    fn tenant() -> Vec<&'static TenantValues> {
        vec![Box::leak(Box::new(tenant_values()))]
    }

    fn angel() -> BrokerManifest {
        BrokerManifest::from_toml("angel_one", &ANGEL.replace("^{tenant}", "^https://cirrus")).unwrap()
    }

    #[test]
    fn parses_full_manifest() {
        let manifest = angel();
        assert_eq!(manifest.kind, BrokerKind::Browser);
        assert_eq!(manifest.steps.len(), 3);
        assert_eq!(manifest.secret_keys().collect::<Vec<_>>(), vec!["mpin", "totp_key"]);
        assert!(manifest.validate(&tenant()).is_ok());
    }

    #[test]
    fn session_policy_reads_manifest() {
        let policy = angel().session_policy();
        assert_eq!(policy.reset, NaiveTime::from_hms_opt(5, 0, 0).unwrap());
    }

    #[test]
    fn secret_fields_cannot_come_from_cirrus() {
        let mut manifest = angel();
        manifest.fields.iter_mut().find(|f| f.key == "mpin").unwrap().from_cirrus = true;
        let err = manifest.validate(&tenant()).unwrap_err().to_string();
        assert!(err.contains("cannot be from_cirrus"), "{err}");
    }

    #[test]
    fn rejects_unknown_placeholder() {
        let mut manifest = angel();
        manifest.steps.push(Step::Goto { url: "https://x/{consts.nope}".into() });
        let err = manifest.validate(&tenant()).unwrap_err().to_string();
        assert!(err.contains("consts.nope"), "{err}");
    }

    #[test]
    fn rejects_totp_without_totp_field() {
        let mut manifest = angel();
        manifest.fields.retain(|f| !f.totp);
        assert!(manifest.validate(&tenant()).is_err());
    }

    #[test]
    fn http_extract_makes_vars_available_to_later_steps() {
        let mut manifest = angel();
        manifest.steps.insert(
            0,
            Step::Http {
                method: "GET".into(),
                url: "{tenant.cirrus_base}/consent".into(),
                headers: BTreeMap::new(),
                body: None,
                extract: [("consent_id".to_string(), "/consent_id".to_string())].into(),
            },
        );
        manifest.steps.push(Step::Goto { url: "https://auth/{vars.consent_id}".into() });
        assert!(manifest.validate(&tenant()).is_ok());
    }

    #[test]
    fn rejects_unknown_action() {
        let bad = ANGEL.replace("action = \"goto\"", "action = \"teleport\"");
        assert!(matches!(BrokerManifest::from_toml("angel_one", &bad), Err(ManifestError::Parse { .. })));
    }

    #[test]
    fn url_patterns_render_tenant_values_and_reject_other_placeholders() {
        let mut manifest = angel();
        manifest.success.as_mut().unwrap().url_matches = Some("^{tenant.cirrus_base}/ok".into());
        assert!(manifest.validate(&tenant()).is_ok());
        manifest.success.as_mut().unwrap().url_matches = Some("^{client_id}/ok".into());
        assert!(manifest.validate(&tenant()).is_err());
    }

    #[test]
    fn rejects_bad_session_and_regex() {
        let mut manifest = angel();
        manifest.session = Some(SessionSpec { reset: "5am".into(), tz: "Asia/Kolkata".into() });
        assert!(manifest.validate(&tenant()).is_err());

        let mut manifest = angel();
        manifest.success.as_mut().unwrap().url_matches = Some("(".into());
        assert!(manifest.validate(&tenant()).is_err());
    }

    #[test]
    fn http_broker_exposes_flow_outputs() {
        let mut manifest = angel();
        manifest.kind = BrokerKind::Http;
        manifest.flow = Some("motilal".into());
        manifest.steps = vec![Step::Goto { url: "{tenant.cirrus_base}/motilal?token={vars.auth_token}".into() }];
        assert!(manifest.validate(&tenant()).is_ok());

        manifest.flow = Some("nope".into());
        assert!(manifest.validate(&tenant()).is_err());
        manifest.flow = None;
        assert!(manifest.validate(&tenant()).is_err());
    }

    fn upstox_callback() -> CallbackSpec {
        CallbackSpec {
            url_matches: Some("^https://cirrus\\.trade/add-broker-account/upstox".into()),
            endpoint: "/api/sessions/upstox/callback".into(),
            body: [
                ("client_id".to_string(), "{query.state}".to_string()),
                ("code".to_string(), "{query.code}".to_string()),
            ]
            .into(),
        }
    }

    #[test]
    fn callback_body_may_use_query_params_of_intercepted_url() {
        let mut manifest = angel();
        manifest.callback = Some(upstox_callback());
        assert!(manifest.validate(&tenant()).is_ok());
    }

    #[test]
    fn callback_rejects_bad_endpoint_and_query_without_interception() {
        let mut manifest = angel();
        let mut callback = upstox_callback();
        callback.endpoint = "https://evil.example/steal".into();
        manifest.callback = Some(callback);
        assert!(manifest.validate(&tenant()).is_err());

        let mut manifest = angel();
        manifest.kind = BrokerKind::Http;
        manifest.flow = Some("motilal".into());
        let mut callback = upstox_callback();
        callback.url_matches = None;
        manifest.callback = Some(callback);
        let err = manifest.validate(&tenant()).unwrap_err().to_string();
        assert!(err.contains("query.state"), "{err}");
    }

    #[test]
    fn broker_requires_success_check_or_callback() {
        let mut manifest = angel();
        manifest.success = None;
        assert!(manifest.validate(&tenant()).is_err());
        manifest.callback = Some(upstox_callback());
        assert!(manifest.validate(&tenant()).is_ok());
    }
}
