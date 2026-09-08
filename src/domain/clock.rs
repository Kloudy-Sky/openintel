//! Market clock: classify an instant into a US equity session state. Pure —
//! `now` is injected. NYSE holidays and early closes are a vendored table with
//! an explicit coverage window; a date outside it is `unknown`, never a guess.

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::America::New_York;
use serde::Serialize;

use crate::domain::values::asset_class::AssetClass;

const OPEN: NaiveTime = NaiveTime::from_hms_opt(9, 30, 0).unwrap();
const FX_ROLL: NaiveTime = NaiveTime::from_hms_opt(17, 0, 0).unwrap();
const CLOSE: NaiveTime = NaiveTime::from_hms_opt(16, 0, 0).unwrap();
const EARLY_CLOSE: NaiveTime = NaiveTime::from_hms_opt(13, 0, 0).unwrap();

/// Full-day NYSE closures. Refresh yearly; the coverage window below must move with it.
const HOLIDAYS: &[(i32, u32, u32, &str)] = &[
    (2026, 1, 1, "New Year's Day"),
    (2026, 1, 19, "Martin Luther King Jr. Day"),
    (2026, 2, 16, "Presidents' Day"),
    (2026, 4, 3, "Good Friday"),
    (2026, 5, 25, "Memorial Day"),
    (2026, 6, 19, "Juneteenth"),
    (2026, 7, 3, "Independence Day (observed)"),
    (2026, 9, 7, "Labor Day"),
    (2026, 11, 26, "Thanksgiving Day"),
    (2026, 12, 25, "Christmas Day"),
    (2027, 1, 1, "New Year's Day"),
    (2027, 1, 18, "Martin Luther King Jr. Day"),
    (2027, 2, 15, "Presidents' Day"),
    (2027, 3, 26, "Good Friday"),
    (2027, 5, 31, "Memorial Day"),
    (2027, 6, 18, "Juneteenth (observed)"),
    (2027, 7, 5, "Independence Day (observed)"),
    (2027, 9, 6, "Labor Day"),
    (2027, 11, 25, "Thanksgiving Day"),
    (2027, 12, 24, "Christmas Day (observed)"),
];

/// Sessions that close at 13:00 ET.
const EARLY_CLOSES: &[(i32, u32, u32)] = &[(2026, 11, 27), (2026, 12, 24), (2027, 11, 26)];

const COVERAGE: (NaiveDate, NaiveDate) = (
    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
    NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketState {
    PreMarket,
    Open,
    PostClose,
    Closed,
    /// The date is outside the vendored calendar; the state cannot be verified.
    Unknown,
}

impl MarketState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreMarket => "pre_market",
            Self::Open => "open",
            Self::PostClose => "post_close",
            Self::Closed => "closed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketClock {
    pub asset_class: AssetClass,
    pub now_utc: DateTime<Utc>,
    /// The same instant in New York, RFC 3339 with its offset (DST-aware).
    pub now_et: String,
    pub date_et: NaiveDate,
    pub weekday: String,
    pub state: MarketState,
    /// Why the market is closed, or the early-close note on a shortened session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Date of the most recent completed regular session, the day the newest
    /// daily bar belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior_close_date: Option<NaiveDate>,
    /// The next date the market opens, so an agent can say when data goes stale.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_open_date: Option<NaiveDate>,
    pub calendar_coverage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

pub fn holiday(date: NaiveDate) -> Option<&'static str> {
    HOLIDAYS
        .iter()
        .find(|(y, m, d, _)| date == NaiveDate::from_ymd_opt(*y, *m, *d).unwrap())
        .map(|(_, _, _, name)| *name)
}

fn is_early_close(date: NaiveDate) -> bool {
    EARLY_CLOSES
        .iter()
        .any(|(y, m, d)| date == NaiveDate::from_ymd_opt(*y, *m, *d).unwrap())
}

fn in_coverage(date: NaiveDate) -> bool {
    date >= COVERAGE.0 && date <= COVERAGE.1
}

