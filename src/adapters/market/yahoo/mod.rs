mod response;

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::market_data_source::MarketDataSource;

const BASE_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const TIMEOUT_SECS: u64 = 10;

#[derive(Clone)]
pub struct YahooMarketSource {
    client: reqwest::Client,
}

impl YahooMarketSource {
    pub fn new() -> Result<Self, DomainError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .user_agent(concat!("openintel/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| DomainError::SourceFailure {
                name: "yahoo".into(),
                message: format!("client build failed: {e}"),
            })?;
        Ok(Self { client })
    }
}

#[async_trait]
impl MarketDataSource for YahooMarketSource {
    fn name(&self) -> &'static str {
        "yahoo"
    }

    async fn snapshot(&self, ticker: &Ticker) -> Result<MarketSnapshot, DomainError> {
        let url = format!("{BASE_URL}/{}?range=3mo&interval=1d", ticker.as_str());
        let fetched_at = Utc::now();

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| DomainError::SourceFailure {
                name: "yahoo".into(),
                message: format!("request failed: {e}"),
            })?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| DomainError::SourceFailure {
            name: "yahoo".into(),
            message: format!("reading body failed (HTTP {status}): {e}"),
        })?;

        response::parse_snapshot(&body, ticker, fetched_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_builds_and_names_yahoo() {
        let src = YahooMarketSource::new().unwrap();
        assert_eq!(src.name(), "yahoo");
    }

    #[tokio::test]
    #[ignore = "hits live Yahoo Finance; run with `cargo test -- --ignored`"]
    async fn live_snapshot_has_positive_prices() {
        let src = YahooMarketSource::new().unwrap();
        let snap = src.snapshot(&Ticker::parse("AAPL").unwrap()).await.unwrap();
        assert!(snap.last_price > 0.0, "last_price = {}", snap.last_price);
        assert!(snap.previous_close > 0.0);
    }
}
