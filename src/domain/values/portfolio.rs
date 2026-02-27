//! Multi-exchange portfolio management.
//!
//! Provides a unified view of positions across Kalshi and IBKR,
//! grouped by asset class for correlation and concentration tracking.

use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

/// Asset class for correlation grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass {
    Crypto,
    Equities,
    Rates,
    Weather,
    Events,
    Commodities,
    Forex,
    Unknown,
}

impl AssetClass {
    /// Infer asset class from a ticker symbol.
    pub fn from_ticker(ticker: &str) -> Self {
        let t = ticker.to_uppercase();

        // Crypto tickers and proxies
        if matches!(
            t.as_str(),
            "BTC" | "ETH" | "COIN" | "MARA" | "RIOT" | "MSTR" | "BITO" | "IBIT" | "ETHE" | "CRCL"
        ) || t.starts_with("KXBTC")
            || t.starts_with("KXETH")
            || t.starts_with("KXCRYPTO")
        {
            return AssetClass::Crypto;
        }

        // Rates / bonds
        if matches!(t.as_str(), "TLT" | "SHY" | "XLF" | "KRE")
            || t.starts_with("KXFED")
            || t.starts_with("KXRATE")
        {
            return AssetClass::Rates;
        }

        // Weather
        if t.starts_with("KXHIGH") || t.starts_with("KXLOW") || t.starts_with("KXTEMP") {
            return AssetClass::Weather;
        }

        // S&P / Nasdaq — equity indices
        if matches!(t.as_str(), "SPY" | "VOO" | "IVV" | "QQQ" | "TQQQ")
            || t.starts_with("KXINXY")
            || t.starts_with("KXNAS")
            || t.starts_with("KXSP")
        {
            return AssetClass::Equities;
        }

        // Other Kalshi series → Events
        if t.starts_with("KX") {
            return AssetClass::Events;
        }

        // Default: if it looks like a stock ticker (1-5 uppercase letters), equities
        if t.len() <= 5 && t.chars().all(|c| c.is_ascii_uppercase()) {
            return AssetClass::Equities;
        }

        AssetClass::Unknown
    }
}

impl fmt::Display for AssetClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetClass::Crypto => write!(f, "crypto"),
            AssetClass::Equities => write!(f, "equities"),
            AssetClass::Rates => write!(f, "rates"),
            AssetClass::Weather => write!(f, "weather"),
            AssetClass::Events => write!(f, "events"),
            AssetClass::Commodities => write!(f, "commodities"),
            AssetClass::Forex => write!(f, "forex"),
            AssetClass::Unknown => write!(f, "unknown"),
        }
    }
}

impl FromStr for AssetClass {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "crypto" => Ok(AssetClass::Crypto),
            "equities" | "equity" | "stocks" => Ok(AssetClass::Equities),
            "rates" | "bonds" | "fixed_income" => Ok(AssetClass::Rates),
            "weather" => Ok(AssetClass::Weather),
            "events" => Ok(AssetClass::Events),
            "commodities" => Ok(AssetClass::Commodities),
            "forex" | "fx" => Ok(AssetClass::Forex),
            "unknown" => Ok(AssetClass::Unknown),
            _ => Err(format!("Unknown asset class: {s}")),
        }
    }
}

/// Exchange where a position is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Exchange {
    Kalshi,
    Ibkr,
}

impl fmt::Display for Exchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Exchange::Kalshi => write!(f, "kalshi"),
            Exchange::Ibkr => write!(f, "ibkr"),
        }
    }
}

impl FromStr for Exchange {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "kalshi" => Ok(Exchange::Kalshi),
            "ibkr" | "ib" | "interactive_brokers" => Ok(Exchange::Ibkr),
            _ => Err(format!("Unknown exchange: {s}")),
        }
    }
}

/// A unified position across exchanges.
#[derive(Debug, Clone, Serialize)]
pub struct Position {
    pub exchange: Exchange,
    pub ticker: String,
    pub asset_class: AssetClass,
    pub direction: String,
    pub quantity: f64,
    /// Cost basis in cents (Kalshi) or dollars (IBKR).
    pub cost_basis: f64,
    /// Current market value in same units as cost_basis.
    pub market_value: Option<f64>,
    /// Unrealized P&L in same units.
    pub unrealized_pnl: Option<f64>,
}

/// Concentration warning when an asset class exceeds a threshold.
#[derive(Debug, Clone, Serialize)]
pub struct ConcentrationWarning {
    pub asset_class: AssetClass,
    pub percentage: f64,
    pub exposure: f64,
    pub message: String,
}

