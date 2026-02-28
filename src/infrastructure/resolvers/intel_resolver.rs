/// Intel-DB-backed market resolver.
///
/// Resolves tickers by querying recent feed entries in the intel database.
/// - Kalshi series (KXFED, KXBTC, etc.) → finds most liquid active contract
/// - Stock tickers (IONQ, NVDA, etc.) → finds latest Yahoo Finance price
///
/// This approach avoids external API calls during execution — all data comes
/// from feed entries already ingested by `openintel feed`.
use std::sync::Arc;

use crate::domain::ports::intel_repository::{IntelRepository, QueryFilter};
use crate::domain::ports::market_resolver::{Exchange, MarketResolver, ResolvedMarket};
use crate::domain::values::category::Category;

/// Known Kalshi series prefixes
const KALSHI_SERIES: &[&str] = &["KXHIGHNY", "KXINXY", "KXFED", "KXBTC"];

pub struct IntelResolver {
    intel_repo: Arc<dyn IntelRepository>,
}

impl IntelResolver {
    pub fn new(intel_repo: Arc<dyn IntelRepository>) -> Self {
        Self { intel_repo }
    }

    fn is_kalshi_series(&self, ticker: &str) -> bool {
        KALSHI_SERIES.iter().any(|s| ticker.starts_with(s))
    }

    /// Resolve a Kalshi series ticker by finding the most liquid active contract.
    async fn resolve_kalshi(&self, series: &str) -> Option<ResolvedMarket> {
        let filter = QueryFilter {
            category: Some(Category::Market),
            tag: Some("kalshi-feed".to_string()),
            limit: Some(200),
            ..Default::default()
        };

        let entries = self.intel_repo.query(&filter).ok()?;

        // Find individual contract entries (not band-sum entries) for this series
        let mut best_contract: Option<ResolvedMarket> = None;
        let mut best_volume: i64 = -1;

        for entry in &entries {
            // Skip band-sum aggregation entries
            if entry.tags.iter().any(|t| t == "band-sum") {
                continue;
            }

            // Must belong to this series
            let is_series = entry.tags.iter().any(|t| t == series);
            if !is_series {
                continue;
            }

            // Must have contract-level metadata
            let meta = match &entry.metadata {
                Some(m) => m,
                None => continue,
            };

            let contract_ticker = match meta.get("ticker").and_then(|v| v.as_str()) {
                Some(t) => t.to_string(),
                None => continue,
            };

            let midpoint = meta.get("midpoint").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if midpoint <= 0.0 || midpoint >= 100.0 {
                continue;
            }

            let volume = meta.get("volume_24h").and_then(|v| v.as_i64()).unwrap_or(0);
            let open_interest = meta
                .get("open_interest")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            // Score by liquidity: volume + open_interest
            let liquidity_score = volume + open_interest;

            // Prefer contracts with actual trading activity
            if liquidity_score > best_volume {
                let yes_bid = meta.get("yes_bid").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let yes_ask = meta
                    .get("yes_ask")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(100.0);

                best_contract = Some(ResolvedMarket {
                    contract_ticker: contract_ticker.clone(),
                    price_cents: midpoint,
                    exchange: Exchange::Kalshi,
                    description: format!(
                        "{} — bid {}¢ / ask {}¢ (OI: {}, Vol: {})",
                        contract_ticker, yes_bid, yes_ask, open_interest, volume
                    ),
                });
                best_volume = liquidity_score;
            }
        }

        best_contract
    }

    /// Resolve a stock ticker by finding the latest Yahoo Finance price.
    async fn resolve_stock(&self, ticker: &str) -> Option<ResolvedMarket> {
        let filter = QueryFilter {
            category: Some(Category::Market),
            tag: Some(ticker.to_string()),
            limit: Some(5),
            ..Default::default()
        };

        let entries = self.intel_repo.query(&filter).ok()?;

        // Find the most recent yahoo-feed entry for this ticker
        for entry in &entries {
            if !entry.tags.iter().any(|t| t == "yahoo-feed") {
                continue;
            }

            let meta = match &entry.metadata {
                Some(m) => m,
                None => continue,
            };

            let price = meta.get("price").and_then(|v| v.as_f64())?;

            return Some(ResolvedMarket {
                contract_ticker: ticker.to_string(),
                price_cents: price * 100.0, // Convert dollars to cents
                exchange: Exchange::Yahoo,
                description: format!("{} @ ${:.2} (from Yahoo Finance feed)", ticker, price),
            });
        }

        None
    }
}

impl MarketResolver for IntelResolver {
    async fn resolve(&self, ticker: &str) -> Option<ResolvedMarket> {
        if self.is_kalshi_series(ticker) {
            self.resolve_kalshi(ticker).await
        } else {
            self.resolve_stock(ticker).await
        }
    }

    fn name(&self) -> &str {
        "intel-db"
    }
}
