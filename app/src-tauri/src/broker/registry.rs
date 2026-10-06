//! The set of broker manifests the app runs with.
//!
//! A `ManifestBundle` is `index.toml` plus one TOML per broker. The bundled
//! copy is compiled into the binary; newer signed copies fetched at runtime
//! (see `remote`) replace it when they validate.

use std::collections::{BTreeMap, HashSet};

use include_dir::{include_dir, Dir};
use serde::{Deserialize, Serialize};

use super::manifest::{BrokerManifest, ManifestError, TenantValues, SCHEMA_VERSION};

static BUNDLED_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/brokers");

const INDEX_FILE: &str = "index.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestIndex {
    pub schema_version: u32,
    pub manifest_version: u64,
    /// Tenant used when an account doesn't name one.
    pub default_tenant: String,
    /// Known Cirrus deployments (`cirrus`, `pocketful`, ...). A Cirrus paste
    /// names one of these ids; raw URLs are never taken from a paste.
    pub tenants: BTreeMap<String, TenantValues>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManifestBundle {
    pub index: ManifestIndex,
    /// Keyed by broker id, ordered for stable UI listing.
    pub brokers: BTreeMap<String, BrokerManifest>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("missing {INDEX_FILE}")]
    MissingIndex,
    #[error("{INDEX_FILE}: {0}")]
    Index(#[from] toml::de::Error),
    #[error("schema_version {found} is not supported (expected {SCHEMA_VERSION})")]
    UnsupportedSchema { found: u32 },
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("{file}: id {id:?} must match the file name")]
    IdMismatch { file: String, id: String },
    #[error("default_tenant {0:?} is not defined in [tenants]")]
    UnknownDefaultTenant(String),
    #[error("tenant {tenant:?} must define exactly the same keys as {reference:?}")]
    TenantKeysDiffer { tenant: String, reference: String },
    #[error("alias {alias:?} is used by both {first} and {second}")]
    DuplicateAlias { alias: String, first: String, second: String },
}

impl ManifestBundle {
    /// Build and validate a bundle from `(file name, contents)` pairs.
    pub fn from_files<'a, I>(files: I) -> Result<Self, RegistryError>
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut index_text = None;
        let mut broker_files = Vec::new();
        for (name, text) in files {
            if name == INDEX_FILE {
                index_text = Some(text);
            } else if let Some(stem) = name.strip_suffix(".toml") {
                broker_files.push((stem, text));
            }
        }

        let index: ManifestIndex = toml::from_str(index_text.ok_or(RegistryError::MissingIndex)?)?;
        if index.schema_version != SCHEMA_VERSION {
            return Err(RegistryError::UnsupportedSchema { found: index.schema_version });
        }

        check_tenants(&index)?;
        let tenants: Vec<&TenantValues> = index.tenants.values().collect();
        let mut brokers = BTreeMap::new();
        for (stem, text) in broker_files {
            let manifest = BrokerManifest::from_toml(stem, text)?;
            if manifest.id != stem {
                return Err(RegistryError::IdMismatch { file: format!("{stem}.toml"), id: manifest.id });
            }
            manifest.validate(&tenants)?;
            brokers.insert(manifest.id.clone(), manifest);
        }

