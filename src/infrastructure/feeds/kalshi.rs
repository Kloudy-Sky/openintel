use super::{Feed, FeedError, FetchOutput};
use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::values::category::Category;
use crate::domain::values::confidence::Confidence;
use crate::domain::values::source_type::SourceType;
use async_trait::async_trait;

/// Kalshi market data feed. Fetches current pricing for tracked series.
/// Uses the public markets endpoint (no auth required for market data).
pub struct KalshiFeed {
    /// Series tickers to track (e.g., KXHIGHNY, KXINXY, KXBTC)
    series: Vec<String>,
    /// Base URL for Kalshi API
    base_url: String,
    client: reqwest::Client,
}

impl KalshiFeed {
    pub fn new(series: Vec<String>) -> Self {
        Self {
            series,
            base_url: "https://api.elections.kalshi.com/trade-api/v2".into(),
            client: reqwest::Client::builder()
                .user_agent("OpenIntel/0.1")
                .build()
                .unwrap_or_default(),
        }
    }

    /// Default series for our trading setup.
    pub fn default_series() -> Self {
        Self::new(vec![
            "KXHIGHNY".into(),
            "KXINXY".into(),
            "KXBTC".into(),
            "KXFED".into(),
        ])
    }
}

#[derive(Debug, serde::Deserialize)]
struct MarketsResponse {
    markets: Vec<KalshiMarket>,
}

#[derive(Debug, serde::Deserialize)]
struct KalshiMarket {
    ticker: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    yes_bid: Option<i64>,
    #[serde(default)]
    yes_ask: Option<i64>,
    #[serde(default)]
    volume_24h: Option<i64>,
    #[serde(default)]
    open_interest: Option<i64>,
    #[serde(default)]
    close_time: Option<String>,
}

#[async_trait]
impl Feed for KalshiFeed {
    fn name(&self) -> &str {
        "kalshi"
    }

    async fn fetch(&self) -> Result<FetchOutput, FeedError> {
        let mut all_entries = Vec::new();
        let mut fetch_errors = Vec::new();

        for series in &self.series {
            match self.fetch_series(series).await {
                Ok(entries) => all_entries.extend(entries),
                Err(e) => {
                    let msg = format!("{series}: {e}");
                    eprintln!("Warning: Failed to fetch {msg}");
                    fetch_errors.push(msg);
                }
            }
        }

        Ok(FetchOutput {
            entries: all_entries,
            fetch_errors,
        })
    }
}

