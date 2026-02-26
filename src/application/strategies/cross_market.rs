//! Cross-market arbitrage strategy.
//!
//! Detects mispricings across correlated markets:
//! - **Band sum arbitrage**: Kalshi band prices that don't sum to ~100%
//! - **Cross-asset divergence**: When prediction market sentiment diverges
//!   from equity market signals (e.g., bullish BTC intel but COIN dropping)
//!
//! Relies on intel entries tagged with both a prediction market series
//! (e.g., "kalshi", "KXBTC") and correlated equity tickers (e.g., "COIN",
//! "MARA") to detect divergences.

use std::collections::HashMap;

use chrono::Utc;

use crate::domain::error::DomainError;
use crate::domain::ports::strategy::{DetectionContext, Direction, Opportunity, Strategy};

/// Maps prediction market series to correlated equity tickers.
const CROSS_MARKET_PAIRS: &[(&str, &[&str])] = &[
    ("btc", &["COIN", "MARA", "RIOT", "MSTR", "BITO", "IBIT"]),
    ("eth", &["COIN", "ETHE"]),
    ("crypto", &["COIN", "MARA", "RIOT", "MSTR", "CRCL"]),
    ("fed", &["TLT", "SHY", "XLF", "KRE"]),
    ("rates", &["TLT", "SHY", "XLF", "KRE"]),
    ("s&p500", &["SPY", "VOO", "IVV"]),
    ("nasdaq", &["QQQ", "TQQQ"]),
];

/// Detects cross-market mispricings and arbitrage opportunities.
pub struct CrossMarketStrategy;

/// A signal extracted from an intel entry about a specific market.
#[derive(Debug)]
struct MarketSignal {
    /// The asset or market this signal is about.
    market: String,
    /// Bullish (+1.0) to bearish (-1.0) sentiment.
    sentiment: f64,
    /// Source entry ID.
    entry_id: String,
    /// Confidence from the source entry.
    confidence: f64,
}

impl Strategy for CrossMarketStrategy {
    fn name(&self) -> &'static str {
        "cross_market"
    }

    fn detect(&self, ctx: &DetectionContext) -> Result<Vec<Opportunity>, DomainError> {
        let mut opportunities = Vec::new();

        // Phase 1: Band sum arbitrage detection
        opportunities.extend(self.detect_band_sum_arbitrage(ctx)?);

        // Phase 2: Cross-asset divergence detection
        opportunities.extend(self.detect_cross_asset_divergence(ctx)?);

        Ok(opportunities)
    }
}

