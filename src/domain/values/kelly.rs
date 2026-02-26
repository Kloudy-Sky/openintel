//! Kelly criterion position sizing.
//!
//! Provides mathematical position sizing based on estimated edge and bankroll.
//! Uses the Kelly formula: `f* = (bp - q) / b` where:
//! - `b` = net odds (payout per dollar risked)
//! - `p` = estimated probability of winning
//! - `q` = 1 - p (probability of losing)
//! - `f*` = optimal fraction of bankroll to bet
//!
//! Half-Kelly (f*/2) is the default to reduce variance while capturing
//! most of the expected growth.

use serde::Serialize;

/// Configuration for Kelly sizing calculations.
#[derive(Debug, Clone, Serialize)]
pub struct KellyConfig {
    /// Fraction of full Kelly to use (0.0–1.0). Default: 0.5 (half-Kelly).
    pub fraction: f64,
    /// Maximum position size in cents. Hard cap that overrides Kelly.
    pub max_position_cents: u64,
    /// Minimum edge (probability - implied probability) required before any sizing.
    /// Below this threshold, suggested size is zero.
    pub min_edge: f64,
    /// Maximum fraction of bankroll for any single position (0.0–1.0).
    pub max_bankroll_fraction: f64,
}

impl Default for KellyConfig {
    fn default() -> Self {
        Self {
            fraction: 0.5,
            max_position_cents: 2500, // $25
            min_edge: 0.05,           // 5% minimum edge
            max_bankroll_fraction: 0.25,
        }
    }
}

/// Result of a Kelly sizing calculation.
#[derive(Debug, Clone, Serialize)]
pub struct KellySizing {
    /// Raw Kelly fraction (before applying half-Kelly or caps).
    pub full_kelly_fraction: f64,
    /// Adjusted fraction after applying the Kelly fraction multiplier.
    pub adjusted_fraction: f64,
    /// Suggested position size in cents.
    pub suggested_size_cents: u64,
    /// Which constraint was binding (if any).
    pub binding_constraint: Option<String>,
    /// The estimated probability used.
    pub estimated_probability: f64,
    /// The implied probability from market odds.
    pub implied_probability: f64,
    /// The edge (estimated - implied).
    pub edge: f64,
}

