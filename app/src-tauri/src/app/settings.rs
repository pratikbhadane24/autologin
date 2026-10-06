//! User preferences, stored as one JSON value in the settings table.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::runner::{default_concurrency, DEFAULT_RETRIES};
use crate::scheduler::Schedule;
use crate::store::settings;

const KEY: &str = "app";
pub const MAX_RETRIES: u32 = 3;
pub const MAX_CONCURRENCY: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Hide the browser on manual runs. Scheduled runs use `schedule.headless`.
    pub manual_headless: bool,
    pub schedule: Schedule,
    pub retries: u32,
    pub concurrency: usize,
    pub start_with_computer: bool,
    /// Install app updates automatically (never during or just before a run).
    pub auto_update: bool,
    /// Set once the user has read the "how AutoLogin handles your data" screen.
    pub privacy_acknowledged: bool,
    /// Last version whose What's New the user saw.
    pub last_seen_version: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            manual_headless: false,
            schedule: Schedule::default(),
            retries: DEFAULT_RETRIES,
            concurrency: default_concurrency(),
            start_with_computer: false,
            auto_update: true,
            privacy_acknowledged: false,
            last_seen_version: None,
        }
    }
}

impl AppSettings {
    /// Clamp values a hand-edited database or old client could send.
    pub fn sanitized(self) -> Self {
        Self { retries: self.retries.min(MAX_RETRIES), concurrency: self.concurrency.clamp(1, MAX_CONCURRENCY), ..self }
    }
}

pub fn load(conn: &Connection) -> AppSettings {
    settings::get::<AppSettings>(conn, KEY).ok().flatten().unwrap_or_default().sanitized()
}

pub fn save(conn: &Connection, value: &AppSettings) -> rusqlite::Result<AppSettings> {
    let clean = value.clone().sanitized();
    settings::set(conn, KEY, &clean)?;
    Ok(clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_then_round_trip_with_clamping() {
        let conn = crate::store::db::open_in_memory().unwrap();
        let defaults = load(&conn);
        assert!(!defaults.manual_headless && defaults.schedule.headless);

        let saved = save(&conn, &AppSettings { retries: 99, concurrency: 0, ..defaults }).unwrap();
        assert_eq!((saved.retries, saved.concurrency), (MAX_RETRIES, 1));
        assert_eq!(load(&conn), saved);
    }
}