impl CrossMarketStrategy {
    /// Detect when Kalshi band prices (from intel entries) don't sum to ~100%.
    ///
    /// Looks for intel entries tagged with a Kalshi series (e.g., "KXBTC",
    /// "KXHIGHNY") that contain band price data in their body/metadata.
    fn detect_band_sum_arbitrage(
        &self,
        ctx: &DetectionContext,
    ) -> Result<Vec<Opportunity>, DomainError> {
        let mut opportunities = Vec::new();

        // Group Kalshi-tagged entries by series
        let mut series_entries: HashMap<String, Vec<(String, String, f64)>> = HashMap::new();

        for entry in &ctx.entries {
            let tags_lower: Vec<String> = entry.tags.iter().map(|t| t.to_lowercase()).collect();

            // Find Kalshi series tags (KXBTC, KXHIGHNY, KXINXY, KXFED, etc.)
            for tag in &tags_lower {
                if tag.starts_with("kx") || tag == "kalshi" {
                    // Look for price data in the body: patterns like "XXc bid", "XX¢"
                    let prices = extract_band_prices(&entry.body);
                    if !prices.is_empty() {
                        let series = if tag == "kalshi" {
                            // Try to find a more specific series tag
                            tags_lower
                                .iter()
                                .find(|t| t.starts_with("kx"))
                                .cloned()
                                .unwrap_or_else(|| "kalshi".to_string())
                        } else {
                            tag.clone()
                        };

                        for price in prices {
                            series_entries.entry(series.clone()).or_default().push((
                                entry.id.clone(),
                                entry.title.clone(),
                                price,
                            ));
                        }
                    }
                }
            }
        }

        // Check each series for band sum anomalies
        for (series, entries) in &series_entries {
            if entries.len() < 3 {
                continue; // Need multiple bands to detect sum anomaly
            }

            let total: f64 = entries.iter().map(|(_, _, p)| p).sum();
            let entry_ids: Vec<String> = entries.iter().map(|(id, _, _)| id.clone()).collect();

            // Bands should sum to ~100. Allow 5% tolerance for bid-ask spread.
            if !(85.0..=115.0).contains(&total) {
                let deviation = (total - 100.0).abs();
                let edge_cents = deviation; // Each cent of deviation is a cent of edge
                let confidence = if deviation > 10.0 {
                    0.8
                } else if deviation > 5.0 {
                    0.6
                } else {
                    0.4
                };

                let direction = if total < 100.0 {
                    Direction::Yes // Bands underpriced — buy the set
                } else {
                    Direction::No // Bands overpriced — sell/fade
                };

                let score = Opportunity::compute_score(confidence, Some(edge_cents), None);

                opportunities.push(Opportunity {
                    strategy: "cross_market".to_string(),
                    signal_type: "band_sum_arbitrage".to_string(),
                    title: format!(
                        "{} bands sum to {:.0}% (deviation: {:.1}%)",
                        series.to_uppercase(),
                        total,
                        deviation
                    ),
                    description: format!(
                        "{} band prices from {} entries sum to {:.1}% instead of ~100%. \
                         This suggests {} across the band set.",
                        series.to_uppercase(),
                        entries.len(),
                        total,
                        if total < 100.0 {
                            "underpricing"
                        } else {
                            "overpricing"
                        }
                    ),
                    confidence,
                    edge_cents: Some(edge_cents),
                    market_ticker: Some(series.to_uppercase()),
                    suggested_direction: Some(direction),
                    suggested_action: Some(format!(
                        "Buy all bands in {} series (total cost {:.0}¢, expected value 100¢)",
                        series.to_uppercase(),
                        total
                    )),
                    supporting_entries: entry_ids,
                    score,
                    liquidity: None,
                    market_price: None,
                    suggested_size_cents: None,
                    detected_at: Utc::now(),
                });
            }
        }

        Ok(opportunities)
    }

