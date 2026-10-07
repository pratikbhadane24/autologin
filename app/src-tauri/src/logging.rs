//! Logging: daily rolling files in the app's log folder plus stderr in dev.
//!
//! Secrets must never reach a log line. Code logs account *fields* only by
//! name, and anything that can echo user input (broker error text, URLs,
//! page snippets) goes through a `Redactor` first.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

pub const LOG_FILE_PREFIX: &str = "autologin";
pub const LOG_FILE_SUFFIX: &str = "log";
pub const KEEP_LOG_FILES: usize = 14;
const DEFAULT_FILTER: &str = "info,autologin_lib=debug,chromiumoxide=warn";
const MASK: &str = "******";
/// Shorter values (e.g. a 4-digit PIN) are still masked; this only avoids
/// masking empty strings or single characters that would mangle every line.
const MIN_SECRET_LEN: usize = 3;

/// Install the global subscriber. Keep the returned guard alive for the
/// lifetime of the app, or buffered lines are lost on exit.
pub fn init(log_dir: &Path) -> Result<WorkerGuard, Box<dyn std::error::Error>> {
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .max_log_files(KEEP_LOG_FILES)
        .build(log_dir)?;
    let (file_writer, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_env("AUTOLOGIN_LOG").unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let file_layer = fmt::layer().with_writer(file_writer).with_ansi(false).with_target(false);
    let console_layer = cfg!(debug_assertions).then(|| fmt::layer().with_target(false).compact());

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .try_init()?;
    Ok(guard)
}

/// Masks known secret values inside arbitrary text.
#[derive(Debug, Default, Clone)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub fn new<I, S>(secrets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut secrets: Vec<String> = secrets
            .into_iter()
            .map(|s| s.as_ref().trim().to_string())
            .filter(|s| s.chars().count() >= MIN_SECRET_LEN)
            .collect();
        // Longest first, so a secret containing another is masked whole.
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        secrets.dedup();
        Self { secrets }
    }

    /// Also mask a value generated at runtime, such as a TOTP code.
    pub fn with(mut self, value: impl AsRef<str>) -> Self {
        let mut all = std::mem::take(&mut self.secrets);
        all.push(value.as_ref().to_string());
        Self::new(all)
    }

    pub fn apply(&self, text: &str) -> String {
        self.secrets.iter().fold(text.to_string(), |acc, secret| {
            let encoded =
                percent_encoding::utf8_percent_encode(secret, percent_encoding::NON_ALPHANUMERIC).to_string();
            acc.replace(secret.as_str(), MASK).replace(&encoded, MASK)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_secrets_including_url_encoded_forms() {
        let redactor = Redactor::new(["p@ss word", "1234"]);
        let text = "login?pw=p%40ss%20word&pin=1234 failed for p@ss word";
        assert_eq!(redactor.apply(text), "login?pw=******&pin=****** failed for ******");
    }

    #[test]
    fn longer_secret_wins_over_contained_one() {
        let redactor = Redactor::new(["abc", "abcdef"]);
        assert_eq!(redactor.apply("x abcdef y"), "x ****** y");
    }

    #[test]
    fn ignores_empty_and_tiny_values_and_adds_runtime_values() {
        let redactor = Redactor::new(["", "a", "  "]).with("482913");
        assert_eq!(redactor.apply("a code 482913"), "a code ******");
    }
}
