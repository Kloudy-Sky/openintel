//! Pure pieces of the morning brief: which earnings rows are worth a line, and
//! which headlines are "since the prior close". No IO, no clock.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::values::earnings_row::{EarningsRow, ReportSlot};
use crate::domain::values::headline::Headline;

#[derive(Debug, Default, Serialize)]
pub struct EarningsToday {
    pub before_open: Vec<EarningsRow>,
    pub after_close: Vec<EarningsRow>,
    pub unspecified: Vec<EarningsRow>,
    /// Rows dropped for being under the market-cap floor and not on the watch list.
    pub below_floor: usize,
    /// Rows dropped because the provider gave no market cap and the symbol is
    /// not on the watch list — unverifiable, so not shown as large.
    pub cap_unknown: usize,
}

/// Keep watched symbols and anything at or above `floor_cap`; bucket by slot,
/// largest cap first. Everything else is counted, never silently dropped.
pub fn split_earnings(rows: Vec<EarningsRow>, watch: &[String], floor_cap: f64) -> EarningsToday {
    let mut out = EarningsToday::default();
    for row in rows {
        let watched = watch.iter().any(|w| w.eq_ignore_ascii_case(&row.symbol));
        match (watched, row.market_cap_usd) {
            (true, _) => {}
            (false, Some(cap)) if cap >= floor_cap => {}
            (false, Some(_)) => {
                out.below_floor += 1;
                continue;
            }
            (false, None) => {
                out.cap_unknown += 1;
                continue;
            }
        }
        match row.slot {
            ReportSlot::BeforeOpen => out.before_open.push(row),
            ReportSlot::AfterClose => out.after_close.push(row),
            ReportSlot::Unspecified => out.unspecified.push(row),
        }
    }
    for bucket in [
        &mut out.before_open,
        &mut out.after_close,
        &mut out.unspecified,
    ] {
        bucket.sort_by(|a, b| {
            b.market_cap_usd
                .unwrap_or(0.0)
                .partial_cmp(&a.market_cap_usd.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    out
}

/// Headlines published at or after `since`, plus how many carried no
/// timestamp (kept out of the list but never forgotten: they might be recent).
pub fn headline_window(headlines: &[Headline], since: DateTime<Utc>) -> (Vec<Headline>, usize) {
    let mut recent: Vec<Headline> = headlines
        .iter()
        .filter(|h| h.published_at.is_some_and(|t| t >= since))
        .cloned()
        .collect();
    recent.sort_by_key(|h| std::cmp::Reverse(h.published_at));
    let undated = headlines
        .iter()
        .filter(|h| h.published_at.is_none())
        .count();
    (recent, undated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn row(symbol: &str, slot: ReportSlot, cap: Option<f64>) -> EarningsRow {
        EarningsRow {
            symbol: symbol.into(),
            company: format!("{symbol} Inc"),
            slot,
            market_cap_usd: cap,
            eps_forecast: None,
            fiscal_quarter: None,
        }
    }

    #[test]
    fn watched_names_survive_the_floor_and_the_rest_are_counted() {
        let rows = vec![
            row("BIG", ReportSlot::AfterClose, Some(30e9)),
            row("HUGE", ReportSlot::AfterClose, Some(90e9)),
            row("TINY", ReportSlot::BeforeOpen, Some(50e6)),
            row("MINE", ReportSlot::BeforeOpen, Some(50e6)),
            row("MYST", ReportSlot::Unspecified, None),
        ];
        let out = split_earnings(rows, &["mine".into()], 500e6);
        assert_eq!(
            out.after_close
                .iter()
                .map(|r| r.symbol.as_str())
                .collect::<Vec<_>>(),
            ["HUGE", "BIG"]
        );
        assert_eq!(out.before_open[0].symbol, "MINE");
        assert!(out.unspecified.is_empty());
        assert_eq!(out.below_floor, 1);
        assert_eq!(out.cap_unknown, 1);
    }

    #[test]
    fn window_keeps_recent_dated_and_counts_undated() {
        let t = |h: u32| Utc.with_ymd_and_hms(2026, 9, 8, h, 0, 0).unwrap();
        let hl = |title: &str, at: Option<DateTime<Utc>>| Headline {
            title: title.into(),
            publisher: "p".into(),
            published_at: at,
        };
        let headlines = vec![
            hl("old", Some(t(1))),
            hl("new", Some(t(10))),
            hl("newer", Some(t(12))),
            hl("undated", None),
        ];
        let (recent, undated) = headline_window(&headlines, t(9));
        assert_eq!(
            recent.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(),
            ["newer", "new"]
        );
        assert_eq!(undated, 1);
    }
}