    /// Detect when prediction market sentiment diverges from equity signals.
    ///
    /// Example: If intel shows bullish BTC sentiment but COIN-tagged entries
    /// are bearish, that's a divergence worth investigating.
    fn detect_cross_asset_divergence(
        &self,
        ctx: &DetectionContext,
    ) -> Result<Vec<Opportunity>, DomainError> {
        let mut opportunities = Vec::new();

        // Collect sentiment signals grouped by market theme
        let mut theme_signals: HashMap<String, Vec<MarketSignal>> = HashMap::new();

        for entry in &ctx.entries {
            let tags_lower: Vec<String> = entry.tags.iter().map(|t| t.to_lowercase()).collect();
            let sentiment = entry_sentiment(entry);

            for (theme, _equities) in CROSS_MARKET_PAIRS {
                if tags_lower.contains(&theme.to_string()) {
                    theme_signals
                        .entry(theme.to_string())
                        .or_default()
                        .push(MarketSignal {
                            market: format!("{}_prediction", theme),
                            sentiment,
                            entry_id: entry.id.clone(),
                            confidence: entry.confidence.value(),
                        });
                }

                // Check if entry mentions correlated equities
                for equity in *_equities {
                    let eq_lower = equity.to_lowercase();
                    if tags_lower.contains(&eq_lower) || entry.tags.iter().any(|t| t == *equity) {
                        theme_signals
                            .entry(theme.to_string())
                            .or_default()
                            .push(MarketSignal {
                                market: format!("{}_equity", equity),
                                sentiment,
                                entry_id: entry.id.clone(),
                                confidence: entry.confidence.value(),
                            });
                    }
                }
            }
        }

        // For each theme, check if prediction and equity signals diverge
        for (theme, signals) in &theme_signals {
            let prediction_signals: Vec<&MarketSignal> = signals
                .iter()
                .filter(|s| s.market.ends_with("_prediction"))
                .collect();
            let equity_signals: Vec<&MarketSignal> = signals
                .iter()
                .filter(|s| s.market.ends_with("_equity"))
                .collect();

            if prediction_signals.is_empty() || equity_signals.is_empty() {
                continue; // Need both sides to detect divergence
            }

            let pred_sentiment: f64 = prediction_signals.iter().map(|s| s.sentiment).sum::<f64>()
                / prediction_signals.len() as f64;
            let eq_sentiment: f64 = equity_signals.iter().map(|s| s.sentiment).sum::<f64>()
                / equity_signals.len() as f64;

            let divergence = (pred_sentiment - eq_sentiment).abs();

            // Significant divergence: prediction and equity markets disagree
            if divergence > 0.5 {
                let entry_ids: Vec<String> = signals.iter().map(|s| s.entry_id.clone()).collect();
                let avg_confidence: f64 =
                    signals.iter().map(|s| s.confidence).sum::<f64>() / signals.len() as f64;

                let confidence = (avg_confidence * divergence).clamp(0.1, 0.9);
                let score = Opportunity::compute_score(confidence, None, None);

                // The equity side is typically slower to adjust, so trade
                // in the direction of prediction market sentiment
                let direction = if pred_sentiment > 0.0 {
                    Direction::Bullish
                } else {
                    Direction::Bearish
                };

                let equities: &[&str] = CROSS_MARKET_PAIRS
                    .iter()
                    .find(|(t, _)| *t == theme.as_str())
                    .map(|(_, eq)| *eq)
                    .unwrap_or(&[]);

                opportunities.push(Opportunity {
                    strategy: "cross_market".to_string(),
                    signal_type: "cross_asset_divergence".to_string(),
                    title: format!(
                        "{} divergence: prediction {} vs equity {} ({:.0}% gap)",
                        theme.to_uppercase(),
                        if pred_sentiment > 0.0 {
                            "bullish"
                        } else {
                            "bearish"
                        },
                        if eq_sentiment > 0.0 {
                            "bullish"
                        } else {
                            "bearish"
                        },
                        divergence * 100.0
                    ),
                    description: format!(
                        "{} prediction market sentiment ({:+.2}) diverges from equity \
                         signals ({:+.2}). Correlated equities: {}. \
                         {} entries analyzed.",
                        theme.to_uppercase(),
                        pred_sentiment,
                        eq_sentiment,
                        equities.join(", "),
                        signals.len()
                    ),
                    confidence,
                    edge_cents: None,
                    market_ticker: equities.first().map(|e| (*e).to_string()),
                    suggested_direction: Some(direction),
                    suggested_action: None,
                    supporting_entries: entry_ids,
                    score,
                    liquidity: None,
                    market_price: None,
                    suggested_size_cents: None,
                    detected_at: Utc::now(),
                });
            }
        }

        Ok(opportunities)
    }
}

/// Extract band prices from text. Looks for patterns like "28c", "28¢",
/// "28 cents", "bid 28", "ask 28", "at 28c".
fn extract_band_prices(text: &str) -> Vec<f64> {
    let mut prices = Vec::new();
    let text_lower = text.to_lowercase();

    // Match patterns: NNc, NN¢, NN cents, bid NN, ask NN
    let words: Vec<&str> = text_lower.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        // "28c" or "28¢" pattern
        let stripped = word.trim_end_matches([',', '.', ';', ')']);
        let trimmed = stripped.trim_end_matches('c').trim_end_matches('¢');
        if let Ok(price) = trimmed.parse::<f64>() {
            if price > 0.0
                && price < 100.0
                && (stripped.ends_with('c')
                    || stripped.ends_with('¢')
                    || stripped.ends_with("cents"))
            {
                prices.push(price);
                continue;
            }
        }

        // "bid 28" or "ask 28" pattern — only when next word is a bare number
        // (skip if next word already ends in 'c'/'¢' since the direct pattern handles that)
        if (*word == "bid" || *word == "ask" || *word == "at") && i + 1 < words.len() {
            let next_raw = words[i + 1].trim_end_matches([',', '.', ';', ')']);
            let has_unit_suffix = next_raw.ends_with('c') || next_raw.ends_with('¢');
            if !has_unit_suffix {
                if let Ok(price) = next_raw.parse::<f64>() {
                    if price > 0.0 && price < 100.0 {
                        prices.push(price);
                    }
                }
            }
        }
    }

    prices
}

