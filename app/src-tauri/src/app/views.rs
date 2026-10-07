//! Data shapes sent to the UI. None of them carries a secret value: secret
//! fields appear only by name ("set" / "missing").

use chrono::Utc;
use serde::Serialize;

use crate::broker::manifest::{Availability, BrokerKind, FieldSpec};
use crate::broker::registry::ManifestBundle;
use crate::store::accounts::{Account, LoginStatus};
use crate::store::validate;

#[derive(Debug, Serialize)]
pub struct BrokerInfo {
    pub id: String,
    pub name: String,
    pub kind: BrokerKind,
    pub coming_soon: bool,
    pub help: Option<String>,
    pub fields: Vec<FieldSpec>,
}

#[derive(Debug, Serialize)]
pub struct TenantInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct Catalog {
    pub brokers: Vec<BrokerInfo>,
    pub tenants: Vec<TenantInfo>,
    pub default_tenant: String,
    pub manifest_version: u64,
}

pub fn catalog(bundle: &ManifestBundle) -> Catalog {
    let brokers = bundle
        .brokers
        .values()
        .map(|m| BrokerInfo {
            id: m.id.clone(),
            name: m.name.clone(),
            kind: m.kind,
            coming_soon: m.availability == Availability::ComingSoon,
            help: m.help.clone(),
            fields: m.fields.clone(),
        })
        .collect();
    let tenants = bundle
        .index
        .tenants
        .iter()
        .map(|(id, values)| TenantInfo { id: id.clone(), name: values.get("name").cloned().unwrap_or_else(|| id.clone()) })
        .collect();
    Catalog {
        brokers,
        tenants,
        default_tenant: bundle.default_tenant().to_string(),
        manifest_version: bundle.index.manifest_version,
    }
}

#[derive(Debug, Serialize)]
pub struct AccountView {
    #[serde(flatten)]
    pub account: Account,
    /// Status after applying the broker's daily session reset.
    pub effective_status: LoginStatus,
    pub broker_name: String,
    /// Labels of required secrets that are not saved yet.
    pub missing: Vec<String>,
    pub coming_soon: bool,
}

pub fn account_view(account: Account, bundle: &ManifestBundle) -> AccountView {
    let manifest = bundle.get(&account.broker_id);
    let missing = manifest
        .map(|m| {
            validate::missing_fields(m, &account.fields, &account.secret_keys)
                .iter()
                .filter_map(|k| m.field(k).map(|f| f.label.clone()))
                .collect()
        })
        .unwrap_or_default();
    AccountView {
        effective_status: account.effective_status(manifest, Utc::now()),
        broker_name: manifest.map_or_else(|| account.broker_id.clone(), |m| m.name.clone()),
        coming_soon: manifest.is_some_and(|m| m.availability == Availability::ComingSoon),
        missing,
        account,
    }
}
