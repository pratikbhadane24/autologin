//! Tolerant timestamp parsing, ported from v1 `utils/datetime_utils.py`.
//!
//! v2 always stores `CANONICAL_FORMAT`, but timestamps still arrive from v1
//! `accounts.json` and from CSV files that a spreadsheet has rewritten in the
//! machine's locale (e.g. `13-07-2026 09:29`). Parsing never fails hard:
//! callers treat `None` as "no usable timestamp".

use chrono::{NaiveDate, NaiveDateTime};

pub const CANONICAL_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// Tried in order. Day-first variants precede month-first ones because the app
/// targets Indian brokers, where a spreadsheet writes 05/07/2026 as 5 July.
const DATETIME_FORMATS: &[&str] = &[
    CANONICAL_FORMAT,
    "%Y-%m-%d %H:%M",
    "%Y-%m-%dT%H:%M:%S",
    "%Y-%m-%dT%H:%M",
    "%Y/%m/%d %H:%M:%S",
    "%Y/%m/%d %H:%M",
    "%d-%m-%Y %H:%M:%S",
    "%d-%m-%Y %H:%M",
    "%d/%m/%Y %H:%M:%S",
    "%d/%m/%Y %H:%M",
    "%d-%m-%y %H:%M:%S",
    "%d-%m-%y %H:%M",
    "%d/%m/%y %H:%M:%S",
    "%d/%m/%y %H:%M",
    "%m/%d/%Y %H:%M:%S",
    "%m/%d/%Y %H:%M",
];

const DATE_FORMATS: &[&str] = &["%Y-%m-%d", "%d-%m-%Y", "%d/%m/%Y", "%m/%d/%Y"];

const NULL_MARKERS: &[&str] = &["nan", "nat", "none", "null"];

/// chrono's `%Y` accepts any digit count, unlike Python's 4-digit `%Y`, so
/// `13-07-26` would otherwise parse as year 26. Reject such years.
const MIN_FOUR_DIGIT_YEAR: i32 = 1000;

fn plausible(parsed: NaiveDateTime, format: &str) -> bool {
    !format.contains("%Y") || chrono::Datelike::year(&parsed) >= MIN_FOUR_DIGIT_YEAR
}

/// Parse `value` with the accepted formats, returning `None` if none match.
pub fn parse_datetime(value: &str) -> Option<NaiveDateTime> {
    let text = value.trim();
    if text.is_empty() || NULL_MARKERS.contains(&text.to_ascii_lowercase().as_str()) {
        return None;
    }

    let full = DATETIME_FORMATS.iter().find_map(|format| {
        NaiveDateTime::parse_from_str(text, format)
            .ok()
            .filter(|parsed| plausible(*parsed, format))
    });

    full.or_else(|| {
        DATE_FORMATS.iter().find_map(|format| {
            NaiveDate::parse_from_str(text, format)
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .filter(|parsed| plausible(*parsed, format))
        })
    })
}

/// Rewrite `value` in `CANONICAL_FORMAT`, or return an empty string.
pub fn normalize_datetime(value: &str) -> String {
    parse_datetime(value)
        .map(|parsed| parsed.format(CANONICAL_FORMAT).to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, mi, s)
            .unwrap()
    }

    #[test]
    fn parses_excel_day_first_value() {
        assert_eq!(parse_datetime("13-07-2026 09:29"), Some(dt(2026, 7, 13, 9, 29, 0)));
    }

    #[test]
    fn normalizes_excel_value_to_canonical() {
        assert_eq!(normalize_datetime("13-07-2026 09:29"), "2026-07-13 09:29:00");
    }

    #[test]
    fn canonical_round_trips() {
        assert_eq!(normalize_datetime("2026-07-13 09:29:00"), "2026-07-13 09:29:00");
    }

    #[test]
    fn parses_common_variants() {
        let expected = dt(2026, 7, 13, 9, 29, 0);
        assert_eq!(parse_datetime("13/07/2026 09:29"), Some(expected));
        assert_eq!(parse_datetime("2026-07-13T09:29:00"), Some(expected));
        assert_eq!(parse_datetime("  13-07-2026 09:29  "), Some(expected));
        assert_eq!(parse_datetime("13-07-2026 09:29:07"), Some(dt(2026, 7, 13, 9, 29, 7)));
    }

    #[test]
    fn parses_date_only_as_midnight() {
        assert_eq!(parse_datetime("2026-07-13"), Some(dt(2026, 7, 13, 0, 0, 0)));
    }

    #[test]
    fn ambiguous_dates_are_day_first() {
        assert_eq!(parse_datetime("05-07-2026 09:29"), Some(dt(2026, 7, 5, 9, 29, 0)));
    }

    #[test]
    fn two_digit_years_are_not_read_as_year_zero_something() {
        assert_eq!(parse_datetime("13-07-26 09:29"), Some(dt(2026, 7, 13, 9, 29, 0)));
    }

    #[test]
    fn rejects_junk_and_null_markers() {
        for junk in ["", "   ", "nan", "NaT", "None", "null", "yesterday", "32-13-2026"] {
            assert_eq!(parse_datetime(junk), None, "input {junk:?}");
        }
        assert_eq!(normalize_datetime("garbage"), "");
    }
}
