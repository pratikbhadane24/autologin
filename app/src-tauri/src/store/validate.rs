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

/// Longest account tag (the user's name for an account; Cirrus's "Account Tag").
pub const MAX_TAG_CHARS: usize = 64;
/// Key under which tag problems are reported, next to the form's Name field.
pub const TAG_FIELD: &str = "tag";

/// Check and tidy an account tag: trimmed, empty means none, at most
/// `MAX_TAG_CHARS` characters and no control characters (tabs, newlines...).
pub fn normalize_tag(tag: Option<&str>) -> Result<Option<String>, String> {
    let Some(tag) = tag.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if tag.chars().count() > MAX_TAG_CHARS {
        return Err(format!("Name can be at most {MAX_TAG_CHARS} characters"));
    }
    if tag.chars().any(char::is_control) {
        return Err("Name can't contain tabs, line breaks or other control characters".into());
    }
    Ok(Some(tag.to_string()))
}

/// How strictly `required` applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    /// Every required field must be present (Add/Edit form).
    Strict,
    /// Anything but the client ID may be missing (imports, Cirrus paste);
    /// the account is saved as "needs setup" and skipped by runs until
    /// completed. Values that are given are still checked.
    AllowMissing,
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
            let may_be_missing = completeness == Completeness::AllowMissing && field.key != "client_id";
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
    fn tags_are_trimmed_limited_and_free_of_control_characters() {
        assert_eq!(normalize_tag(None), Ok(None));
        assert_eq!(normalize_tag(Some("   ")), Ok(None));
        assert_eq!(normalize_tag(Some("  Pratik D ")), Ok(Some("Pratik D".into())));
        let longest = "é".repeat(MAX_TAG_CHARS);
        assert_eq!(normalize_tag(Some(&longest)), Ok(Some(longest.clone())));
        assert!(normalize_tag(Some(&format!("{longest}x"))).unwrap_err().contains("64"));
        assert!(normalize_tag(Some("Pratik\nD")).is_err());
        assert!(normalize_tag(Some("Pratik\tD")).is_err());
        assert!(normalize_tag(Some("Pratik\u{7}D")).is_err());
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
    fn imports_may_omit_anything_but_the_client_id() {
        let no_secrets = values(&[("client_id", "UP1"), ("api_key", "key"), ("mobile_number", "9876543210")]);
        assert_eq!(validate(&upstox(), &no_secrets, &[], Completeness::AllowMissing), Ok(()));
        assert_eq!(missing_fields(&upstox(), &no_secrets, &[]), vec!["mpin".to_string(), "totp_key".to_string()]);
        // Values the user types (mobile number) or Cirrus didn't have (API key)
        // may be added later too; the account shows "Needs setup" until then.
        let only_client = values(&[("client_id", "UP1")]);
        assert_eq!(validate(&upstox(), &only_client, &[], Completeness::AllowMissing), Ok(()));
        assert_eq!(missing_fields(&upstox(), &only_client, &[]), vec!["api_key", "mobile_number", "mpin", "totp_key"]);
        assert!(validate(&upstox(), &only_client, &[], Completeness::Strict).is_err());
        let no_client = values(&[("api_key", "key")]);
        assert!(validate(&upstox(), &no_client, &[], Completeness::AllowMissing).is_err());
        // Whatever is given is still checked.
        let bad_mobile = values(&[("client_id", "UP1"), ("mobile_number", "123")]);
        assert!(validate(&upstox(), &bad_mobile, &[], Completeness::AllowMissing).is_err());
    }
}
