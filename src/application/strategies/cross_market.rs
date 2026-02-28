//! Cross-market arbitrage strategy.
//!
//! Detects mispricings across correlated markets:
//! - **Band sum arbitrage**: Kalshi band prices that don't sum to ~100%
//!   within a single expiry date
//! - **Cross-asset divergence**: When prediction market sentiment diverges
//!   from equity market signals (e.g., bullish BTC intel but COIN dropping)
//!
//! Relies on intel entries tagged with both a prediction market series
//! (e.g., "kalshi", "KXBTC") and correlated equity tickers (e.g., "COIN",
//! "MARA") to detect divergences.

use std::collections::{HashMap, HashSet};

use chrono::{NaiveDate, Utc};

use crate::domain::error::DomainError;
use crate::domain::ports::strategy::{DetectionContext, Direction, Opportunity, Strategy};

/// Maps prediction market series to correlated equity tickers.
/// Note: "rates" is intentionally merged into "fed" to avoid duplicate signals
/// when entries are tagged with both.
const CROSS_MARKET_PAIRS: &[(&str, &[&str])] = &[
    ("btc", &["COIN", "MARA", "RIOT", "MSTR", "BITO", "IBIT"]),
    ("eth", &["COIN", "ETHE"]),
    ("crypto", &["COIN", "MARA", "RIOT", "MSTR", "CRCL"]),
    ("fed", &["TLT", "SHY", "XLF", "KRE"]),
    // "rates" removed — identical to "fed", causes duplicate signals (#7)
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

/// A band contract with its price and metadata, grouped for sum analysis.
#[derive(Debug, Clone)]
struct BandEntry {
    entry_id: String,
    midpoint: f64,
}

/// Key for grouping band contracts: (series, expiry_date_string).
/// Bands are only summed within the same series AND same expiry date.
type BandGroupKey = (String, String);

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
    /// Detect when Kalshi band prices (from intel entries) don't sum to ~100%
    /// within a single expiry date.
    ///
    /// Only applies to **band contracts** (ticker contains "-B" prefix, e.g.,
    /// KXHIGHNY-26FEB27-B42.5). Threshold contracts (ticker contains "-T"
    /// prefix, e.g., KXFED-26MAR-T3.25) are cumulative and should NOT be
    /// summed — they represent P(rate > threshold), not mutually exclusive
    /// outcomes.
    ///
    /// Prices are extracted from metadata (`midpoint` field) when available,
    /// falling back to text extraction from the body.
    fn detect_band_sum_arbitrage(
        &self,
        ctx: &DetectionContext,
    ) -> Result<Vec<Opportunity>, DomainError> {
        let mut opportunities = Vec::new();

        // Group band contracts by (series, expiry_date)
        let mut band_groups: HashMap<BandGroupKey, Vec<BandEntry>> = HashMap::new();

        for entry in &ctx.entries {
            let tags_lower: Vec<String> = entry.tags.iter().map(|t| t.to_lowercase()).collect();

            // Find Kalshi series tags (KXBTC, KXHIGHNY, KXINXY, KXFED, etc.)
            for tag in &tags_lower {
                if !tag.starts_with("kx") && tag != "kalshi" {
                    continue;
                }

                let series = if tag == "kalshi" {
                    match tags_lower.iter().find(|t| t.starts_with("kx")) {
                        Some(kx_tag) => kx_tag.clone(),
                        None => continue,
                    }
                } else {
                    tag.clone()
                };

                // Try to extract from metadata first (preferred — structured data)
                if let Some(ref meta) = entry.metadata {
                    let ticker = meta
                        .get("ticker")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    // Skip threshold contracts — they're cumulative, not
                    // mutually exclusive. Band-sum only applies to B contracts.
                    if is_threshold_contract(ticker) {
                        continue;
                    }

                    // Extract expiry date from metadata
                    let expiry = extract_expiry_date(meta, ticker);
                    if expiry.is_empty() {
                        // No expiry info — skip to avoid cross-date contamination
                        continue;
                    }

                    // Use midpoint price from metadata
                    if let Some(midpoint) = meta.get("midpoint").and_then(|v| v.as_f64()) {
                        if midpoint > 0.0 && midpoint < 100.0 {
                            let key = (series.clone(), expiry);
                            band_groups.entry(key).or_default().push(BandEntry {
                                entry_id: entry.id.clone(),
                                midpoint,
                            });
                            continue;
                        }
                    }
                }

                // Fallback: extract prices from body text (legacy entries without metadata)
                let prices = extract_band_prices(&entry.body);
                if !prices.is_empty() {
                    // Without metadata we can't reliably determine expiry date,
                    // so use "unknown" — these will only group with other
                    // metadata-less entries from the same series.
                    let key = (series.clone(), "unknown".to_string());
                    for price in prices {
                        band_groups.entry(key.clone()).or_default().push(BandEntry {
                            entry_id: entry.id.clone(),
                            midpoint: price,
                        });
                    }
                }
            }
        }

        // Check each (series, expiry) group for band sum anomalies
        for ((series, expiry), bands) in &band_groups {
            // Need at least 2 bands to detect sum anomaly
            if bands.len() < 2 {
                continue;
            }

            let total: f64 = bands.iter().map(|b| b.midpoint).sum();
            let entry_ids: Vec<String> = bands
                .iter()
                .map(|b| b.entry_id.clone())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();

            // Bands should sum to ~100. Allow 5% tolerance for bid-ask spread.
            if (95.0..=105.0).contains(&total) {
                continue;
            }

            let deviation = (total - 100.0).abs();
            let edge_cents = deviation;
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

            let expiry_label = if expiry == "unknown" {
                String::new()
            } else {
                format!(" (expiry: {})", expiry)
            };

            let suggested_action = if total < 100.0 {
                format!(
                    "Buy all {} bands in {}{} (total cost {:.0}¢, expected value 100¢)",
                    bands.len(),
                    series.to_uppercase(),
                    expiry_label,
                    total
                )
            } else {
                format!(
                    "Fade/short overpriced bands in {}{} \
                     ({} bands total {:.0}¢ > 100¢, sell expensive bands)",
                    series.to_uppercase(),
                    expiry_label,
                    bands.len(),
                    total
                )
            };

            let score = Opportunity::compute_score(confidence, Some(edge_cents), None);

            opportunities.push(Opportunity {
                strategy: self.name().to_string(),
                signal_type: "band_sum_arbitrage".to_string(),
                title: format!(
                    "{} {} bands sum to {:.0}% ({} bands, deviation: {:.1}%)",
                    series.to_uppercase(),
                    expiry,
                    total,
                    bands.len(),
                    deviation
                ),
                description: format!(
                    "{} band prices for expiry {} ({} contracts) sum to {:.1}% \
                     instead of ~100%. This suggests {} across the band set.",
                    series.to_uppercase(),
                    expiry,
                    bands.len(),
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
                suggested_action: Some(suggested_action),
                supporting_entries: entry_ids,
                score,
                liquidity: None,
                market_price: None,
                suggested_size_cents: None,
                detected_at: Utc::now(),
            });
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

            for (theme, equities) in CROSS_MARKET_PAIRS {
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
                for equity in *equities {
                    let eq_lower = equity.to_lowercase();
                    if tags_lower.contains(&eq_lower) {
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

            // Need at least 2 signals per side to reduce noise from single entries
            if prediction_signals.len() < 2 || equity_signals.len() < 2 {
                continue;
            }

            let pred_sentiment: f64 = prediction_signals.iter().map(|s| s.sentiment).sum::<f64>()
                / prediction_signals.len() as f64;
            let eq_sentiment: f64 = equity_signals.iter().map(|s| s.sentiment).sum::<f64>()
                / equity_signals.len() as f64;

            let divergence = (pred_sentiment - eq_sentiment).abs();

            // Significant divergence: prediction and equity markets disagree
            if divergence > 0.5 {
                // Deduplicate entry IDs
                let entry_ids: Vec<String> = signals
                    .iter()
                    .map(|s| s.entry_id.clone())
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();
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
                    strategy: self.name().to_string(),
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

/// Check if a ticker represents a threshold contract (cumulative, not
/// mutually exclusive). Threshold tickers contain "-T" followed by a number
/// (e.g., "KXFED-26MAR-T3.25").
fn is_threshold_contract(ticker: &str) -> bool {
    // Match pattern: ...-T<number>...
    // Examples: KXFED-26MAR-T2.75, KXFED-26MAR-T3.00
    // Non-matches: KXHIGHNY-26FEB27-B42.5, empty string
    let upper = ticker.to_uppercase();
    upper
        .split('-')
        .any(|part| part.starts_with('T') && part[1..].parse::<f64>().is_ok())
}

/// Extract expiry date string from metadata or ticker.
///
/// Prefers `close_time` from metadata (ISO 8601 → "YYYY-MM-DD").
/// Falls back to extracting date component from ticker
/// (e.g., "KXHIGHNY-26FEB27-B42.5" → "26FEB27").
fn extract_expiry_date(meta: &serde_json::Value, ticker: &str) -> String {
    // Try close_time from metadata first
    if let Some(close_time) = meta.get("close_time").and_then(|v| v.as_str()) {
        // Parse ISO timestamp and extract just the date
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(close_time) {
            return dt.format("%Y-%m-%d").to_string();
        }
        // Try parsing just the date portion
        if close_time.len() >= 10 {
            if NaiveDate::parse_from_str(&close_time[..10], "%Y-%m-%d").is_ok() {
                return close_time[..10].to_string();
            }
        }
    }

    // Fallback: extract date component from ticker
    // Ticker format: SERIES-DATEPART-CONTRACT (e.g., KXHIGHNY-26FEB27-B42.5)
    let parts: Vec<&str> = ticker.split('-').collect();
    if parts.len() >= 3 {
        // The date part is typically the second segment (e.g., "26FEB27", "26MAR", "26DEC31H1600")
        return parts[1].to_string();
    }

    String::new()
}

/// Extract band prices from text. Looks for patterns like "28c", "28¢",
/// "28 cents", "bid 28", "ask 28".
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
        // Note: "at" removed — too generic, causes false positives (#4)
        if (*word == "bid" || *word == "ask") && i + 1 < words.len() {
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
    use crate::domain::entities::intel_entry::IntelEntry;
    use crate::domain::values::category::Category;
    use crate::domain::values::source_type::SourceType;

    fn make_band_entry(
        id: &str,
        ticker: &str,
        series: &str,
        midpoint: f64,
        close_time: &str,
    ) -> IntelEntry {
        IntelEntry {
            id: id.to_string(),
            category: Category::Market,
            title: format!("{} — {}¢", ticker, midpoint),
            body: format!("Bid: {}¢, Ask: {}¢, Midpoint: {}¢", midpoint - 0.5, midpoint + 0.5, midpoint),
            source: Some("kalshi".to_string()),
            tags: vec![
                ticker.to_string(),
                series.to_string(),
                "kalshi-feed".to_string(),
            ],
            confidence: crate::domain::values::confidence::Confidence::new(0.9).unwrap(),
            actionable: false,
            source_type: SourceType::External,
            metadata: Some(serde_json::json!({
                "ticker": ticker,
                "series": series,
                "midpoint": midpoint,
                "close_time": close_time,
                "yes_bid": midpoint - 0.5,
                "yes_ask": midpoint + 0.5,
            })),
            updated_at: Utc::now(),
            created_at: Utc::now(),
        }
    }

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
    fn test_extract_band_prices_at_not_extracted() {
        // "at" is too generic — should NOT extract prices
        let prices = extract_band_prices("looking at 45 contracts");
        assert!(prices.is_empty());
    }

    #[test]
    fn test_extract_band_prices_binary_market() {
        // Binary markets have exactly 2 bands (Yes/No)
        let prices = extract_band_prices("Yes 72c, No 30c");
        assert_eq!(prices.len(), 2);
        assert!((prices[0] - 72.0).abs() < 0.01);
        assert!((prices[1] - 30.0).abs() < 0.01);
    }

    #[test]
    fn test_is_threshold_contract() {
        assert!(is_threshold_contract("KXFED-26MAR-T2.75"));
        assert!(is_threshold_contract("KXFED-26MAR-T3.00"));
        assert!(is_threshold_contract("KXFED-26JUN-T4.50"));
        assert!(!is_threshold_contract("KXHIGHNY-26FEB27-B42.5"));
        assert!(!is_threshold_contract("KXINXY-26DEC31H1600-B7700"));
        assert!(!is_threshold_contract(""));
        assert!(!is_threshold_contract("KXFED")); // series tag, no contract
    }

    #[test]
    fn test_extract_expiry_from_close_time() {
        let meta = serde_json::json!({"close_time": "2026-03-18T17:55:00Z"});
        assert_eq!(extract_expiry_date(&meta, ""), "2026-03-18");
    }

    #[test]
    fn test_extract_expiry_from_ticker_fallback() {
        let meta = serde_json::json!({});
        assert_eq!(
            extract_expiry_date(&meta, "KXHIGHNY-26FEB27-B42.5"),
            "26FEB27"
        );
    }

    #[test]
    fn test_extract_expiry_empty_when_no_info() {
        let meta = serde_json::json!({});
        assert_eq!(extract_expiry_date(&meta, "KXFED"), "");
    }

    #[test]
    fn test_band_sum_groups_by_expiry_date() {
        let strategy = CrossMarketStrategy;

        // Create bands for TWO different expiry dates in the same series
        let entries = vec![
            // Expiry 1: Feb 27 — bands sum to 85 (underpriced)
            make_band_entry("e1", "KXHIGHNY-26FEB27-B38.5", "kxhighny", 10.0, "2026-02-28T04:59:00Z"),
            make_band_entry("e2", "KXHIGHNY-26FEB27-B40.5", "kxhighny", 25.0, "2026-02-28T04:59:00Z"),
            make_band_entry("e3", "KXHIGHNY-26FEB27-B42.5", "kxhighny", 30.0, "2026-02-28T04:59:00Z"),
            make_band_entry("e4", "KXHIGHNY-26FEB27-B44.5", "kxhighny", 20.0, "2026-02-28T04:59:00Z"),
            // Expiry 2: Feb 28 — bands sum to 110 (overpriced)
            make_band_entry("e5", "KXHIGHNY-26FEB28-B38.5", "kxhighny", 15.0, "2026-03-01T04:59:00Z"),
            make_band_entry("e6", "KXHIGHNY-26FEB28-B40.5", "kxhighny", 30.0, "2026-03-01T04:59:00Z"),
            make_band_entry("e7", "KXHIGHNY-26FEB28-B42.5", "kxhighny", 35.0, "2026-03-01T04:59:00Z"),
            make_band_entry("e8", "KXHIGHNY-26FEB28-B44.5", "kxhighny", 30.0, "2026-03-01T04:59:00Z"),
        ];

        let ctx = DetectionContext {
            entries,
            open_trades: vec![],
            window_hours: 48,
        };

        let result = strategy.detect_band_sum_arbitrage(&ctx).unwrap();

        // Should get 2 separate opportunities, NOT one combined 195% opportunity
        assert_eq!(result.len(), 2, "Expected 2 opportunities (one per expiry), got {}", result.len());

        // Find the underpriced one (85%)
        let underpriced = result.iter().find(|o| o.title.contains("85")).expect("Should have 85% opportunity");
        assert!(underpriced.suggested_action.as_ref().unwrap().contains("Buy"));
        assert!(underpriced.title.contains("4 bands"));

        // Find the overpriced one (110%)
        let overpriced = result.iter().find(|o| o.title.contains("110")).expect("Should have 110% opportunity");
        assert!(overpriced.suggested_action.as_ref().unwrap().contains("Fade"));
        assert!(overpriced.title.contains("4 bands"));
    }

    #[test]
    fn test_threshold_contracts_excluded_from_band_sum() {
        let strategy = CrossMarketStrategy;

        // KXFED threshold contracts — should NOT trigger band-sum arbitrage
        let entries = vec![
            make_band_entry("e1", "KXFED-26MAR-T2.75", "kxfed", 99.5, "2026-03-18T17:55:00Z"),
            make_band_entry("e2", "KXFED-26MAR-T3.00", "kxfed", 99.5, "2026-03-18T17:55:00Z"),
            make_band_entry("e3", "KXFED-26MAR-T3.25", "kxfed", 98.5, "2026-03-18T17:55:00Z"),
            make_band_entry("e4", "KXFED-26MAR-T3.50", "kxfed", 95.0, "2026-03-18T17:55:00Z"),
            make_band_entry("e5", "KXFED-26MAR-T3.75", "kxfed", 85.0, "2026-03-18T17:55:00Z"),
            make_band_entry("e6", "KXFED-26MAR-T4.00", "kxfed", 55.0, "2026-03-18T17:55:00Z"),
        ];

        let ctx = DetectionContext {
            entries,
            open_trades: vec![],
            window_hours: 48,
        };

        let result = strategy.detect_band_sum_arbitrage(&ctx).unwrap();

        // Threshold contracts should be completely excluded
        assert!(
            result.is_empty(),
            "Threshold contracts should not trigger band-sum arbitrage, got {} opportunities",
            result.len()
        );
    }

    #[test]
    fn test_band_sum_single_expiry_within_tolerance() {
        let strategy = CrossMarketStrategy;

        // Bands that sum to ~100 (within 95-105 tolerance)
        let entries = vec![
            make_band_entry("e1", "KXHIGHNY-26FEB27-B38.5", "kxhighny", 5.0, "2026-02-28T04:59:00Z"),
            make_band_entry("e2", "KXHIGHNY-26FEB27-B40.5", "kxhighny", 15.0, "2026-02-28T04:59:00Z"),
            make_band_entry("e3", "KXHIGHNY-26FEB27-B42.5", "kxhighny", 35.0, "2026-02-28T04:59:00Z"),
            make_band_entry("e4", "KXHIGHNY-26FEB27-B44.5", "kxhighny", 30.0, "2026-02-28T04:59:00Z"),
            make_band_entry("e5", "KXHIGHNY-26FEB27-B46.5", "kxhighny", 15.0, "2026-02-28T04:59:00Z"),
        ];

        let ctx = DetectionContext {
            entries,
            open_trades: vec![],
            window_hours: 48,
        };

        let result = strategy.detect_band_sum_arbitrage(&ctx).unwrap();
        // Sum = 100, within tolerance — no opportunity
        assert!(result.is_empty(), "Bands summing to 100 should not trigger, got {} opportunities", result.len());
    }

    #[test]
    fn test_band_sum_legacy_text_extraction() {
        let strategy = CrossMarketStrategy;
        // Entries without metadata (legacy) — uses text extraction
        let entries = vec![
            IntelEntry {
                id: "e1".to_string(),
                category: Category::Market,
                title: "KXBTC Yes band".to_string(),
                body: "Yes 40c".to_string(),
                source: None,
                tags: vec!["kxbtc".to_string()],
                confidence: crate::domain::values::confidence::Confidence::new(0.8).unwrap(),
                actionable: false,
                source_type: SourceType::External,
                metadata: None,
                updated_at: Utc::now(),
                created_at: Utc::now(),
            },
            IntelEntry {
                id: "e2".to_string(),
                category: Category::Market,
                title: "KXBTC No band".to_string(),
                body: "No 45c".to_string(),
                source: None,
                tags: vec!["kxbtc".to_string()],
                confidence: crate::domain::values::confidence::Confidence::new(0.8).unwrap(),
                actionable: false,
                source_type: SourceType::External,
                metadata: None,
                updated_at: Utc::now(),
                created_at: Utc::now(),
            },
        ];
        let ctx = DetectionContext {
            entries,
            open_trades: vec![],
            window_hours: 48,
        };
        let result = strategy.detect_band_sum_arbitrage(&ctx).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].signal_type, "band_sum_arbitrage");
        // 40 + 45 = 85 < 95 → underpriced
        assert!(result[0].suggested_action.as_ref().unwrap().contains("Buy"));
    }

    #[test]
    fn test_band_sum_overpriced_suggests_fade() {
        let strategy = CrossMarketStrategy;
        let entries = vec![
            IntelEntry {
                id: "e1".to_string(),
                category: Category::Market,
                title: "Band A".to_string(),
                body: "Yes 60c".to_string(),
                source: None,
                tags: vec!["kxbtc".to_string()],
                confidence: crate::domain::values::confidence::Confidence::new(0.8).unwrap(),
                actionable: false,
                source_type: SourceType::External,
                metadata: None,
                updated_at: Utc::now(),
                created_at: Utc::now(),
            },
            IntelEntry {
                id: "e2".to_string(),
                category: Category::Market,
                title: "Band B".to_string(),
                body: "No 50c".to_string(),
                source: None,
                tags: vec!["kxbtc".to_string()],
                confidence: crate::domain::values::confidence::Confidence::new(0.8).unwrap(),
                actionable: false,
                source_type: SourceType::External,
                metadata: None,
                updated_at: Utc::now(),
                created_at: Utc::now(),
            },
        ];
        let ctx = DetectionContext {
            entries,
            open_trades: vec![],
            window_hours: 48,
        };
        let result = strategy.detect_band_sum_arbitrage(&ctx).unwrap();
        assert_eq!(result.len(), 1);
        // 60 + 50 = 110 > 105 → overpriced
        assert!(result[0]
            .suggested_action
            .as_ref()
            .unwrap()
            .contains("Fade"));
    }

    #[test]
    fn test_entry_sentiment_bullish_tags() {
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
