//! Margin overlay on a `RiskFrame`: buying-power cap, borrow amount, and the
//! price at which a maintenance call would fire. Pure and synchronous.
//! Single-position model (assumes this is the account's only margined
//! position); interest and fees are not modeled — the disclaimer says so.

use serde::Serialize;

use crate::domain::error::DomainError;
use crate::domain::risk::{Direction, RiskFrame};

/// Overnight Reg-T buying power; 4x day-trade BP is opt-in and not holdable.
pub const OVERNIGHT_MAX_LEVERAGE: f64 = 2.0;
pub const INTRADAY_MAX_LEVERAGE: f64 = 4.0;
pub const DEFAULT_LEVERAGE: f64 = 2.0;
pub const DEFAULT_MAINTENANCE: f64 = 0.25;
pub const DEFAULT_RISK_PCT: f64 = 0.01;

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "margin".into(),
        message: message.into(),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MarginInputs {
    pub equity_usd: f64,
    pub leverage: f64,
    pub maintenance: f64,
    /// Fraction of equity risked to the stop (sizes the underlying RiskFrame).
    pub risk_pct: f64,
    /// Unlocks 4x intraday buying power (with a not-holdable-overnight note).
    pub intraday_bp: bool,
}

impl Default for MarginInputs {
    fn default() -> Self {
        Self {
            equity_usd: 0.0,
            leverage: DEFAULT_LEVERAGE,
            maintenance: DEFAULT_MAINTENANCE,
            risk_pct: DEFAULT_RISK_PCT,
            intraday_bp: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MarginFrame {
    pub equity_usd: f64,
    pub leverage: f64,
    pub maintenance: f64,
    pub shares: u64,
    pub notional_usd: f64,
    pub cash_used_usd: f64,
    pub borrowed_usd: f64,
    /// None when nothing is borrowed — no maintenance call is possible.
    pub margin_call_price: Option<f64>,
    pub mc_distance_pct: Option<f64>,
    pub mc_distance_atr: Option<f64>,
    /// True when buying power, not the risk budget, set the share count.
    pub capped_by_buying_power: bool,
    pub notes: Vec<String>,
}

/// Overlay margin mechanics on an already-computed long `RiskFrame`.
pub fn margin_frame(risk: &RiskFrame, inputs: &MarginInputs) -> Result<MarginFrame, DomainError> {
    if risk.direction != Direction::Long {
        return Err(fail("margin framing supports long dip entries only"));
    }
    if !(inputs.equity_usd.is_finite() && inputs.equity_usd > 0.0) {
        return Err(fail("equity must be a positive number"));
    }
    if !inputs.leverage.is_finite() || !inputs.maintenance.is_finite() {
        return Err(fail("leverage and maintenance must be numbers"));
    }
    let max_leverage = if inputs.intraday_bp {
        INTRADAY_MAX_LEVERAGE
    } else {
        OVERNIGHT_MAX_LEVERAGE
    };
    let leverage = inputs.leverage.clamp(1.0, max_leverage);
    let maintenance = inputs.maintenance.clamp(0.10, 0.90);

    let mut notes: Vec<String> = Vec::new();
    if leverage > OVERNIGHT_MAX_LEVERAGE {
        notes.push(format!(
            "{leverage}x is intraday buying power — not holdable overnight (Reg-T max is {OVERNIGHT_MAX_LEVERAGE}x)"
        ));
    }

    let buying_power = inputs.equity_usd * leverage;
    let bp_shares = (buying_power / risk.entry).floor() as u64;
    let shares = risk.shares.min(bp_shares);
    let capped_by_buying_power = shares < risk.shares;
    if capped_by_buying_power {
        notes.push("buying power, not the risk budget, capped this size".into());
    }

    let notional = shares as f64 * risk.entry;
    let cash_used = inputs.equity_usd.min(notional);
    let borrowed = (notional - cash_used).max(0.0);

    let margin_call_price =
        (borrowed > 0.0 && shares > 0).then(|| borrowed / (shares as f64 * (1.0 - maintenance)));
    let mc_distance_pct = margin_call_price.map(|mc| (risk.entry - mc) / risk.entry * 100.0);
    let mc_distance_atr = margin_call_price.map(|mc| (risk.entry - mc) / risk.atr);

    if let Some(mc) = margin_call_price {
        if mc >= risk.stop {
            notes.push(format!(
                "margin call (~{mc:.2}) would trigger BEFORE your stop ({:.2}) at this leverage",
                risk.stop
            ));
        }
    }

    Ok(MarginFrame {
        equity_usd: inputs.equity_usd,
        leverage,
        maintenance,
        shares,
        notional_usd: notional,
        cash_used_usd: cash_used,
        borrowed_usd: borrowed,
        margin_call_price,
        mc_distance_pct,
        mc_distance_atr,
        capped_by_buying_power,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::risk::RiskFrame;
    use chrono::TimeZone;

    /// entry 100, ATR 4, 2xATR stop at 92, risk-budget size 500 shares.
    fn risk(shares: u64) -> RiskFrame {
        RiskFrame {
            ticker: "TEST".into(),
            direction: Direction::Long,
            entry: 100.0,
            atr: 4.0,
            stop_multiple: 2.0,
            stop: 92.0,
            risk_per_share: 8.0,
            shares,
            max_loss_usd: shares as f64 * 8.0,
            budget_usd: shares as f64 * 8.0,
            targets: [108.0, 116.0, 124.0],
            notional_usd: shares as f64 * 100.0,
            bars_used: 30,
            note: None,
            generated_at: chrono::Utc.with_ymd_and_hms(2026, 8, 14, 0, 0, 0).unwrap(),
        }
    }

    fn inputs(equity: f64) -> MarginInputs {
        MarginInputs {
            equity_usd: equity,
            ..MarginInputs::default()
        }
    }

    #[test]
    fn no_borrow_means_no_margin_call() {
        // 10 risk shares × $100 = $1,000 notional, equity $50,000 — all cash
        let f = margin_frame(&risk(10), &inputs(50_000.0)).unwrap();
        assert_eq!(f.shares, 10);
        assert_eq!(f.borrowed_usd, 0.0);
        assert_eq!(f.margin_call_price, None);
        assert!(!f.capped_by_buying_power);
        assert!(f.notes.is_empty());
    }

    #[test]
    fn two_x_case_hand_computed() {
        // equity 10k, 2x BP = 20k -> bp cap 200 shares; risk wants 500.
        // notional 20k, cash 10k, borrowed 10k.
        // mc = 10_000 / (200 × 0.75) = 66.67
        let f = margin_frame(&risk(500), &inputs(10_000.0)).unwrap();
        assert_eq!(f.shares, 200);
        assert!(f.capped_by_buying_power);
        assert!((f.notional_usd - 20_000.0).abs() < 1e-9);
        assert!((f.borrowed_usd - 10_000.0).abs() < 1e-9);
        let mc = f.margin_call_price.unwrap();
        assert!((mc - 66.6667).abs() < 1e-3);
        assert!((f.mc_distance_pct.unwrap() - 33.3333).abs() < 1e-3);
        assert!((f.mc_distance_atr.unwrap() - 8.3333).abs() < 1e-3);
        // mc (66.67) < stop (92): stop fires first, no warning note
        assert!(!f.notes.iter().any(|n| n.contains("BEFORE")));
    }

    #[test]
    fn margin_call_above_stop_warns() {
        // Force a high mc: maintenance 0.90 -> mc = borrowed/(shares×0.1).
        // equity 10k, 2x -> 200 shares, borrowed 10k -> mc = 10k/20 = 500?? that's
        // above entry — degenerate but exactly the case the warning exists for.
        let mut i = inputs(10_000.0);
        i.maintenance = 0.90;
        let f = margin_frame(&risk(500), &i).unwrap();
        assert!(f.margin_call_price.unwrap() >= 92.0);
        assert!(f.notes.iter().any(|n| n.contains("BEFORE")));
    }

    #[test]
    fn overnight_clamp_and_intraday_unlock() {
        let mut i = inputs(10_000.0);
        i.leverage = 4.0;
        let f = margin_frame(&risk(500), &i).unwrap();
        assert_eq!(f.leverage, 2.0); // clamped to Reg-T overnight
        assert!(f.notes.iter().all(|n| !n.contains("intraday")));

        i.intraday_bp = true;
        let f = margin_frame(&risk(500), &i).unwrap();
        assert_eq!(f.leverage, 4.0);
        assert_eq!(f.shares, 400); // 40k BP / 100
        assert!(f.notes.iter().any(|n| n.contains("not holdable overnight")));
    }

    #[test]
    fn zero_shares_passthrough_and_errors() {
        let f = margin_frame(&risk(0), &inputs(10_000.0)).unwrap();
        assert_eq!(f.shares, 0);
        assert_eq!(f.margin_call_price, None);

        assert!(margin_frame(&risk(10), &inputs(0.0)).is_err());
        assert!(margin_frame(&risk(10), &inputs(f64::NAN)).is_err());
        let mut short = risk(10);
        short.direction = Direction::Short;
        assert!(margin_frame(&short, &inputs(10_000.0)).is_err());
    }
}