/// Derive sentiment from an intel entry. Returns -1.0 to +1.0.
fn entry_sentiment(entry: &crate::domain::entities::intel_entry::IntelEntry) -> f64 {
    // Use explicit sentiment if available in metadata
    if let Some(ref meta) = entry.metadata {
        if let Some(sentiment) = meta.get("sentiment").and_then(|v| v.as_str()) {
            return match sentiment {
                "bullish" | "positive" => 1.0,
                "bearish" | "negative" => -1.0,
                "mixed" | "neutral" => 0.0,
                _ => 0.0,
            };
        }
    }

    // Fallback: keyword-based sentiment from tags
    let tags_lower: Vec<String> = entry.tags.iter().map(|t| t.to_lowercase()).collect();
    let bullish_keywords = ["bullish", "beat", "rally", "surge", "momentum", "growth"];
    let bearish_keywords = ["bearish", "miss", "crash", "decline", "loss", "warning"];

    let bull_count = tags_lower
        .iter()
        .filter(|t| bullish_keywords.iter().any(|k| t.contains(k)))
        .count() as f64;
    let bear_count = tags_lower
        .iter()
        .filter(|t| bearish_keywords.iter().any(|k| t.contains(k)))
        .count() as f64;

    let total = bull_count + bear_count;
    if total == 0.0 {
        return 0.0;
    }

    (bull_count - bear_count) / total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_band_prices_cents() {
        let prices = extract_band_prices("B38.5 at 28c, B40.5 at 35c, T43 at 9c");
        assert_eq!(prices.len(), 3);
        assert!((prices[0] - 28.0).abs() < 0.01);
        assert!((prices[1] - 35.0).abs() < 0.01);
        assert!((prices[2] - 9.0).abs() < 0.01);
    }

    #[test]
    fn test_extract_band_prices_bid_ask() {
        let prices = extract_band_prices("bid 22 ask 23");
        assert_eq!(prices.len(), 2);
        assert!((prices[0] - 22.0).abs() < 0.01);
        assert!((prices[1] - 23.0).abs() < 0.01);
    }

    #[test]
    fn test_extract_band_prices_empty() {
        let prices = extract_band_prices("No price data here at all");
        assert!(prices.is_empty());
    }

    #[test]
    fn test_extract_band_prices_ignores_out_of_range() {
        let prices = extract_band_prices("price 150c and 0c");
        assert!(prices.is_empty());
    }

    #[test]
    fn test_entry_sentiment_bullish_tags() {
        use crate::domain::entities::intel_entry::IntelEntry;
        use crate::domain::values::category::Category;
        use crate::domain::values::source_type::SourceType;

        let entry = IntelEntry {
            id: "test".to_string(),
            category: Category::Market,
            title: "BTC Rally".to_string(),
            body: "test".to_string(),
            source: None,
            tags: vec![
                "btc".to_string(),
                "rally".to_string(),
                "momentum".to_string(),
            ],
            confidence: crate::domain::values::confidence::Confidence::new(0.8).unwrap(),
            actionable: false,
            source_type: SourceType::External,
            metadata: None,
            updated_at: Utc::now(),
            created_at: Utc::now(),
        };

        let sentiment = entry_sentiment(&entry);
        assert!(
            sentiment > 0.0,
            "Expected positive sentiment, got {}",
            sentiment
        );
    }

    #[test]
    fn test_entry_sentiment_metadata_override() {
        use crate::domain::entities::intel_entry::IntelEntry;
        use crate::domain::values::category::Category;
        use crate::domain::values::source_type::SourceType;

        let entry = IntelEntry {
            id: "test".to_string(),
            category: Category::Market,
            title: "Mixed signals".to_string(),
            body: "test".to_string(),
            source: None,
            tags: vec!["rally".to_string()], // bullish tag
            confidence: crate::domain::values::confidence::Confidence::new(0.8).unwrap(),
            actionable: false,
            source_type: SourceType::External,
            metadata: Some(serde_json::json!({"sentiment": "bearish"})),
            updated_at: Utc::now(),
            created_at: Utc::now(),
        };

        let sentiment = entry_sentiment(&entry);
        // Metadata should override tag-based sentiment
        assert!(
            (sentiment - (-1.0)).abs() < 0.01,
            "Expected -1.0 from metadata, got {}",
            sentiment
        );
    }

    #[test]
    fn test_cross_market_strategy_empty_context() {
        let strategy = CrossMarketStrategy;
        let ctx = DetectionContext {
            entries: vec![],
            open_trades: vec![],
            window_hours: 48,
        };
        let result = strategy.detect(&ctx).unwrap();
        assert!(result.is_empty());
    }
}