/// A date on which a regular session happens. `None` when the calendar can't say.
pub fn is_trading_day(date: NaiveDate) -> Option<bool> {
    if !in_coverage(date) {
        return None;
    }
    let weekend = matches!(date.weekday(), Weekday::Sat | Weekday::Sun);
    Some(!weekend && holiday(date).is_none())
}

fn step_trading_day(from: NaiveDate, days: i64) -> Option<NaiveDate> {
    let mut date = from;
    loop {
        date = date.checked_add_signed(chrono::Duration::days(days))?;
        match is_trading_day(date) {
            Some(true) => return Some(date),
            Some(false) => continue,
            None => return None,
        }
    }
}

/// The NYSE clock: what every equity surface means by "the market".
pub fn market_clock(now: DateTime<Utc>) -> MarketClock {
    market_clock_for(now, AssetClass::Equity)
}

fn base_clock(now: DateTime<Utc>, class: AssetClass) -> MarketClock {
    let et = New_York.from_utc_datetime(&now.naive_utc());
    let date = et.date_naive();
    MarketClock {
        asset_class: class,
        now_utc: now,
        now_et: et.to_rfc3339(),
        date_et: date,
        weekday: format!("{:?}", date.weekday()),
        state: MarketState::Unknown,
        reason: None,
        prior_close_date: None,
        next_open_date: None,
        calendar_coverage: format!("{} to {}", COVERAGE.0, COVERAGE.1),
        note: None,
    }
}

/// Crypto never closes; its daily bars roll at 00:00 UTC, so the "prior
/// session" for overnight windows is the previous UTC day.
fn crypto_clock(now: DateTime<Utc>) -> MarketClock {
    let utc_date = now.date_naive();
    MarketClock {
        state: MarketState::Open,
        reason: Some("trades continuously; daily bars roll at 00:00 UTC".into()),
        prior_close_date: utc_date.pred_opt(),
        next_open_date: None,
        calendar_coverage: "not applicable: no exchange calendar".into(),
        ..base_clock(now, AssetClass::Crypto)
    }
}

fn previous_weekday(date: NaiveDate) -> Option<NaiveDate> {
    let mut d = date;
    loop {
        d = d.pred_opt()?;
        if !matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
            return Some(d);
        }
    }
}

/// Forex runs Sunday 17:00 ET to Friday 17:00 ET. Futures borrow the same
/// window as an approximation and say so; CME hours differ by product.
fn fx_clock(now: DateTime<Utc>, class: AssetClass) -> MarketClock {
    let base = base_clock(now, class);
    let et = New_York.from_utc_datetime(&now.naive_utc());
    let (date, time) = (et.date_naive(), et.time());
    let closed = match date.weekday() {
        Weekday::Sat => true,
        Weekday::Fri => time >= FX_ROLL,
        Weekday::Sun => time < FX_ROLL,
        _ => false,
    };
    let note = (class == AssetClass::Future).then(|| {
        "futures use the forex session as an approximation; CME hours differ by product".to_string()
    });
    if closed {
        let friday = previous_weekday(date.succ_opt().unwrap_or(date))
            .filter(|d| d.weekday() == Weekday::Fri)
            .or_else(|| previous_weekday(date));
        let sunday = (0..7)
            .filter_map(|i| date.checked_add_signed(chrono::Duration::days(i)))
            .find(|d| d.weekday() == Weekday::Sun);
        return MarketClock {
            state: MarketState::Closed,
            reason: Some("weekend (session runs Sunday 17:00 to Friday 17:00 ET)".into()),
            prior_close_date: friday,
            next_open_date: sunday,
            calendar_coverage: "Sunday 17:00 to Friday 17:00 ET, no holiday table".into(),
            note,
            ..base
        };
    }
    MarketClock {
        state: MarketState::Open,
        reason: Some("session runs Sunday 17:00 to Friday 17:00 ET".into()),
        prior_close_date: previous_weekday(date),
        next_open_date: None,
        calendar_coverage: "Sunday 17:00 to Friday 17:00 ET, no holiday table".into(),
        note,
        ..base
    }
}

