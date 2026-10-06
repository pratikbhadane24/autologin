//! Daily auto-run schedule.
//!
//! Users pick the time and days; 08:45 IST on weekdays is the default and
//! the recommendation shown in Settings.

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

pub const RECOMMENDED_HOUR: u32 = 8;
pub const RECOMMENDED_MINUTE: u32 = 45;

/// Shown next to the time picker in Settings.
pub const RECOMMENDATION: &str = "Recommended: 8:45 AM. Brokers have finished their overnight \
session reset by then, and your accounts are ready before the 9:00 AM pre-open.";

/// A run missed while the computer was asleep or off is still started on wake,
/// but only within this many minutes of the scheduled time.
pub const CATCH_UP_WINDOW_MINUTES: i64 = 6 * 60;

const TRADING_DAYS: [Weekday; 5] = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri];
const DAYS_TO_SCAN: u32 = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub enabled: bool,
    pub time: NaiveTime,
    pub days: Vec<Weekday>,
    pub tz: Tz,
    /// Re-run accounts that failed, this many minutes after the main run.
    pub retry_failed_after_minutes: Option<u32>,
    /// Scheduled runs hide the browser unless the user turns this off.
    pub headless: bool,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            enabled: false,
            time: NaiveTime::from_hms_opt(RECOMMENDED_HOUR, RECOMMENDED_MINUTE, 0).expect("valid constant time"),
            days: TRADING_DAYS.to_vec(),
            tz: chrono_tz::Asia::Kolkata,
            retry_failed_after_minutes: Some(5),
            headless: true,
        }
    }
}

impl Schedule {
    fn runs_on(&self, date: NaiveDate) -> bool {
        self.days.contains(&date.weekday())
    }

    fn instant_on(&self, date: NaiveDate) -> Option<DateTime<Utc>> {
        self.tz
            .from_local_datetime(&date.and_time(self.time))
            .earliest()
            .map(|local| local.with_timezone(&Utc))
    }

    /// The next scheduled run strictly after `now`, or `None` when disabled
    /// or no days are selected.
    pub fn next_run_after(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if !self.enabled {
            return None;
        }
        let today = now.with_timezone(&self.tz).date_naive();
        today
            .iter_days()
            .take(DAYS_TO_SCAN as usize)
            .filter(|date| self.runs_on(*date))
            .filter_map(|date| self.instant_on(date))
            .find(|instant| *instant > now)
    }

    /// The most recent scheduled run at or before `now` that has not happened
    /// yet (no run since it) and is still within the catch-up window.
    pub fn missed_run(&self, now: DateTime<Utc>, last_run: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
        if !self.enabled {
            return None;
        }
        let today = now.with_timezone(&self.tz).date_naive();
        let due = self.instant_on(today).filter(|due| self.runs_on(today) && *due <= now)?;
        let within_window = (now - due).num_minutes() <= CATCH_UP_WINDOW_MINUTES;
        let already_ran = last_run.is_some_and(|last| last >= due);
        (within_window && !already_ran).then_some(due)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ist(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    fn enabled() -> Schedule {
        Schedule { enabled: true, ..Schedule::default() }
    }

    #[test]
    fn default_is_off_at_recommended_time_on_weekdays() {
        let schedule = Schedule::default();
        assert!(!schedule.enabled);
        assert!(schedule.headless, "scheduled runs are headless by default");
        assert_eq!(schedule.time, NaiveTime::from_hms_opt(8, 45, 0).unwrap());
        assert_eq!(schedule.days.len(), 5);
        assert!(RECOMMENDATION.contains("8:45"));
    }

    #[test]
    fn next_run_is_today_before_time_and_tomorrow_after() {
        // 2026-10-05 is a Monday.
        let schedule = enabled();
        assert_eq!(schedule.next_run_after(ist(2026, 10, 5, 7, 0)), Some(ist(2026, 10, 5, 8, 45)));
        assert_eq!(schedule.next_run_after(ist(2026, 10, 5, 8, 45)), Some(ist(2026, 10, 6, 8, 45)));
    }

    #[test]
    fn friday_evening_skips_weekend() {
        let schedule = enabled();
        assert_eq!(schedule.next_run_after(ist(2026, 10, 9, 18, 0)), Some(ist(2026, 10, 12, 8, 45)));
    }

    #[test]
    fn user_chosen_time_and_days_are_respected() {
        let schedule = Schedule {
            time: NaiveTime::from_hms_opt(7, 30, 0).unwrap(),
            days: vec![Weekday::Sat],
            ..enabled()
        };
        assert_eq!(schedule.next_run_after(ist(2026, 10, 5, 9, 0)), Some(ist(2026, 10, 10, 7, 30)));
    }

    #[test]
    fn disabled_or_no_days_never_runs() {
        assert_eq!(Schedule::default().next_run_after(ist(2026, 10, 5, 7, 0)), None);
        let no_days = Schedule { days: vec![], ..enabled() };
        assert_eq!(no_days.next_run_after(ist(2026, 10, 5, 7, 0)), None);
    }

    #[test]
    fn catches_up_after_sleep_once_within_window() {
        let schedule = enabled();
        let woke = ist(2026, 10, 5, 9, 30);
        assert_eq!(schedule.missed_run(woke, None), Some(ist(2026, 10, 5, 8, 45)));
        assert_eq!(schedule.missed_run(woke, Some(ist(2026, 10, 5, 8, 46))), None);
        assert_eq!(schedule.missed_run(ist(2026, 10, 5, 16, 0), None), None);
        assert_eq!(schedule.missed_run(ist(2026, 10, 10, 9, 30), None), None); // Saturday
    }
}
