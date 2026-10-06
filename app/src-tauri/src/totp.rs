//! TOTP codes, compatible with pyotp's defaults (SHA1, 6 digits, 30 s).

use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;
use totp_rs::{Algorithm, Secret, TOTP};

const DIGITS: usize = 6;
const STEP_SECONDS: u64 = 30;
const SKEW: u8 = 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TotpError {
    #[error("TOTP secret is not valid base32")]
    InvalidSecret,
}

/// Users paste secrets with spaces, lowercase or `=` padding; pyotp accepts
/// all of these, so accept them too.
fn normalize_secret(secret: &str) -> String {
    secret
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '=' && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn build(secret: &str) -> Result<TOTP, TotpError> {
    let bytes = Secret::Encoded(normalize_secret(secret))
        .to_bytes()
        .map_err(|_| TotpError::InvalidSecret)?;
    // new_unchecked: broker secrets are often shorter than RFC's 128-bit minimum.
    Ok(TOTP::new_unchecked(Algorithm::SHA1, DIGITS, SKEW, STEP_SECONDS, bytes))
}

pub fn validate_secret(secret: &str) -> Result<(), TotpError> {
    build(secret).map(|_| ())
}

/// The code for `unix_seconds`.
pub fn code_at(secret: &str, unix_seconds: u64) -> Result<String, TotpError> {
    Ok(build(secret)?.generate(unix_seconds))
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

pub fn current_code(secret: &str) -> Result<String, TotpError> {
    code_at(secret, now_seconds())
}

/// Seconds until the current code rolls over. Callers about to submit a code
/// with very little time left should wait for the next window.
pub fn seconds_remaining() -> u64 {
    STEP_SECONDS - now_seconds() % STEP_SECONDS
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6238 SHA1 seed "12345678901234567890" in base32.
    const RFC_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn matches_rfc6238_vectors() {
        // RFC 8-digit values 94287082 / 07081804, truncated to 6 digits.
        assert_eq!(code_at(RFC_SECRET, 59).unwrap(), "287082");
        assert_eq!(code_at(RFC_SECRET, 1_111_111_109).unwrap(), "081804");
    }

    #[test]
    fn accepts_spaced_lowercase_padded_secret() {
        let messy = "gezd gnbv gy3t qojq gezd gnbv gy3t qojq==";
        assert_eq!(code_at(messy, 59).unwrap(), code_at(RFC_SECRET, 59).unwrap());
    }

    #[test]
    fn rejects_non_base32_secret() {
        assert_eq!(validate_secret("not!base32"), Err(TotpError::InvalidSecret));
    }

    #[test]
    fn seconds_remaining_is_within_window() {
        assert!((1..=STEP_SECONDS).contains(&seconds_remaining()));
    }
}