/// Calculate Kelly criterion position sizing.
///
/// # Arguments
/// * `estimated_prob` — Our estimated probability of the outcome (0.0–1.0)
/// * `market_price_cents` — Current market price in cents (1–99 for Kalshi)
/// * `bankroll_cents` — Total available bankroll in cents
/// * `config` — Sizing configuration (fraction, caps, minimums)
///
/// # Returns
/// `None` if inputs are invalid (probability out of range, zero bankroll, etc.)
/// `Some(KellySizing)` with the calculated position size
pub fn compute_kelly(
    estimated_prob: f64,
    market_price_cents: f64,
    bankroll_cents: u64,
    config: &KellyConfig,
) -> Option<KellySizing> {
    // Validate inputs
    if estimated_prob <= 0.0
        || estimated_prob >= 1.0
        || market_price_cents <= 0.0
        || market_price_cents >= 100.0
        || bankroll_cents == 0
    {
        return None;
    }

    let implied_prob = market_price_cents / 100.0;
    let edge = estimated_prob - implied_prob;

    // Below minimum edge threshold — no position
    if edge < config.min_edge {
        return Some(KellySizing {
            full_kelly_fraction: 0.0,
            adjusted_fraction: 0.0,
            suggested_size_cents: 0,
            binding_constraint: Some("below_min_edge".to_string()),
            estimated_probability: estimated_prob,
            implied_probability: implied_prob,
            edge,
        });
    }

    // Kelly formula: f* = (bp - q) / b
    // where b = (100 - market_price) / market_price (net odds)
    // p = estimated_prob, q = 1 - p
    let b = (100.0 - market_price_cents) / market_price_cents;
    let q = 1.0 - estimated_prob;
    let full_kelly = (b * estimated_prob - q) / b;

    // Kelly can be negative if edge is negative (shouldn't happen after min_edge check,
    // but guard anyway)
    if full_kelly <= 0.0 {
        return Some(KellySizing {
            full_kelly_fraction: full_kelly,
            adjusted_fraction: 0.0,
            suggested_size_cents: 0,
            binding_constraint: Some("negative_kelly".to_string()),
            estimated_probability: estimated_prob,
            implied_probability: implied_prob,
            edge,
        });
    }

    // Apply fractional Kelly (e.g., half-Kelly)
    let adjusted = full_kelly * config.fraction;

    // Calculate raw size in cents
    let raw_size = (adjusted * bankroll_cents as f64).round() as u64;

    // Apply constraints and track which one binds
    let mut final_size = raw_size;
    let mut binding = None;

    // Max bankroll fraction
    let max_from_bankroll = (config.max_bankroll_fraction * bankroll_cents as f64).round() as u64;
    if final_size > max_from_bankroll {
        final_size = max_from_bankroll;
        binding = Some("max_bankroll_fraction".to_string());
    }

    // Hard cap
    if final_size > config.max_position_cents {
        final_size = config.max_position_cents;
        binding = Some("max_position_cents".to_string());
    }

    Some(KellySizing {
        full_kelly_fraction: full_kelly,
        adjusted_fraction: adjusted,
        suggested_size_cents: final_size,
        binding_constraint: binding,
        estimated_probability: estimated_prob,
        implied_probability: implied_prob,
        edge,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> KellyConfig {
        KellyConfig::default()
    }

    #[test]
    fn test_basic_kelly_sizing() {
        // 60% estimated vs 30% market price (30c) = 30% edge
        // b = (100-30)/30 = 2.333
        // f* = (2.333 * 0.6 - 0.4) / 2.333 = (1.4 - 0.4) / 2.333 = 0.4286
        let result =
            compute_kelly(0.6, 30.0, 10000, &default_config()).expect("should return sizing");

        assert!(result.full_kelly_fraction > 0.42);
        assert!(result.full_kelly_fraction < 0.44);
        assert!(result.edge > 0.29);
        assert!(result.suggested_size_cents > 0);
    }

    #[test]
    fn test_half_kelly_reduces_size() {
        let half = compute_kelly(0.7, 40.0, 10000, &default_config()).unwrap();

        let mut full_config = default_config();
        full_config.fraction = 1.0;
        let full = compute_kelly(0.7, 40.0, 10000, &full_config).unwrap();

        // Half-Kelly should give roughly half the adjusted fraction
        let ratio = half.adjusted_fraction / full.adjusted_fraction;
        assert!((ratio - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_below_min_edge_returns_zero() {
        // 32% estimated vs 30% market = 2% edge, below 5% min
        let result =
            compute_kelly(0.32, 30.0, 10000, &default_config()).expect("should return sizing");

        assert_eq!(result.suggested_size_cents, 0);
        assert_eq!(
            result.binding_constraint,
            Some("below_min_edge".to_string())
        );
    }

    #[test]
    fn test_max_position_cap() {
        // Huge edge + huge bankroll should hit the $25 cap
        let result =
            compute_kelly(0.9, 20.0, 100_000, &default_config()).expect("should return sizing");

        assert_eq!(result.suggested_size_cents, 2500);
        assert_eq!(
            result.binding_constraint,
            Some("max_position_cents".to_string())
        );
    }

    #[test]
    fn test_max_bankroll_fraction_cap() {
        // Moderate edge but bankroll fraction cap should bind before position cap
        // 50% estimated vs 30% market, bankroll 5000 cents ($50)
        // max_bankroll_fraction 0.25 = 1250 cents, which is less than max_position 2500
        let result =
            compute_kelly(0.5, 30.0, 5000, &default_config()).expect("should return sizing");

        assert!(result.suggested_size_cents <= 1250);
    }

    #[test]
    fn test_invalid_inputs_return_none() {
        assert!(compute_kelly(0.0, 30.0, 10000, &default_config()).is_none());
        assert!(compute_kelly(1.0, 30.0, 10000, &default_config()).is_none());
        assert!(compute_kelly(0.5, 0.0, 10000, &default_config()).is_none());
        assert!(compute_kelly(0.5, 100.0, 10000, &default_config()).is_none());
        assert!(compute_kelly(0.5, 30.0, 0, &default_config()).is_none());
    }

    #[test]
    fn test_negative_edge_returns_zero() {
        // 20% estimated vs 30% market = -10% edge
        let result =
            compute_kelly(0.2, 30.0, 10000, &default_config()).expect("should return sizing");

        assert_eq!(result.suggested_size_cents, 0);
        assert_eq!(
            result.binding_constraint,
            Some("below_min_edge".to_string())
        );
    }

    #[test]
    fn test_small_edge_above_threshold() {
        // 40% estimated vs 30% market = 10% edge (above 5% min)
        let result =
            compute_kelly(0.4, 30.0, 10000, &default_config()).expect("should return sizing");

        assert!(result.suggested_size_cents > 0);
        assert!(result.edge > 0.09);
    }

    #[test]
    fn test_custom_config() {
        let config = KellyConfig {
            fraction: 0.25, // quarter-Kelly
            max_position_cents: 1000,
            min_edge: 0.10,
            max_bankroll_fraction: 0.15,
        };

        // 60% estimated vs 30% market = 30% edge
        let result = compute_kelly(0.6, 30.0, 10000, &config).expect("should return sizing");
        assert!(result.suggested_size_cents > 0);
        assert!(result.suggested_size_cents <= 1000); // position cap
        assert!(result.suggested_size_cents <= 1500); // bankroll cap (15% of 10000)
    }

    #[test]
    fn test_sizing_scales_with_bankroll() {
        let small = compute_kelly(0.6, 30.0, 5000, &default_config()).unwrap();
        let large = compute_kelly(0.6, 30.0, 20000, &default_config()).unwrap();

        // Larger bankroll should produce larger (or equal, if capped) size
        assert!(large.suggested_size_cents >= small.suggested_size_cents);
    }
}
