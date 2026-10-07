//! `{placeholder}` templates used in broker manifests.
//!
//! Syntax: `{key}` or `{filter:...:key}`, where `key` is a bare account field
//! (`client_id`), a namespaced value (`consts.api_key`, `tenant.cirrus_app`,
//! `vars.consent_id`) or the special `totp`. Filters: `b64`, `url`, `json`
//! (a quoted JSON string, for request bodies); they
//! apply right to left, so `{url:b64:client_id}` is `url(b64(client_id))`.
//! `{{` and `}}` produce literal braces.

use base64::Engine;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TemplateError {
    #[error("unclosed '{{' in template {0:?}")]
    Unclosed(String),
    #[error("unknown filter {filter:?} in template {template:?}")]
    UnknownFilter { filter: String, template: String },
    #[error("unknown placeholder {{{0}}}")]
    Unresolved(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Base64,
    Url,
    /// A JSON string literal, quotes included, for JSON request bodies.
    Json,
}

impl Filter {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "b64" => Some(Self::Base64),
            "url" => Some(Self::Url),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    fn apply(self, value: &str) -> String {
        match self {
            Self::Base64 => base64::engine::general_purpose::STANDARD.encode(value),
            Self::Url => {
                percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
                    .to_string()
            }
            Self::Json => serde_json::Value::from(value).to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Literal(String),
    /// `filters` are stored in application order (innermost first).
    Placeholder { key: String, filters: Vec<Filter> },
}

/// Split a template into literal and placeholder segments.
pub fn parse(template: &str) -> Result<Vec<Segment>, TemplateError> {
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '{' => {
                let mut inner = String::new();
                let mut closed = false;
                for ch in chars.by_ref() {
                    if ch == '}' {
                        closed = true;
                        break;
                    }
                    inner.push(ch);
                }
                if !closed {
                    return Err(TemplateError::Unclosed(template.to_string()));
                }
                if !literal.is_empty() {
                    segments.push(Segment::Literal(std::mem::take(&mut literal)));
                }
                segments.push(parse_placeholder(&inner, template)?);
            }
            other => literal.push(other),
        }
    }
    if !literal.is_empty() {
        segments.push(Segment::Literal(literal));
    }
    Ok(segments)
}

fn parse_placeholder(inner: &str, template: &str) -> Result<Segment, TemplateError> {
    let mut parts: Vec<&str> = inner.split(':').map(str::trim).collect();
    let key = parts.pop().unwrap_or_default().to_string();
    let filters = parts
        .into_iter()
        .rev()
        .map(|name| {
            Filter::parse(name).ok_or_else(|| TemplateError::UnknownFilter {
                filter: name.to_string(),
                template: template.to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Segment::Placeholder { key, filters })
}

/// Every placeholder key referenced by `template`.
pub fn keys(template: &str) -> Result<Vec<String>, TemplateError> {
    Ok(parse(template)?
        .into_iter()
        .filter_map(|segment| match segment {
            Segment::Placeholder { key, .. } => Some(key),
            Segment::Literal(_) => None,
        })
        .collect())
}

/// Render `template`, resolving each key through `lookup`.
pub fn render<F>(template: &str, mut lookup: F) -> Result<String, TemplateError>
where
    F: FnMut(&str) -> Option<String>,
{
    parse(template)?
        .into_iter()
        .map(|segment| match segment {
            Segment::Literal(text) => Ok(text),
            Segment::Placeholder { key, filters } => {
                let value = lookup(&key).ok_or(TemplateError::Unresolved(key))?;
                Ok(filters.iter().fold(value, |acc, filter| filter.apply(&acc)))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_from(pairs: &[(&str, &str)]) -> impl FnMut(&str) -> Option<String> {
        let map: HashMap<String, String> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn json_filter_makes_a_quoted_escaped_string() {
        let out = render(r#"{{"id":{json:client_id}}}"#, lookup_from(&[("client_id", r#"a"b\c"#)])).unwrap();
        assert_eq!(out, r#"{"id":"a\"b\\c"}"#);
    }

    #[test]
    fn renders_plain_and_namespaced_keys() {
        let out = render(
            "{tenant.cirrus_base}/x?api_key={consts.api_key}&state={client_id}",
            lookup_from(&[
                ("tenant.cirrus_base", "https://cirrus.trade"),
                ("consts.api_key", "oS35ILQ1"),
                ("client_id", "AB123"),
            ]),
        );
        assert_eq!(out.unwrap(), "https://cirrus.trade/x?api_key=oS35ILQ1&state=AB123");
    }

    #[test]
    fn applies_filters() {
        let out = render("{b64:client_id}|{url:pw}", lookup_from(&[("client_id", "AB123"), ("pw", "a&b c")]));
        assert_eq!(out.unwrap(), "QUIxMjM=|a%26b%20c");
    }

    #[test]
    fn chains_filters_right_to_left() {
        // base64("AB12") = "QUIxMg==", whose '=' must then be url-encoded.
        let out = render("{url:b64:client_id}", lookup_from(&[("client_id", "AB12")]));
        assert_eq!(out.unwrap(), "QUIxMg%3D%3D");
    }

    #[test]
    fn escaped_braces_are_literal() {
        assert_eq!(render("{{x}}", lookup_from(&[])).unwrap(), "{x}");
    }

    #[test]
    fn reports_unresolved_key() {
        let err = render("{missing}", lookup_from(&[])).unwrap_err();
        assert_eq!(err, TemplateError::Unresolved("missing".into()));
    }

    #[test]
    fn rejects_unknown_filter_and_unclosed_brace() {
        assert!(matches!(parse("{zip:x}"), Err(TemplateError::UnknownFilter { .. })));
        assert!(matches!(parse("abc {client_id"), Err(TemplateError::Unclosed(_))));
    }

    #[test]
    fn lists_keys() {
        assert_eq!(keys("a{x}b{url:y}").unwrap(), vec!["x".to_string(), "y".to_string()]);
    }
}