/// Aggregated exposure for an asset class.
#[derive(Debug, Clone, Serialize)]
pub struct ClassExposure {
    pub asset_class: AssetClass,
    pub position_count: usize,
    pub total_exposure: f64,
    pub percentage: f64,
    pub exchanges: Vec<Exchange>,
}

/// Unified portfolio view across all exchanges.
#[derive(Debug, Clone, Serialize)]
pub struct Portfolio {
    pub positions: Vec<Position>,
    pub total_exposure: f64,
    pub total_unrealized_pnl: f64,
    pub class_exposures: Vec<ClassExposure>,
    pub warnings: Vec<ConcentrationWarning>,
}

impl Portfolio {
    /// Build a portfolio from a list of positions.
    /// `concentration_threshold` is the percentage (0.0–1.0) above which
    /// a warning is generated for an asset class.
    pub fn from_positions(positions: Vec<Position>, concentration_threshold: f64) -> Self {
        let total_exposure: f64 = positions.iter().map(|p| p.cost_basis.abs()).sum();
        let total_unrealized_pnl: f64 = positions.iter().filter_map(|p| p.unrealized_pnl).sum();

        // Group by asset class
        let mut class_map: HashMap<AssetClass, (usize, f64, Vec<Exchange>)> = HashMap::new();
        for pos in &positions {
            let entry = class_map
                .entry(pos.asset_class)
                .or_insert((0, 0.0, Vec::new()));
            entry.0 += 1;
            entry.1 += pos.cost_basis.abs();
            if !entry.2.contains(&pos.exchange) {
                entry.2.push(pos.exchange);
            }
        }

        let mut class_exposures: Vec<ClassExposure> = class_map
            .into_iter()
            .map(|(asset_class, (count, exposure, exchanges))| {
                let percentage = if total_exposure > 0.0 {
                    exposure / total_exposure
                } else {
                    0.0
                };
                ClassExposure {
                    asset_class,
                    position_count: count,
                    total_exposure: exposure,
                    percentage,
                    exchanges,
                }
            })
            .collect();

        // Sort by exposure descending
        class_exposures.sort_by(|a, b| {
            b.total_exposure
                .partial_cmp(&a.total_exposure)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Generate concentration warnings
        let warnings: Vec<ConcentrationWarning> = class_exposures
            .iter()
            .filter(|ce| ce.percentage > concentration_threshold)
            .map(|ce| ConcentrationWarning {
                asset_class: ce.asset_class,
                percentage: ce.percentage * 100.0,
                exposure: ce.total_exposure,
                message: format!(
                    "{} concentration at {:.1}% ({} positions, {:.0} exposure) — \
                     exceeds {:.0}% threshold",
                    ce.asset_class,
                    ce.percentage * 100.0,
                    ce.position_count,
                    ce.total_exposure,
                    concentration_threshold * 100.0,
                ),
            })
            .collect();

        Portfolio {
            positions,
            total_exposure,
            total_unrealized_pnl,
            class_exposures,
            warnings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kalshi_position(ticker: &str, direction: &str, cost: f64) -> Position {
        Position {
            exchange: Exchange::Kalshi,
            ticker: ticker.to_string(),
            asset_class: AssetClass::from_ticker(ticker),
            direction: direction.to_string(),
            quantity: 10.0,
            cost_basis: cost,
            market_value: None,
            unrealized_pnl: None,
        }
    }

    fn ibkr_position(ticker: &str, direction: &str, cost: f64, pnl: f64) -> Position {
        Position {
            exchange: Exchange::Ibkr,
            ticker: ticker.to_string(),
            asset_class: AssetClass::from_ticker(ticker),
            direction: direction.to_string(),
            quantity: 10.0,
            cost_basis: cost,
            market_value: Some(cost + pnl),
            unrealized_pnl: Some(pnl),
        }
    }

    #[test]
    fn test_asset_class_from_ticker() {
        assert_eq!(AssetClass::from_ticker("COIN"), AssetClass::Crypto);
        assert_eq!(AssetClass::from_ticker("MARA"), AssetClass::Crypto);
        assert_eq!(AssetClass::from_ticker("KXBTC-123"), AssetClass::Crypto);
        assert_eq!(AssetClass::from_ticker("SPY"), AssetClass::Equities);
        assert_eq!(AssetClass::from_ticker("AAPL"), AssetClass::Equities);
        assert_eq!(AssetClass::from_ticker("TLT"), AssetClass::Rates);
        assert_eq!(AssetClass::from_ticker("KXFED-123"), AssetClass::Rates);
        assert_eq!(AssetClass::from_ticker("KXHIGHNY-123"), AssetClass::Weather);
        assert_eq!(AssetClass::from_ticker("KXINXY-123"), AssetClass::Equities);
        assert_eq!(AssetClass::from_ticker("KXSOME-OTHER"), AssetClass::Events);
    }

    #[test]
    fn test_empty_portfolio() {
        let portfolio = Portfolio::from_positions(vec![], 0.5);
        assert!(portfolio.positions.is_empty());
        assert_eq!(portfolio.total_exposure, 0.0);
        assert!(portfolio.warnings.is_empty());
    }

    #[test]
    fn test_single_position() {
        let positions = vec![kalshi_position("KXBTC-123", "yes", 100.0)];
        let portfolio = Portfolio::from_positions(positions, 0.5);
        assert_eq!(portfolio.positions.len(), 1);
        assert_eq!(portfolio.total_exposure, 100.0);
        assert_eq!(portfolio.class_exposures.len(), 1);
        assert_eq!(portfolio.class_exposures[0].asset_class, AssetClass::Crypto);
        assert!((portfolio.class_exposures[0].percentage - 1.0).abs() < 0.01);
        // 100% concentration > 50% threshold → warning
        assert_eq!(portfolio.warnings.len(), 1);
    }

    #[test]
    fn test_multi_exchange_diversified() {
        let positions = vec![
            kalshi_position("KXBTC-123", "yes", 50.0),
            kalshi_position("KXHIGHNY-456", "yes", 30.0),
            ibkr_position("AAPL", "long", 200.0, 10.0),
            ibkr_position("TLT", "long", 100.0, -5.0),
        ];
        let portfolio = Portfolio::from_positions(positions, 0.5);
        assert_eq!(portfolio.positions.len(), 4);
        assert!((portfolio.total_exposure - 380.0).abs() < 0.01);
        assert!((portfolio.total_unrealized_pnl - 5.0).abs() < 0.01);
        assert_eq!(portfolio.class_exposures.len(), 4); // crypto, weather, equities, rates
                                                        // No single class > 50% (equities = 200/380 ≈ 52.6%) → 1 warning
        assert_eq!(portfolio.warnings.len(), 1);
        assert_eq!(portfolio.warnings[0].asset_class, AssetClass::Equities);
    }

    #[test]
    fn test_concentration_warning_threshold() {
        let positions = vec![
            kalshi_position("KXBTC-1", "yes", 80.0),
            kalshi_position("KXHIGHNY-1", "yes", 20.0),
        ];
        // At 70% threshold, crypto (80%) triggers warning
        let portfolio = Portfolio::from_positions(positions.clone(), 0.7);
        assert_eq!(portfolio.warnings.len(), 1);
        assert_eq!(portfolio.warnings[0].asset_class, AssetClass::Crypto);

        // At 90% threshold, no warnings
        let portfolio = Portfolio::from_positions(positions, 0.9);
        assert!(portfolio.warnings.is_empty());
    }

    #[test]
    fn test_correlated_cross_exchange() {
        // BTC on Kalshi + COIN on IBKR = both crypto
        let positions = vec![
            kalshi_position("KXBTC-123", "yes", 50.0),
            ibkr_position("COIN", "long", 500.0, 25.0),
        ];
        let portfolio = Portfolio::from_positions(positions, 0.5);
        assert_eq!(portfolio.class_exposures.len(), 1);
        assert_eq!(portfolio.class_exposures[0].asset_class, AssetClass::Crypto);
        assert_eq!(portfolio.class_exposures[0].position_count, 2);
        assert_eq!(portfolio.class_exposures[0].exchanges.len(), 2);
        // 100% in crypto > 50% → warning
        assert_eq!(portfolio.warnings.len(), 1);
    }

    #[test]
    fn test_exchange_from_str() {
        assert_eq!(Exchange::from_str("kalshi").unwrap(), Exchange::Kalshi);
        assert_eq!(Exchange::from_str("ibkr").unwrap(), Exchange::Ibkr);
        assert_eq!(Exchange::from_str("ib").unwrap(), Exchange::Ibkr);
        assert!(Exchange::from_str("unknown").is_err());
    }

    #[test]
    fn test_asset_class_from_str() {
        assert_eq!(AssetClass::from_str("crypto").unwrap(), AssetClass::Crypto);
        assert_eq!(
            AssetClass::from_str("stocks").unwrap(),
            AssetClass::Equities
        );
        assert_eq!(AssetClass::from_str("bonds").unwrap(), AssetClass::Rates);
        assert!(AssetClass::from_str("invalid").is_err());
    }
}
