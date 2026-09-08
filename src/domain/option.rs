//! Defined-risk framing for a long option: the premium paid is the whole
//! loss, and everything else here is the arithmetic an agent tends to skip.
//! Pure — `now` is injected. No Greeks, no probability of profit: the ATR
//! range is a scale for the move the trade needs, never a forecast.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::America::New_York;
use serde::Serialize;

use crate::domain::clock::is_trading_day;
use crate::domain::error::DomainError;
use crate::domain::risk::{atr, ATR_PERIOD};
use crate::domain::trade_journal::OptionKind;
use crate::domain::values::bar::Bar;

/// US-listed equity options deliver 100 shares per contract.
pub const CONTRACT_MULTIPLIER: f64 = 100.0;
/// Under this many calendar days the frame warns about time decay.
const THETA_WARNING_DAYS: i64 = 21;

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "option".into(),
        message: message.into(),
    }
}

/// What the caller knows from the broker's chain quote and their own budget.
#[derive(Debug, Clone)]
pub struct OptionSpec {
    pub underlying: String,
    pub kind: OptionKind,
    pub strike: f64,
    pub expiry: NaiveDate,
    /// Quoted premium per share (the contract costs premium × 100).
    pub premium: f64,
    pub budget_usd: f64,
}

/// Context fetched at the application edge; every field may be missing.
#[derive(Debug, Clone, Copy, Default)]
pub struct MarketContext {
    pub spot: Option<f64>,
    pub realized_vol: Option<f64>,
    pub iv_rank: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OptionFrame {
    pub underlying: String,
    pub kind: OptionKind,
    pub strike: f64,
    pub expiry: NaiveDate,
    pub premium: f64,
    pub contract_multiplier: f64,
    /// Whole contracts whose total premium fits the budget.
    pub contracts: u64,
    /// contracts × premium × multiplier: the max loss, paid up front.
    pub cost_usd: f64,
    pub budget_usd: f64,
    /// Underlying price at expiry where the position breaks even before fees.
    pub breakeven: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spot: Option<f64>,
    /// Percent the underlying must move, in the trade's direction, to reach breakeven.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_to_breakeven_pct: Option<f64>,
    pub calendar_days_to_expiry: i64,
    /// Sessions left per the vendored NYSE calendar; None past its coverage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trading_days_to_expiry: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atr: Option<f64>,
    /// ATR(14) × √(trading days): the scale of a typical range through expiry, not a forecast.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atr_scaled_range: Option<f64>,
    /// |breakeven − spot| ÷ atr_scaled_range: how many typical ranges the trade needs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breakeven_in_atr_ranges: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realized_vol: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iv_rank: Option<f64>,
    pub notes: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

/// Sessions strictly after `today` through `expiry`; None when the calendar
/// can't say for some date in the window.
fn trading_days_between(today: NaiveDate, expiry: NaiveDate) -> Option<usize> {
    let mut count = 0;
    let mut d = today;
    while d < expiry {
        d = d.succ_opt()?;
        if is_trading_day(d)? {
            count += 1;
        }
    }
    Some(count)
}

pub fn option_frame(
    spec: &OptionSpec,
    ctx: MarketContext,
    bars: &[Bar],
    now: DateTime<Utc>,
) -> Result<OptionFrame, DomainError> {
    if !(spec.premium.is_finite() && spec.premium > 0.0) {
        return Err(fail("premium must be a positive number"));
    }
    if !(spec.strike.is_finite() && spec.strike > 0.0) {
        return Err(fail("strike must be a positive number"));
    }
    if !(spec.budget_usd.is_finite() && spec.budget_usd > 0.0) {
        return Err(fail("budget must be a positive number"));
    }
    let today = now.with_timezone(&New_York).date_naive();
    let calendar_days = (spec.expiry - today).num_days();
    if calendar_days < 0 {
        return Err(fail(format!(
            "expiry {} is before today {today} in New York",
            spec.expiry
        )));
    }
    let breakeven = match spec.kind {
        OptionKind::Call => spec.strike + spec.premium,
        OptionKind::Put => spec.strike - spec.premium,
    };
    if !(breakeven.is_finite() && breakeven > 0.0) {
        return Err(fail(
            "breakeven must be a finite positive price (a put's premium cannot exceed its strike)",
        ));
    }

    let mut notes = Vec::new();
    let per_contract = spec.premium * CONTRACT_MULTIPLIER;
    if !per_contract.is_finite() {
        return Err(fail("premium is too large to price a contract"));
    }
    let contracts = (spec.budget_usd / per_contract).floor() as u64;
    if contracts == 0 {
        notes.push(format!(
            "budget ${:.2} buys no contract at ${per_contract:.2} each",
            spec.budget_usd
        ));
    }
    let cost_usd = contracts as f64 * per_contract;

    if calendar_days <= THETA_WARNING_DAYS {
        notes.push(format!(
            "{calendar_days} calendar days to expiry: time decay accelerates from here, and the full premium can be lost with the underlying unchanged"
        ));
    }
    let trading_days = trading_days_between(today, spec.expiry);
    if trading_days.is_none() {
        notes.push("expiry is past the vendored NYSE calendar; trading-day count and ATR range unavailable".into());
    }

    let atr14 = atr(bars, ATR_PERIOD).filter(|a| a.is_finite() && *a > 0.0);
    if atr14.is_none() {
        notes.push(format!(
            "not enough history for ATR({ATR_PERIOD}); the range scale is unavailable"
        ));
    }
    let atr_scaled_range = match (atr14, trading_days) {
        (Some(a), Some(n)) if n > 0 => Some(a * (n as f64).sqrt()),
        _ => None,
    };

    let move_to_breakeven_pct = ctx.spot.filter(|s| *s > 0.0).map(|s| match spec.kind {
        OptionKind::Call => (breakeven - s) / s * 100.0,
        OptionKind::Put => (s - breakeven) / s * 100.0,
    });
    let breakeven_in_atr_ranges = match (ctx.spot, atr_scaled_range) {
        (Some(s), Some(r)) if r > 0.0 => Some((breakeven - s).abs() / r),
        _ => None,
    };
    if ctx.spot.is_none() {
        notes.push("spot unavailable; moneyness and the move to breakeven are unscored".into());
    }
    if ctx.iv_rank.is_none() {
        notes.push("IV rank is not available from keyless sources; the broker's chain quote carries implied volatility".into());
    }

    Ok(OptionFrame {
        underlying: spec.underlying.clone(),
        kind: spec.kind,
        strike: spec.strike,
        expiry: spec.expiry,
        premium: spec.premium,
        contract_multiplier: CONTRACT_MULTIPLIER,
        contracts,
        cost_usd,
        budget_usd: spec.budget_usd,
        breakeven,
        spot: ctx.spot,
        move_to_breakeven_pct,
        calendar_days_to_expiry: calendar_days,
        trading_days_to_expiry: trading_days,
        atr: atr14,
        atr_scaled_range,
        breakeven_in_atr_ranges,
        realized_vol: ctx.realized_vol,
        iv_rank: ctx.iv_rank,
        notes,
        generated_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 8, 14, 0, 0).unwrap()
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// 16 bars with a constant 4.0 true range.
    fn bars() -> Vec<Bar> {
        let bar = |high: f64, low: f64, close: f64| Bar {
            date: ymd(2026, 9, 4),
            open: close,
            high,
            low,
            close,
        };
        let mut v = vec![bar(101.0, 99.0, 100.0)];
        for _ in 0..15 {
            v.push(bar(102.0, 98.0, 100.0));
        }
        v
    }

    fn spec(kind: OptionKind, expiry: NaiveDate) -> OptionSpec {
        OptionSpec {
            underlying: "NVDA".into(),
            kind,
            strike: 100.0,
            expiry,
            premium: 5.0,
            budget_usd: 1200.0,
        }
    }

    #[test]
    fn call_contracts_cost_breakeven_and_ranges() {
        let ctx = MarketContext {
            spot: Some(100.0),
            realized_vol: Some(0.4),
            iv_rank: None,
        };
        let f = option_frame(
            &spec(OptionKind::Call, ymd(2026, 9, 18)),
            ctx,
            &bars(),
            now(),
        )
        .unwrap();
        assert_eq!(f.contracts, 2); // floor(1200 / 500)
        assert!((f.cost_usd - 1000.0).abs() < 1e-9);
        assert!((f.breakeven - 105.0).abs() < 1e-9);
        assert!((f.move_to_breakeven_pct.unwrap() - 5.0).abs() < 1e-9);
        assert_eq!(f.calendar_days_to_expiry, 10);
        assert_eq!(f.trading_days_to_expiry, Some(8)); // 09-09..09-11, 09-14..09-18
        assert!((f.atr.unwrap() - 4.0).abs() < 1e-9);
        let range = 4.0 * 8f64.sqrt();
        assert!((f.atr_scaled_range.unwrap() - range).abs() < 1e-9);
        assert!((f.breakeven_in_atr_ranges.unwrap() - 5.0 / range).abs() < 1e-9);
        assert!(f.notes.iter().any(|n| n.contains("time decay")));
        assert!(f.notes.iter().any(|n| n.contains("IV rank")));
    }

    #[test]
    fn put_breakeven_and_move_direction() {
        let ctx = MarketContext {
            spot: Some(100.0),
            ..Default::default()
        };
        let f = option_frame(
            &spec(OptionKind::Put, ymd(2026, 11, 20)),
            ctx,
            &bars(),
            now(),
        )
        .unwrap();
        assert!((f.breakeven - 95.0).abs() < 1e-9);
        assert!((f.move_to_breakeven_pct.unwrap() - 5.0).abs() < 1e-9);
        assert!(!f.notes.iter().any(|n| n.contains("time decay")));
    }

    #[test]
    fn zero_contracts_is_a_note_and_missing_context_is_said() {
        let mut s = spec(OptionKind::Call, ymd(2026, 10, 16));
        s.budget_usd = 300.0;
        let f = option_frame(&s, MarketContext::default(), &[], now()).unwrap();
        assert_eq!(f.contracts, 0);
        assert_eq!(f.cost_usd, 0.0);
        assert!(f.notes.iter().any(|n| n.contains("buys no contract")));
        assert!(f.notes.iter().any(|n| n.contains("spot unavailable")));
        assert!(f.notes.iter().any(|n| n.contains("not enough history")));
        assert!(f.atr_scaled_range.is_none());
    }

    #[test]
    fn expiry_past_the_calendar_loses_the_trading_day_count_not_the_frame() {
        let f = option_frame(
            &spec(OptionKind::Call, ymd(2028, 1, 21)),
            MarketContext::default(),
            &bars(),
            now(),
        )
        .unwrap();
        assert_eq!(f.trading_days_to_expiry, None);
        assert!(f.atr.is_some());
        assert!(f.atr_scaled_range.is_none());
        assert!(f
            .notes
            .iter()
            .any(|n| n.contains("past the vendored NYSE calendar")));
    }

    #[test]
    fn rejects_expired_and_impossible_inputs() {
        assert!(option_frame(
            &spec(OptionKind::Call, ymd(2026, 9, 4)),
            MarketContext::default(),
            &bars(),
            now()
        )
        .is_err());
        let mut bad = spec(OptionKind::Put, ymd(2026, 10, 16));
        bad.premium = 150.0;
        assert!(option_frame(&bad, MarketContext::default(), &bars(), now()).is_err());
        bad.premium = 0.0;
        assert!(option_frame(&bad, MarketContext::default(), &bars(), now()).is_err());
        // finite inputs whose derived values overflow are refused, never serialized as inf/NaN
        let mut huge = spec(OptionKind::Call, ymd(2026, 10, 16));
        huge.strike = f64::MAX;
        huge.premium = f64::MAX;
        assert!(option_frame(&huge, MarketContext::default(), &bars(), now()).is_err());
    }
}