pub fn market_clock_for(now: DateTime<Utc>, class: AssetClass) -> MarketClock {
    match class {
        AssetClass::Equity => equity_clock(now),
        AssetClass::Crypto => crypto_clock(now),
        AssetClass::Forex | AssetClass::Future => fx_clock(now, class),
    }
}

fn equity_clock(now: DateTime<Utc>) -> MarketClock {
    let et = New_York.from_utc_datetime(&now.naive_utc());
    let date = et.date_naive();
    let time = et.time();
    let base = base_clock(now, AssetClass::Equity);

    let Some(trading_today) = is_trading_day(date) else {
        return MarketClock {
            note: Some(format!(
                "{date} is outside the vendored NYSE calendar ({}); session state cannot be verified",
                base.calendar_coverage
            )),
            ..base
        };
    };

    if !trading_today {
        let reason = holiday(date)
            .map(str::to_string)
            .unwrap_or_else(|| "weekend".to_string());
        return MarketClock {
            state: MarketState::Closed,
            reason: Some(reason),
            prior_close_date: step_trading_day(date, -1),
            next_open_date: step_trading_day(date, 1),
            ..base
        };
    }

    let early = is_early_close(date);
    let close = if early { EARLY_CLOSE } else { CLOSE };
    let reason = early.then(|| "early close at 13:00 ET".to_string());
    if time < OPEN {
        MarketClock {
            state: MarketState::PreMarket,
            reason,
            prior_close_date: step_trading_day(date, -1),
            next_open_date: Some(date),
            ..base
        }
    } else if time < close {
        MarketClock {
            state: MarketState::Open,
            reason,
            prior_close_date: step_trading_day(date, -1),
            next_open_date: step_trading_day(date, 1),
            ..base
        }
    } else {
        MarketClock {
            state: MarketState::PostClose,
            reason,
            prior_close_date: Some(date),
            next_open_date: step_trading_day(date, 1),
            ..base
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn et(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        New_York
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn labor_day_is_closed_with_prior_friday_and_next_tuesday() {
        let c = market_clock(et(2026, 9, 7, 11, 0));
        assert_eq!(c.state, MarketState::Closed);
        assert_eq!(c.reason.as_deref(), Some("Labor Day"));
        assert_eq!(c.prior_close_date, Some(ymd(2026, 9, 4)));
        assert_eq!(c.next_open_date, Some(ymd(2026, 9, 8)));
    }

    #[test]
    fn weekend_is_closed() {
        let c = market_clock(et(2026, 9, 5, 12, 0));
        assert_eq!(c.state, MarketState::Closed);
        assert_eq!(c.reason.as_deref(), Some("weekend"));
        assert_eq!(c.weekday, "Sat");
    }

    #[test]
    fn open_boundaries_in_daylight_time() {
        assert_eq!(
            market_clock(et(2026, 9, 8, 9, 29)).state,
            MarketState::PreMarket
        );
        assert_eq!(market_clock(et(2026, 9, 8, 9, 30)).state, MarketState::Open);
        assert_eq!(
            market_clock(et(2026, 9, 8, 15, 59)).state,
            MarketState::Open
        );
        assert_eq!(
            market_clock(et(2026, 9, 8, 16, 0)).state,
            MarketState::PostClose
        );
    }

    #[test]
    fn open_boundaries_in_standard_time_are_read_in_eastern_not_utc() {
        // 14:30Z is 09:30 EST in January; a UTC-4 assumption would call it pre-market.
        let jan = Utc.with_ymd_and_hms(2026, 1, 20, 14, 30, 0).unwrap();
        assert_eq!(market_clock(jan).state, MarketState::Open);
        let before = Utc.with_ymd_and_hms(2026, 1, 20, 14, 29, 0).unwrap();
        assert_eq!(market_clock(before).state, MarketState::PreMarket);
    }

    #[test]
    fn early_close_ends_the_session_at_one() {
        let c = market_clock(et(2026, 11, 27, 13, 0));
        assert_eq!(c.state, MarketState::PostClose);
        assert_eq!(c.reason.as_deref(), Some("early close at 13:00 ET"));
        assert_eq!(
            market_clock(et(2026, 11, 27, 12, 59)).state,
            MarketState::Open
        );
    }

    #[test]
    fn pre_market_after_a_holiday_points_at_the_last_real_session() {
        let c = market_clock(et(2026, 9, 8, 8, 30));
        assert_eq!(c.state, MarketState::PreMarket);
        assert_eq!(c.prior_close_date, Some(ymd(2026, 9, 4)));
        assert_eq!(c.next_open_date, Some(ymd(2026, 9, 8)));
    }

    #[test]
    fn post_close_prior_session_is_today() {
        let c = market_clock(et(2026, 9, 8, 17, 0));
        assert_eq!(c.prior_close_date, Some(ymd(2026, 9, 8)));
        assert_eq!(c.next_open_date, Some(ymd(2026, 9, 9)));
    }

    #[test]
    fn outside_coverage_is_unknown_with_a_note() {
        let c = market_clock(et(2028, 3, 1, 12, 0));
        assert_eq!(c.state, MarketState::Unknown);
        assert!(c
            .note
            .unwrap()
            .contains("outside the vendored NYSE calendar"));
        assert_eq!(c.prior_close_date, None);
        assert_eq!(is_trading_day(ymd(2025, 12, 31)), None);
    }

    #[test]
    fn crypto_is_always_open_with_a_utc_prior_day() {
        let c = market_clock_for(et(2026, 9, 7, 11, 0), AssetClass::Crypto);
        assert_eq!(c.state, MarketState::Open);
        assert_eq!(c.asset_class, AssetClass::Crypto);
        // 11:00 ET on 09-07 is 15:00 UTC on 09-07, so the prior UTC day is 09-06
        assert_eq!(c.prior_close_date, Some(ymd(2026, 9, 6)));
        assert_eq!(c.next_open_date, None);
    }

    #[test]
    fn forex_session_boundaries() {
        assert_eq!(
            market_clock_for(et(2026, 9, 5, 12, 0), AssetClass::Forex).state,
            MarketState::Closed
        );
        assert_eq!(
            market_clock_for(et(2026, 9, 4, 17, 30), AssetClass::Forex).state,
            MarketState::Closed
        );
        assert_eq!(
            market_clock_for(et(2026, 9, 4, 16, 59), AssetClass::Forex).state,
            MarketState::Open
        );
        assert_eq!(
            market_clock_for(et(2026, 9, 6, 16, 59), AssetClass::Forex).state,
            MarketState::Closed
        );
        assert_eq!(
            market_clock_for(et(2026, 9, 6, 17, 0), AssetClass::Forex).state,
            MarketState::Open
        );
        // Labor Day is not a forex holiday
        let mon = market_clock_for(et(2026, 9, 7, 11, 0), AssetClass::Forex);
        assert_eq!(mon.state, MarketState::Open);
        assert_eq!(mon.prior_close_date, Some(ymd(2026, 9, 4)));
        let sat = market_clock_for(et(2026, 9, 5, 12, 0), AssetClass::Forex);
        assert_eq!(sat.prior_close_date, Some(ymd(2026, 9, 4)));
        assert_eq!(sat.next_open_date, Some(ymd(2026, 9, 6)));
        assert!(market_clock_for(et(2026, 9, 9, 12, 0), AssetClass::Future)
            .note
            .unwrap()
            .contains("approximation"));
    }

    #[test]
    fn now_et_carries_the_eastern_offset() {
        let c = market_clock(et(2026, 9, 8, 9, 30));
        assert!(c.now_et.ends_with("-04:00"));
        let w = market_clock(et(2026, 1, 20, 9, 30));
        assert!(w.now_et.ends_with("-05:00"));
    }
}
