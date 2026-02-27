use super::{Feed, FeedError};
use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::values::category::Category;
use crate::domain::values::confidence::Confidence;
use crate::domain::values::source_type::SourceType;
use async_trait::async_trait;

/// Yahoo Finance quote feed using the v8 chart API (no auth required).
pub struct YahooFeed {
    tickers: Vec<String>,
    client: reqwest::Client,
}

impl YahooFeed {
    pub fn new(tickers: Vec<String>) -> Self {
        Self {
            tickers,
            client: reqwest::Client::builder()
                .user_agent(
                    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                     AppleWebKit/537.36 (KHTML, like Gecko) \
                     Chrome/120.0.0.0 Safari/537.36",
                )
                .build()
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct ChartResponse {
    chart: ChartResult,
}

#[derive(Debug, serde::Deserialize)]
struct ChartResult {
    result: Option<Vec<ChartData>>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
struct ChartData {
    meta: ChartMeta,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChartMeta {
    symbol: String,
    #[serde(default)]
    short_name: Option<String>,
    #[serde(default)]
    long_name: Option<String>,
    #[serde(default)]
    regular_market_price: Option<f64>,
    #[serde(default)]
    chart_previous_close: Option<f64>,
    #[serde(default)]
    regular_market_volume: Option<u64>,
    #[serde(default)]
    fifty_two_week_high: Option<f64>,
    #[serde(default)]
    fifty_two_week_low: Option<f64>,
    #[serde(default)]
    regular_market_day_high: Option<f64>,
    #[serde(default)]
    regular_market_day_low: Option<f64>,
}

#[async_trait]
impl Feed for YahooFeed {
    fn name(&self) -> &str {
        "yahoo_finance"
    }

    async fn fetch(&self) -> Result<Vec<IntelEntry>, FeedError> {
        if self.tickers.is_empty() {
            return Ok(vec![]);
        }

        let mut entries = Vec::new();

        for ticker in &self.tickers {
            match self.fetch_one(ticker).await {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    eprintln!("Warning: Failed to fetch {ticker}: {e}");
                }
            }
        }

        Ok(entries)
    }
}

impl YahooFeed {
    async fn fetch_one(&self, ticker: &str) -> Result<IntelEntry, FeedError> {
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{ticker}?range=1d&interval=1d"
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| FeedError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(FeedError::Network(format!(
                "Yahoo API returned {} for {ticker}",
                resp.status()
            )));
        }

        let data: ChartResponse = resp
            .json()
            .await
            .map_err(|e| FeedError::Parse(e.to_string()))?;

        if let Some(err) = data.chart.error {
            return Err(FeedError::Parse(format!("Yahoo error: {err}")));
        }

        let results = data
            .chart
            .result
            .ok_or_else(|| FeedError::Parse("No chart results".into()))?;

        let meta = &results
            .first()
            .ok_or_else(|| FeedError::Parse("Empty chart results".into()))?
            .meta;

        let price = meta
            .regular_market_price
            .ok_or_else(|| FeedError::Parse(format!("No price for {ticker}")))?;

        let prev_close = meta.chart_previous_close.unwrap_or(price);
        let change = price - prev_close;
        let change_pct = if prev_close > 0.0 {
            (change / prev_close) * 100.0
        } else {
            0.0
        };

        let name = meta
            .short_name
            .as_deref()
            .or(meta.long_name.as_deref())
            .unwrap_or(&meta.symbol);

        let direction = if change_pct > 2.0 {
            "bullish"
        } else if change_pct < -2.0 {
            "bearish"
        } else {
            "neutral"
        };

        let title = format!("{} ${:.2} ({:+.2}%)", meta.symbol, price, change_pct);

        let mut body_parts = vec![format!(
            "{name} trading at ${price:.2} ({change:+.2}, {change_pct:+.2}%)"
        )];

        if let Some(vol) = meta.regular_market_volume {
            body_parts.push(format!("Volume: {vol}"));
        }
        if let (Some(high), Some(low)) = (meta.fifty_two_week_high, meta.fifty_two_week_low) {
            let range_pct = if high > low {
                (price - low) / (high - low) * 100.0
            } else {
                50.0
            };
            body_parts.push(format!(
                "52w range: ${low:.2}-${high:.2} ({range_pct:.0}% of range)"
            ));
        }
        if let (Some(dh), Some(dl)) = (meta.regular_market_day_high, meta.regular_market_day_low) {
            body_parts.push(format!("Day range: ${dl:.2}-${dh:.2}"));
        }

        let body = body_parts.join(". ");

        let mut tags = vec![
            meta.symbol.clone(),
            "yahoo-feed".to_string(),
            direction.to_string(),
        ];

        if change_pct.abs() > 5.0 {
            tags.push("big-mover".to_string());
        }
        if change_pct.abs() > 10.0 {
            tags.push("extreme-mover".to_string());
        }

        let conf_val = if meta.regular_market_volume.is_some()
            && meta.fifty_two_week_high.is_some()
        {
            0.85
        } else {
            0.7
        };

        Ok(IntelEntry::new(
            Category::Market,
            title,
            body,
            Some("yahoo_finance".to_string()),
            tags,
            Confidence::new(conf_val).map_err(|e| FeedError::Config(e))?,
            change_pct.abs() > 5.0,
            SourceType::External,
            Some(serde_json::json!({
                "price": price,
                "previous_close": prev_close,
                "change": change,
                "change_pct": change_pct,
                "volume": meta.regular_market_volume,
                "day_high": meta.regular_market_day_high,
                "day_low": meta.regular_market_day_low,
                "52w_high": meta.fifty_two_week_high,
                "52w_low": meta.fifty_two_week_low,
            })),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yahoo_feed_creation() {
        let feed = YahooFeed::new(vec!["AAPL".into(), "MSFT".into()]);
        assert_eq!(feed.name(), "yahoo_finance");
        assert_eq!(feed.tickers.len(), 2);
    }

    #[test]
    fn test_empty_tickers() {
        let feed = YahooFeed::new(vec![]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(feed.fetch());
        assert!(result.unwrap().is_empty());
    }
}
