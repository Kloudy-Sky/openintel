//! Strategy port for signal detection and opportunity discovery.
//!
//! Defines the [`Strategy`] trait and supporting types for detecting trading
//! opportunities from intel data. Strategies analyze entries in the knowledge
//! base and identify actionable signals.
//!
//! # Overview
//!
//! The strategy system is designed for extensibility:
//!
//! - Implement [`Strategy`] to add new detection algorithms
//! - Use [`DetectionContext`] to access intel data during detection
//! - Return [`Opportunity`] values ranked by confidence and edge

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::entities::trade::Trade;
use crate::domain::error::DomainError;

/// Suggested trade direction for an opportunity.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Bullish,
    Bearish,
    Yes,
    No,
}

/// An opportunity detected by a strategy.
///
/// Represents an actionable trading signal with confidence scoring,
/// estimated edge, and supporting evidence from the intel database.
#[derive(Debug, Clone, Serialize)]
pub struct Opportunity {
    /// Which strategy detected this opportunity.
    pub strategy: String,
    /// What type of signal (e.g., "weather_edge", "earnings_momentum").
    pub signal_type: String,
    /// Human-readable title.
    pub title: String,
    /// Detailed description of the opportunity.
    pub description: String,
    /// Confidence level (0.0–1.0).
    pub confidence: f64,
    /// Estimated edge in cents per contract, if applicable.
    pub edge_cents: Option<f64>,
    /// Suggested market ticker to trade.
    pub market_ticker: Option<String>,
    /// Suggested trade direction.
    pub suggested_direction: Option<Direction>,
    /// Suggested action (e.g., "Buy 80 contracts at 28c").
    pub suggested_action: Option<String>,
    /// IDs of supporting intel entries.
    pub supporting_entries: Vec<String>,
    /// Composite score: `confidence × edge_cents × sqrt(liquidity)`.
    /// When edge is unknown, uses `confidence × 100`. When liquidity
    /// is unknown, defaults to 1.0 (no penalty).
    pub score: f64,
    /// Liquidity factor (0.0–1.0), normalized from 24h volume.
    /// `None` when volume data is unavailable (defaults to 1.0 in scoring).
    pub liquidity: Option<f64>,
    /// Current market price (1–99 for Kalshi-style binary markets).
    /// Required for Kelly criterion sizing. `None` when price is unknown.
    pub market_price: Option<f64>,
    /// Suggested position size in cents, computed via Kelly criterion.
    /// `None` when no bankroll is provided, no market price, or sizing is not applicable.
    pub suggested_size_cents: Option<u64>,
    /// When this opportunity was detected.
    pub detected_at: DateTime<Utc>,
}

impl Opportunity {
    /// Compute composite score: confidence × edge × sqrt(liquidity).
    ///
    /// - If no edge estimate, uses `confidence × 100` as proxy.
    /// - If no liquidity data, assumes 1.0 (no penalty).
    /// - Thin markets (low liquidity) get penalized via sqrt dampening.
    pub fn compute_score(confidence: f64, edge_cents: Option<f64>, liquidity: Option<f64>) -> f64 {
        let base = match edge_cents {
            Some(edge) => confidence * edge,
            None => confidence * 100.0,
        };
        let liq_factor = liquidity.unwrap_or(1.0).clamp(0.0, 1.0).sqrt();
        base * liq_factor
    }
}

/// Context provided to strategies during detection.
///
/// Gives strategies access to recent intel entries and open trades
/// for analysis.
pub struct DetectionContext {
    /// Recent intel entries within the detection window.
    pub entries: Vec<IntelEntry>,
    /// Currently open (unresolved) trades.
    pub open_trades: Vec<Trade>,
    /// How many hours back the entries span.
    pub window_hours: u32,
}

/// Trait for signal detection strategies.
///
/// Implement this to add new detection algorithms. Each strategy
/// analyzes intel data and returns zero or more opportunities.
///
/// # Example
///
/// ```ignore
/// struct MyStrategy;
///
/// impl Strategy for MyStrategy {
///     fn name(&self) -> &'static str { "my_strategy" }
///
///     fn detect(&self, ctx: &DetectionContext) -> Result<Vec<Opportunity>, DomainError> {
///         // Analyze ctx.entries, return opportunities
///         Ok(vec![])
///     }
/// }
/// ```
pub trait Strategy: Send + Sync {
    /// Unique name for this strategy.
    fn name(&self) -> &'static str;

    /// Run detection against the provided context.
    fn detect(&self, ctx: &DetectionContext) -> Result<Vec<Opportunity>, DomainError>;
}
