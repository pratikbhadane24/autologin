//! Failure screenshots and page HTML, kept locally for 7 days.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const FAILURES_DIR: &str = "failures";
pub const KEEP_DAYS: u64 = 7;
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

/// `<data>/failures/<YYYY-MM-DD>/<broker>_<client>_<HHMMSS>` (no extension).
pub fn failure_stem(data_dir: &Path, broker_id: &str, client_id: &str, now: chrono::DateTime<chrono::Local>) -> PathBuf {
    let safe = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect::<String>();
    data_dir
        .join(FAILURES_DIR)
        .join(now.format("%Y-%m-%d").to_string())
        .join(format!("{}_{}_{}", safe(broker_id), safe(client_id), now.format("%H%M%S")))
}

/// Delete dated failure folders older than `KEEP_DAYS`. Errors are logged,
/// never fatal.
pub fn prune(data_dir: &Path, now: SystemTime) {
    let root = data_dir.join(FAILURES_DIR);
    let Ok(entries) = std::fs::read_dir(&root) else { return };
    let max_age = Duration::from_secs(KEEP_DAYS * SECONDS_PER_DAY);
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > max_age);
        if old {
            if let Err(error) = std::fs::remove_dir_all(entry.path()) {
                tracing::warn!(%error, "could not delete old failure screenshots");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn stem_is_dated_and_filesystem_safe() {
        let now = chrono::Local.with_ymd_and_hms(2026, 10, 5, 8, 45, 3).unwrap();
        let stem = failure_stem(Path::new("/d"), "zerodha", "AB/1 x", now);
        assert_eq!(stem, PathBuf::from("/d/failures/2026-10-05/zerodha_AB_1_x_084503"));
    }

    #[test]
    fn prune_removes_only_old_folders() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join(FAILURES_DIR).join("2026-10-05");
        std::fs::create_dir_all(&day).unwrap();
        prune(dir.path(), SystemTime::now());
        assert!(day.exists());
        prune(dir.path(), SystemTime::now() + Duration::from_secs((KEEP_DAYS + 1) * SECONDS_PER_DAY));
        assert!(!day.exists());
    }
}