        let bundle = Self { index, brokers };
        bundle.check_aliases()?;
        Ok(bundle)
    }

    /// The manifests compiled into this binary.
    pub fn bundled() -> Result<Self, RegistryError> {
        Self::from_files(
            BUNDLED_DIR
                .files()
                .filter_map(|file| Some((file.path().to_str()?, file.contents_utf8()?))),
        )
    }

    fn check_aliases(&self) -> Result<(), RegistryError> {
        let mut owners: BTreeMap<String, &str> = BTreeMap::new();
        for manifest in self.brokers.values() {
            for alias in manifest.aliases.iter().map(|a| normalize_alias(a)).chain([manifest.id.clone()]) {
                match owners.get(&alias) {
                    Some(first) if *first != manifest.id => {
                        return Err(RegistryError::DuplicateAlias {
                            alias,
                            first: first.to_string(),
                            second: manifest.id.clone(),
                        })
                    }
                    _ => {
                        owners.insert(alias, &manifest.id);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&BrokerManifest> {
        self.brokers.get(id)
    }

    /// Resolve a broker name from CSV or v1 data ("Motilal Oswal", "kotak neo",
    /// "5paisa") to a manifest. One lookup replaces v1's five name maps.
    pub fn resolve_alias(&self, name: &str) -> Option<&BrokerManifest> {
        let wanted = normalize_alias(name);
        self.brokers.values().find(|manifest| {
            normalize_alias(&manifest.id) == wanted
                || normalize_alias(&manifest.name) == wanted
                || manifest.aliases.iter().any(|alias| normalize_alias(alias) == wanted)
        })
    }

    /// A `{tenant.*}` value for `tenant_id`.
    pub fn tenant_value(&self, tenant_id: &str, key: &str) -> Option<&str> {
        self.index.tenants.get(tenant_id)?.get(key).map(String::as_str)
    }

    pub fn has_tenant(&self, tenant_id: &str) -> bool {
        self.index.tenants.contains_key(tenant_id)
    }

    pub fn default_tenant(&self) -> &str {
        &self.index.default_tenant
    }
}

fn check_tenants(index: &ManifestIndex) -> Result<(), RegistryError> {
    if !index.tenants.contains_key(&index.default_tenant) {
        return Err(RegistryError::UnknownDefaultTenant(index.default_tenant.clone()));
    }
    let reference = &index.tenants[&index.default_tenant];
    let reference_keys: HashSet<&String> = reference.keys().collect();
    match index.tenants.iter().find(|(_, values)| values.keys().collect::<HashSet<_>>() != reference_keys) {
        Some((tenant, _)) => Err(RegistryError::TenantKeysDiffer {
            tenant: tenant.clone(),
            reference: index.default_tenant.clone(),
        }),
        None => Ok(()),
    }
}

/// Lowercase and drop everything but letters and digits, so "Kotak Neo",
/// "kotak_neo" and "kotakneo" compare equal.
pub fn normalize_alias(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIER_ONE: &[&str] = &["fivepaisa", "fyers", "motilal", "pocketful", "upstox", "zerodha"];

    #[test]
    fn bundled_manifests_all_validate() {
        let bundle = ManifestBundle::bundled().unwrap_or_else(|e| panic!("{e}"));
        for id in TIER_ONE {
            assert!(bundle.get(id).is_some(), "missing bundled broker {id}");
        }
    }

    #[test]
    fn resolves_aliases_case_and_punctuation_insensitively() {
        let bundle = ManifestBundle::bundled().unwrap();
        let resolve = |name| bundle.resolve_alias(name).map(|m| m.id.as_str());
        assert_eq!(resolve("Motilal Oswal"), Some("motilal"));
        assert_eq!(resolve("5Paisa"), Some("fivepaisa"));
        assert_eq!(resolve("  ZERODHA "), Some("zerodha"));
        assert_eq!(resolve("unknown broker"), None);
    }

    const INDEX: &str = "schema_version = 1\nmanifest_version = 1\ndefault_tenant = \"cirrus\"\n[tenants.cirrus]\ncirrus_base = \"https://c\"\n";
    const MINIMAL: &str = r#"
id = "demo"
name = "Demo"
kind = "browser"
aliases = ["shared"]
[[fields]]
key = "client_id"
label = "ID"
[[steps]]
action = "goto"
url = "{tenant.cirrus_base}/x"
[success]
text = "ok"
"#;

    #[test]
    fn bundled_tenants_include_cirrus_default_and_pocketful() {
        let bundle = ManifestBundle::bundled().unwrap();
        assert_eq!(bundle.default_tenant(), "cirrus");
        assert_eq!(bundle.tenant_value("cirrus", "cirrus_app"), Some("https://app.cirrus.trade"));
        assert_eq!(bundle.tenant_value("pocketful", "cirrus_app"), Some("https://cirrus.pocketful.in"));
        assert!(!bundle.has_tenant("evil"));
    }

    #[test]
    fn rejects_tenants_with_different_keys_and_unknown_default() {
        let uneven = format!("{INDEX}[tenants.other]\nsomething_else = \"x\"\n");
        assert!(matches!(
            ManifestBundle::from_files([("index.toml", uneven.as_str())]),
            Err(RegistryError::TenantKeysDiffer { .. })
        ));
        let missing = INDEX.replace("default_tenant = \"cirrus\"", "default_tenant = \"nope\"");
        assert!(matches!(
            ManifestBundle::from_files([("index.toml", missing.as_str())]),
            Err(RegistryError::UnknownDefaultTenant(_))
        ));
    }

    #[test]
    fn rejects_id_that_does_not_match_file_name() {
        let err = ManifestBundle::from_files([("index.toml", INDEX), ("other.toml", MINIMAL)]).unwrap_err();
        assert!(matches!(err, RegistryError::IdMismatch { .. }));
    }

    #[test]
    fn rejects_duplicate_alias_across_brokers() {
        let second = MINIMAL.replace("\"demo\"", "\"demo2\"");
        let err = ManifestBundle::from_files([
            ("index.toml", INDEX),
            ("demo.toml", MINIMAL),
            ("demo2.toml", second.as_str()),
        ])
        .unwrap_err();
        assert!(matches!(err, RegistryError::DuplicateAlias { .. }));
    }

    #[test]
    fn rejects_unsupported_schema_and_missing_index() {
        let future = INDEX.replace("schema_version = 1", "schema_version = 99");
        assert!(matches!(
            ManifestBundle::from_files([("index.toml", future.as_str())]),
            Err(RegistryError::UnsupportedSchema { found: 99 })
        ));
        assert!(matches!(ManifestBundle::from_files([("demo.toml", MINIMAL)]), Err(RegistryError::MissingIndex)));
    }
}
