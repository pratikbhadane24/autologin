//! Values that manifest templates resolve against for one login attempt.

use std::collections::HashMap;

use super::manifest::{BrokerManifest, QUERY_PREFIX, TOTP_KEY};
use super::registry::ManifestBundle;
use super::vars::{self, TemplateError};
use crate::totp;

/// Account field values (secrets included) for a single login.
pub type AccountValues = HashMap<String, String>;

pub struct TemplateContext<'a> {
    manifest: &'a BrokerManifest,
    bundle: &'a ManifestBundle,
    account: &'a AccountValues,
    vars: HashMap<String, String>,
    query: HashMap<String, String>,
    tenant_id: String,
}

impl<'a> TemplateContext<'a> {
    pub fn new(manifest: &'a BrokerManifest, bundle: &'a ManifestBundle, account: &'a AccountValues) -> Self {
        Self {
            manifest,
            bundle,
            account,
            vars: HashMap::new(),
            query: HashMap::new(),
            tenant_id: bundle.default_tenant().to_string(),
        }
    }

    /// Resolve `{tenant.*}` against this tenant instead of the default.
    /// Callers must pass a tenant the bundle defines.
    pub fn with_tenant(mut self, tenant_id: &str) -> Self {
        self.tenant_id = tenant_id.to_string();
        self
    }

    pub fn manifest(&self) -> &BrokerManifest {
        self.manifest
    }

    pub fn set_var(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    /// Query parameters of the intercepted callback URL.
    pub fn set_query(&mut self, query: HashMap<String, String>) {
        self.query = query;
    }

    pub fn field(&self, key: &str) -> Option<&str> {
        self.account.get(key).map(|v| v.trim())
    }

    /// A fresh code from the manifest's TOTP field, generated on each call.
    pub fn totp(&self) -> Option<String> {
        let key_field = self.manifest.fields.iter().find(|f| f.totp)?;
        totp::current_code(self.field(&key_field.key)?).ok()
    }

    /// Values that don't depend on consts, so consts can reference them
    /// without recursion.
    fn lookup_base(&self, key: &str) -> Option<String> {
        if key == TOTP_KEY {
            return self.totp();
        }
        if let Some(name) = key.strip_prefix("tenant.") {
            return self.bundle.tenant_value(&self.tenant_id, name).map(str::to_string);
        }
        if let Some(name) = key.strip_prefix("vars.") {
            return self.vars.get(name).cloned();
        }
        if let Some(name) = key.strip_prefix(QUERY_PREFIX) {
            return self.query.get(name).cloned();
        }
        self.field(key).map(str::to_string)
    }

    pub fn lookup(&self, key: &str) -> Option<String> {
        match key.strip_prefix("consts.") {
            Some(name) => {
                let raw = self.manifest.consts.get(name)?;
                vars::render(raw, |k| self.lookup_base(k)).ok()
            }
            None => self.lookup_base(key),
        }
    }

    pub fn render(&self, template: &str) -> Result<String, TemplateError> {
        vars::render(template, |key| self.lookup(key))
    }

    /// `consts.<name>` rendered, for Rust flows that need a configured value.
    pub fn konst(&self, name: &str) -> Result<String, TemplateError> {
        self.lookup(&format!("consts.{name}"))
            .ok_or_else(|| TemplateError::Unresolved(format!("consts.{name}")))
    }

    /// The manifest's `[headers]` table, rendered.
    pub fn headers(&self) -> Result<Vec<(String, String)>, TemplateError> {
        self.manifest
            .headers
            .iter()
            .map(|(name, template)| Ok((name.clone(), self.render(template)?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consts_can_reference_tenant_values() {
        let bundle = ManifestBundle::bundled().unwrap();
        let mut fyers = bundle.get("fyers").unwrap().clone();
        fyers.consts.insert("redirect_uri".into(), "{tenant.cirrus_app}/add-broker-account/fyers".into());
        let account = AccountValues::new();
        let ctx = TemplateContext::new(&fyers, &bundle, &account);
        assert_eq!(ctx.konst("redirect_uri").unwrap(), "https://app.cirrus.trade/add-broker-account/fyers");
    }

    #[test]
    fn tenant_placeholders_follow_the_chosen_tenant() {
        let bundle = ManifestBundle::bundled().unwrap();
        let pocketful = bundle.get("pocketful").unwrap();
        let account = AccountValues::new();
        let ctx = TemplateContext::new(pocketful, &bundle, &account).with_tenant("pocketful");
        assert_eq!(ctx.render("{tenant.cirrus_app}").unwrap(), "https://cirrus.pocketful.in");
    }

    #[test]
    fn fields_are_trimmed_and_vars_resolve() {
        let bundle = ManifestBundle::bundled().unwrap();
        let fyers = bundle.get("fyers").unwrap();
        let account: AccountValues = [("client_id".to_string(), "  XA123 ".to_string())].into();
        let mut ctx = TemplateContext::new(fyers, &bundle, &account);
        ctx.set_var("auth_code", "a b");
        assert_eq!(ctx.render("{client_id}|{url:vars.auth_code}").unwrap(), "XA123|a%20b");
    }
}