impl KalshiFeed {
    async fn fetch_series(&self, series_ticker: &str) -> Result<Vec<IntelEntry>, FeedError> {
        let resp = self
            .client
            .get(format!("{}/markets", self.base_url))
            .query(&[
                ("series_ticker", series_ticker),
                ("limit", "20"),
                ("status", "open"),
            ])
            .send()
            .await
            .map_err(|e| FeedError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(FeedError::Network(format!(
                "Kalshi API returned {} for {}",
                resp.status(),
                series_ticker
            )));
        }

        let data: MarketsResponse = resp
            .json()
            .await
            .map_err(|e| FeedError::Parse(e.to_string()))?;

        // Generate individual market entries
        let mut entries: Vec<IntelEntry> = data
            .markets
            .iter()
            .filter_map(|m| {
                let yes_bid = m.yes_bid.unwrap_or(0);
                let yes_ask = m.yes_ask.unwrap_or(0);

                // Skip markets with no pricing
                if yes_bid == 0 && yes_ask == 0 {
                    return None;
                }

                let midpoint = if yes_bid > 0 && yes_ask > 0 {
                    (yes_bid + yes_ask) as f64 / 2.0
                } else if yes_ask > 0 {
                    yes_ask as f64
                } else {
                    yes_bid as f64
                };

                let title_text = m
                    .title
                    .as_deref()
                    .or(m.subtitle.as_deref())
                    .unwrap_or(&m.ticker);

                let title = format!("{} — {}¢/{}", m.ticker, midpoint, yes_ask.max(yes_bid));

                let mut body = format!(
                    "{title_text}. Bid: {yes_bid}¢, Ask: {yes_ask}¢, Midpoint: {midpoint:.0}¢"
                );

                if let Some(vol) = m.volume_24h {
                    body.push_str(&format!(". 24h volume: {vol}"));
                }
                if let Some(oi) = m.open_interest {
                    body.push_str(&format!(". Open interest: {oi}"));
                }

                let mut tags = vec![
                    m.ticker.clone(),
                    series_ticker.to_string(),
                    "kalshi-feed".to_string(),
                ];

                // Tag by price range (cheap contracts = speculative)
                if midpoint < 10.0 {
                    tags.push("speculative".to_string());
                } else if midpoint > 80.0 {
                    tags.push("high-confidence-market".to_string());
                }

                Some(IntelEntry::new(
                    Category::Market,
                    title,
                    body,
                    Some("kalshi".to_string()),
                    tags,
                    Confidence::new(0.9).unwrap(), // Market data is factual
                    false,
                    SourceType::External,
                    Some(serde_json::json!({
                        "ticker": m.ticker,
                        "series": series_ticker,
                        "yes_bid": yes_bid,
                        "yes_ask": yes_ask,
                        "midpoint": midpoint,
                        "volume_24h": m.volume_24h,
                        "open_interest": m.open_interest,
                        "close_time": m.close_time,
                    })),
                ))
            })
            .collect();

        // Band sum analysis — detect arbitrage
        let priced_markets: Vec<f64> = data
            .markets
            .iter()
            .filter_map(|m| {
                let bid = m.yes_bid.unwrap_or(0);
                let ask = m.yes_ask.unwrap_or(0);
                if bid == 0 && ask == 0 {
                    return None;
                }
                Some(if bid > 0 && ask > 0 {
                    (bid + ask) as f64 / 2.0
                } else {
                    bid.max(ask) as f64
                })
            })
            .collect();

        if priced_markets.len() >= 2 {
            let band_sum: f64 = priced_markets.iter().sum();
            let priced_count = priced_markets.len();

            let deviation = (band_sum - 100.0).abs();
            if deviation > 5.0 {
                let direction = if band_sum > 105.0 {
                    "overpriced"
                } else {
                    "underpriced"
                };

                entries.push(IntelEntry::new(
                    Category::Market,
                    format!(
                        "{series_ticker} bands sum to {band_sum:.0}¢ — {direction} ({deviation:.0}¢ deviation)"
                    ),
                    format!(
                        "{priced_count} priced markets in {series_ticker} sum to {band_sum:.0}¢ vs expected 100¢. \
                         Deviation: {deviation:.0}¢. Direction: {direction}. \
                         Potential band arbitrage opportunity."
                    ),
                    Some("kalshi".to_string()),
                    vec![
                        series_ticker.to_string(),
                        "kalshi-feed".to_string(),
                        "band-sum".to_string(),
                        "arbitrage".to_string(),
                        direction.to_string(),
                    ],
                    Confidence::new(0.8).unwrap(),
                    true, // Arb opportunities are actionable
                    SourceType::External,
                    Some(serde_json::json!({
                        "series": series_ticker,
                        "band_sum": band_sum,
                        "deviation": deviation,
                        "priced_count": priced_count,
                        "direction": direction,
                    })),
                ));
            }
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kalshi_feed_creation() {
        let feed = KalshiFeed::new(vec!["KXHIGHNY".into()]);
        assert_eq!(feed.name(), "kalshi");
        assert_eq!(feed.series.len(), 1);
    }

    #[test]
    fn test_default_series() {
        let feed = KalshiFeed::default_series();
        assert_eq!(feed.series.len(), 4);
        assert!(feed.series.contains(&"KXHIGHNY".to_string()));
        assert!(feed.series.contains(&"KXINXY".to_string()));
    }
}
