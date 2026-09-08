//! Vendored schedule of scheduled macro releases (CPI, jobs, FOMC, GDP, PCE),
//! the same shape as the holiday table in `clock`: data with an explicit
//! coverage window, refreshed yearly. Outside the window the answer is None,
//! never an empty day.

use std::sync::OnceLock;

use chrono::NaiveDate;
use serde::Deserialize;

use crate::domain::values::macro_release::MacroRelease;

const DATA: &str = include_str!("macro_calendar_2026.json");

#[derive(Debug, Deserialize)]
struct Coverage {
    from: NaiveDate,
    to: NaiveDate,
}

#[derive(Debug, Deserialize)]
struct Calendar {
    coverage: Coverage,
    note: String,
    releases: Vec<MacroRelease>,
}

fn calendar() -> &'static Calendar {
    static CAL: OnceLock<Calendar> = OnceLock::new();
    CAL.get_or_init(|| serde_json::from_str(DATA).expect("vendored macro calendar is valid JSON"))
}

pub fn coverage() -> (NaiveDate, NaiveDate) {
    let c = &calendar().coverage;
    (c.from, c.to)
}

/// The provenance caveat that ships with the data.
pub fn coverage_note() -> &'static str {
    &calendar().note
}

/// Releases scheduled on `date`, or None when the date is outside coverage.
pub fn releases_on(date: NaiveDate) -> Option<Vec<MacroRelease>> {
    let (from, to) = coverage();
    if date < from || date > to {
        return None;
    }
    Some(
        calendar()
            .releases
            .iter()
            .filter(|r| r.date == date)
            .cloned()
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn every_vendored_release_falls_inside_coverage_and_is_sorted() {
        let (from, to) = coverage();
        let releases = &calendar().releases;
        assert!(!releases.is_empty());
        assert!(releases.iter().all(|r| r.date >= from && r.date <= to));
        assert!(releases.windows(2).all(|w| w[0].date <= w[1].date));
    }

    #[test]
    fn a_cpi_day_and_a_quiet_day() {
        let cpi = releases_on(ymd(2026, 9, 11)).unwrap();
        assert_eq!(cpi.len(), 1);
        assert!(cpi[0].release.starts_with("CPI"));
        assert_eq!(cpi[0].time_et, "08:30");
        assert!(releases_on(ymd(2026, 9, 8)).unwrap().is_empty());
    }

    #[test]
    fn outside_coverage_is_none_not_empty() {
        assert!(releases_on(ymd(2027, 1, 5)).is_none());
        assert!(releases_on(ymd(2025, 12, 31)).is_none());
        assert!(coverage_note().contains("BEA"));
    }
}
