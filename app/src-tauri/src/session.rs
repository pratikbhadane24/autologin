//! Broker session expiry.
//!
//! Indian brokers invalidate API sessions once a day at a fixed local time
//! (usually early morning IST). v1 compared the *hour of the login* with 5,
//! so a 03:00 login never expired and a 06:00 login expired at midnight.
//! v2 instead asks: has the first reset after the login already happened?

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

const DEFAULT_RESET_HOUR: u32 = 5;

/// When a broker's sessions are reset. Comes from the manifest `[session]` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPolicy {
    /// Local wall-clock time at which sessions are invalidated.
    pub reset: NaiveTime,
    /// Timezone `reset` is expressed in.
    pub tz: Tz,
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self {
            reset: NaiveTime::from_hms_opt(DEFAULT_RESET_HOUR, 0, 0).expect("valid constant time"),
            tz: chrono_tz::Asia::Kolkata,
        }
    }
}

/// The first session reset that happens strictly after `last_login`.
///
/// Every calendar day counts as a reset day, weekends included: brokers do
/// expire tokens on non-trading days, and expiring too eagerly only costs one
/// extra login, while expiring too late would show a dead session as live.
pub fn next_reset_after(last_login: DateTime<Utc>, policy: &SessionPolicy) -> DateTime<Utc> {
    let local_date = last_login.with_timezone(&policy.tz).date_naive();
    let same_day = reset_on(local_date, policy);
    if same_day > last_login {
        return same_day;
    }
    let next_date = local_date.succ_opt().expect("date within chrono's range");
    reset_on(next_date, policy)
}

/// The reset instant on `date`. If a DST change makes that wall-clock time
/// ambiguous the earlier instant wins; if DST skips it, fall back to reading
/// it as UTC so there is still an answer (IST itself has no DST).
fn reset_on(date: NaiveDate, policy: &SessionPolicy) -> DateTime<Utc> {
    let wall_clock = date.and_time(policy.reset);
    policy
        .tz
        .from_local_datetime(&wall_clock)
        .earliest()
        .map(|local| local.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&wall_clock))
}

/// A session is expired once `now` has reached the first reset after login.
pub fn is_expired(last_login: DateTime<Utc>, now: DateTime<Utc>, policy: &SessionPolicy) -> bool {
    now >= next_reset_after(last_login, policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a UTC instant from an IST wall-clock time.
    fn ist(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .expect("unambiguous IST time")
            .with_timezone(&Utc)
    }

    #[test]
    fn early_morning_login_expires_at_same_day_reset() {
        // v1 bug: a 03:00 login never expired.
        let policy = SessionPolicy::default();
        let login = ist(2026, 10, 5, 3, 0);
        assert!(!is_expired(login, ist(2026, 10, 5, 4, 59), &policy));
        assert!(is_expired(login, ist(2026, 10, 5, 5, 0), &policy));
    }

    #[test]
    fn daytime_login_survives_midnight_until_next_reset() {
        // v1 bug: a 06:00 login expired right after midnight.
        let policy = SessionPolicy::default();
        let login = ist(2026, 10, 5, 6, 0);
        assert!(!is_expired(login, ist(2026, 10, 6, 0, 30), &policy));
        assert!(!is_expired(login, ist(2026, 10, 6, 4, 59), &policy));
        assert!(is_expired(login, ist(2026, 10, 6, 5, 0), &policy));
    }

    #[test]
    fn login_exactly_at_reset_lasts_a_full_day() {
        let policy = SessionPolicy::default();
        let login = ist(2026, 10, 5, 5, 0);
        assert_eq!(next_reset_after(login, &policy), ist(2026, 10, 6, 5, 0));
    }

    #[test]
    fn late_night_login_expires_next_morning() {
        let policy = SessionPolicy::default();
        assert_eq!(next_reset_after(ist(2026, 10, 5, 23, 50), &policy), ist(2026, 10, 6, 5, 0));
    }

    #[test]
    fn reset_time_comes_from_policy() {
        let policy = SessionPolicy {
            reset: NaiveTime::from_hms_opt(7, 30, 0).unwrap(),
            tz: chrono_tz::Asia::Kolkata,
        };
        let login = ist(2026, 10, 5, 6, 0);
        assert_eq!(next_reset_after(login, &policy), ist(2026, 10, 5, 7, 30));
    }
}
