//! Validation of account field values against a broker manifest. Errors are
//! keyed by field so the form can show them next to the right input.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::broker::manifest::BrokerManifest;
use crate::totp;

/// field key -> user-facing message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FieldErrors(pub BTreeMap<String, String>);

impl FieldErrors {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn add(&mut self, key: &str, message: impl Into<String>) {
        self.0.entry(key.to_string()).or_insert_with(|| message.into());
    }
}

impl std::fmt::Display for FieldErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = self.0.iter().map(|(k, v)| format!("{k}: {v}")).collect();
        write!(f, "{}", parts.join("; "))
    }
}

/// Trim every value and drop empty ones, so "  " counts as not provided.
pub fn normalize(values: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(k, v)| (k.clone(), v.trim().to_string()))
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

/// How strictly `required` applies to secret fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    /// Every required field must be present (Add/Edit form).
    Strict,
    /// Secrets may be missing (imports without secrets); the account is
    /// saved as "needs setup" and skipped by runs until completed.
    AllowMissingSecrets,
}

/// Check `values` (already normalized). `stored_secrets` names secret fields
/// that already have a saved value, which satisfies `required` on edit.
pub fn validate(
    manifest: &BrokerManifest,
    values: &BTreeMap<String, String>,
    stored_secrets: &[String],
    completeness: Completeness,
) -> Result<(), FieldErrors> {
    let mut errors = FieldErrors::default();

    for key in values.keys() {
        if manifest.field(key).is_none() {
            errors.add(key, format!("{} has no field named {key:?}", manifest.name));
        }
    }

    for field in &manifest.fields {
        let Some(value) = values.get(&field.key) else {
            let already_saved = field.secret && stored_secrets.contains(&field.key);
            let may_be_missing = field.secret && completeness == Completeness::AllowMissingSecrets;
            if field.required && !already_saved && !may_be_missing {
                errors.add(&field.key, format!("{} is required", field.label));
            }
            continue;
        };
        if let Some(pattern) = &field.pattern {
            let anchored = format!("^(?:{})$", pattern.trim_start_matches('^').trim_end_matches('$'));
            let matches = regex::Regex::new(&anchored).map(|re| re.is_match(value)).unwrap_or(false);
            if !matches {
                errors.add(&field.key, format!("{} has an invalid format", field.label));
            }
        }
        if field.totp && totp::validate_secret(value).is_err() {
            errors.add(
                &field.key,
                "This isn't a valid TOTP secret. Copy the text key shown under the QR code (letters A–Z and digits 2–7).",
            );
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Required secret fields with no saved value: the account "needs setup".
/// Required fields an account still lacks before it can log in: secrets with
/// no saved value and plain values that are empty (e.g. an API key that an
/// AutoLogin 1.x account never had). The client ID is stored separately and
/// always present. Keys are in manifest order.
pub fn missing_fields(
    manifest: &BrokerManifest,
    values: &std::collections::BTreeMap<String, String>,
    stored_secrets: &[String],
) -> Vec<String> {
    manifest
        .fields
        .iter()
        .filter(|f| f.required && f.key != "client_id")
        .filter(|f| {
            if f.secret {
                !stored_secrets.contains(&f.key)
            } else {
                values.get(&f.key).is_none_or(|v| v.trim().is_empty())
            }
        })
        .map(|f| f.key.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::registry::ManifestBundle;

    fn upstox() -> BrokerManifest {
        ManifestBundle::bundled().unwrap().get("upstox").unwrap().clone()
    }

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        normalize(&pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }

    fn complete() -> Vec<(&'static str, &'static str)> {
        vec![
            ("client_id", "UP1"),
            ("api_key", "key"),
            ("mobile_number", "9876543210"),
            ("mpin", "123456"),
            ("totp_key", "JBSWY3DPEHPK3PXP"),
        ]
    }

    #[test]
    fn missing_fields_covers_secrets_and_required_plain_values() {
        let fivepaisa = ManifestBundle::bundled().unwrap().get("fivepaisa").unwrap().clone();
        // An account from AutoLogin 1.x: PIN and TOTP saved, but no API key.
        let migrated = missing_fields(&fivepaisa, &BTreeMap::new(), &["mpin".into(), "totp_key".into()]);
        assert_eq!(migrated, vec!["api_key".to_string()]);

        let blank_key: BTreeMap<String, String> = [("api_key".to_string(), " ".to_string())].into();
        assert_eq!(missing_fields(&fivepaisa, &blank_key, &[]), vec!["api_key", "totp_key", "mpin"]);

        let complete: BTreeMap<String, String> = [("api_key".to_string(), "KEY".to_string())].into();
        assert!(missing_fields(&fivepaisa, &complete, &["mpin".into(), "totp_key".into()]).is_empty());
    }

    #[test]
    fn accepts_complete_valid_values() {
        assert_eq!(validate(&upstox(), &values(&complete()), &[], Completeness::Strict), Ok(()));
    }

    #[test]
    fn reports_each_problem_by_field() {
        let errors = validate(
            &upstox(),
            &values(&[("client_id", " "), ("mobile_number", "12345"), ("totp_key", "not base32!"), ("bogus", "x")]),
            &[],
            Completeness::Strict,
        )
        .unwrap_err();
        let keys: Vec<&str> = errors.0.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["api_key", "bogus", "client_id", "mobile_number", "mpin", "totp_key"]);
        assert!(errors.0["totp_key"].contains("TOTP secret"));
    }

    #[test]
    fn saved_secret_satisfies_required_on_edit() {
        let edit: Vec<_> = complete().into_iter().filter(|(k, _)| *k != "mpin").collect();
        assert!(validate(&upstox(), &values(&edit), &[], Completeness::Strict).is_err());
        assert_eq!(validate(&upstox(), &values(&edit), &["mpin".to_string()], Completeness::Strict), Ok(()));
    }

    #[test]
    fn patterns_must_match_whole_value() {
        let mut v = complete();
        v[2] = ("mobile_number", "98765432101");
        assert!(validate(&upstox(), &values(&v), &[], Completeness::Strict).is_err());
    }

    #[test]
    fn imports_may_omit_secrets_but_not_other_required_fields() {
        let no_secrets = values(&[("client_id", "UP1"), ("api_key", "key"), ("mobile_number", "9876543210")]);
        assert_eq!(validate(&upstox(), &no_secrets, &[], Completeness::AllowMissingSecrets), Ok(()));
        assert_eq!(missing_fields(&upstox(), &no_secrets, &[]), vec!["mpin".to_string(), "totp_key".to_string()]);
        let no_api_key = values(&[("client_id", "UP1"), ("mobile_number", "9876543210")]);
        assert!(validate(&upstox(), &no_api_key, &[], Completeness::AllowMissingSecrets).is_err());
    }
}
