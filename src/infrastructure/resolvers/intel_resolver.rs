/// Intel-DB-backed market resolver.
///
/// Resolves tickers by querying recent feed entries in the intel database.
/// - Kalshi series (prefix "KX") → finds most liquid active contract
/// - Stock tickers (IONQ, NVDA, etc.) → finds latest Yahoo Finance price
///
/// This approach avoids external API calls during execution — all data comes
/// from feed entries already ingested by `openintel feed`.
///
/// Note: This resolver is used as a concrete type, not `dyn MarketResolver`.
/// The `async_fn_in_trait` allowance in the port reflects this design choice.
use std::sync::Arc;

use chrono::{Duration, Utc};

use crate::domain::ports::intel_repository::{IntelRepository, QueryFilter};
use crate::domain::ports::market_resolver::{Exchange, MarketResolver, ResolvedMarket};
use crate::domain::values::category::Category;

/// Maximum age of feed data to consider (in hours).
/// Prices older than this are treated as stale and ignored.
const MAX_FEED_AGE_HOURS: i64 = 4;

pub struct IntelResolver {
    intel_repo: Arc<dyn IntelRepository>,
}

impl IntelResolver {
    pub fn new(intel_repo: Arc<dyn IntelRepository>) -> Self {
        Self { intel_repo }
    }

    /// Detect Kalshi tickers dynamically by prefix.
    /// Kalshi series all use the "KX" prefix (e.g., KXHIGHNY, KXFED, KXBTC).
    /// If a future equity ticker starts with "KX", this would misroute it —
    /// unlikely but documented here for awareness.
    fn is_kalshi_ticker(&self, ticker: &str) -> bool {
        ticker.starts_with("KX")
    }

    /// Resolve a Kalshi series ticker by finding the most liquid active contract.
    async fn resolve_kalshi(&self, series: &str) -> Option<ResolvedMarket> {
        let filter = QueryFilter {
            category: Some(Category::Market),
            tag: Some("kalshi-feed".to_string()),
            since: Some(Utc::now() - Duration::hours(MAX_FEED_AGE_HOURS)),
            limit: Some(200),
            ..Default::default()
        };

        let entries = self
            .intel_repo
            .query(&filter)
            .map_err(|e| {
                eprintln!("IntelResolver: DB query failed for Kalshi series {series}: {e}");
                e
            })
            .ok()?;

        // Find individual contract entries (not band-sum entries) for this series
        let mut best_contract: Option<ResolvedMarket> = None;
        let mut best_volume: i64 = 0; // Require at least some liquidity

        for entry in &entries {
            // Skip band-sum aggregation entries
            if entry.tags.iter().any(|t| t == "band-sum") {
                continue;
            }

            // Must belong to this series
            if !entry.tags.iter().any(|t| t == series) {
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

        if best_contract.is_none() {
            eprintln!(
                "IntelResolver: No liquid Kalshi contract found for series {series} (within {}h)",
                MAX_FEED_AGE_HOURS
            );
        }

        best_contract
    }

    /// Resolve a stock ticker by finding the latest Yahoo Finance price.
    ///
    /// Note: Stock prices are returned in dollar-cents (e.g., IONQ at $38 → 3800).
    /// These should NOT be fed into Kalshi-style Kelly sizing (which expects 1–99
    /// binary contract prices). The execute pipeline only uses Kelly for Kalshi
    /// contracts; stock prices are informational for IBKR position sizing.
    async fn resolve_stock(&self, ticker: &str) -> Option<ResolvedMarket> {
        // Query by yahoo-feed tag and filter by ticker in-memory.
        // This avoids missing Yahoo entries when many non-Yahoo entries
        // share the same ticker tag (e.g., strategy-generated entries).
        let filter = QueryFilter {
            category: Some(Category::Market),
            tag: Some("yahoo-feed".to_string()),
            since: Some(Utc::now() - Duration::hours(MAX_FEED_AGE_HOURS)),
            limit: Some(50),
            ..Default::default()
        };

        let entries = self
            .intel_repo
            .query(&filter)
            .map_err(|e| {
                eprintln!("IntelResolver: DB query failed for stock {ticker}: {e}");
                e
            })
            .ok()?;

        // Find the most recent yahoo-feed entry for this ticker.
        // Results are ordered newest-first (SQLite ORDER BY created_at DESC).
        for entry in &entries {
            if !entry.tags.iter().any(|t| t == ticker) {
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
                exchange: Exchange::Equity,
                description: format!("{} @ ${:.2} (from Yahoo Finance feed)", ticker, price),
            });
        }

        eprintln!(
            "IntelResolver: No fresh price found for stock {ticker} (within {}h)",
            MAX_FEED_AGE_HOURS
        );
        None
    }
}

impl MarketResolver for IntelResolver {
    async fn resolve(&self, ticker: &str) -> Option<ResolvedMarket> {
        let ticker = ticker.trim().to_uppercase();
        if self.is_kalshi_ticker(&ticker) {
            self.resolve_kalshi(&ticker).await
        } else {
            self.resolve_stock(&ticker).await
        }
    }

    fn name(&self) -> &str {
        "intel-db"
    }
}
