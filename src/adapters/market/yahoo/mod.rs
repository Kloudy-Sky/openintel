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
            .get(url)
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

        to_snapshot(status, &body, ticker, fetched_at)
    }
}

/// Map an HTTP status + body to a snapshot. On a failed parse, prefix the HTTP
/// status when the response was not 2xx, so transient failures (e.g. 429) are
/// self-describing without discarding Yahoo's own JSON error message.
fn to_snapshot(
    status: reqwest::StatusCode,
    body: &str,
    ticker: &Ticker,
    fetched_at: chrono::DateTime<chrono::Utc>,
) -> Result<MarketSnapshot, DomainError> {
    match response::parse_snapshot(body, ticker, fetched_at) {
        Ok(snapshot) => Ok(snapshot),
        Err(DomainError::SourceFailure { message, .. }) if !status.is_success() => {
            Err(DomainError::SourceFailure {
                name: "yahoo".into(),
                message: format!("HTTP {status}: {message}"),
            })
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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

    #[test]
    fn to_snapshot_prefixes_http_status_on_failed_non_2xx() {
        let t = Ticker::parse("AAPL").unwrap();
        let at = chrono::Utc.timestamp_opt(0, 0).single().unwrap();
        let err =
            to_snapshot(reqwest::StatusCode::TOO_MANY_REQUESTS, "garbage", &t, at).unwrap_err();
        assert!(err.to_string().contains("429"), "got {err}");
    }

    #[test]
    fn to_snapshot_passes_parser_error_through_on_2xx() {
        let t = Ticker::parse("AAPL").unwrap();
        let at = chrono::Utc.timestamp_opt(0, 0).single().unwrap();
        let err = to_snapshot(reqwest::StatusCode::OK, "garbage", &t, at).unwrap_err();
        // On a 2xx, no HTTP prefix is added — the parser's message stands.
        assert!(
            !err.to_string().contains("HTTP "),
            "unexpected HTTP prefix: {err}"
        );
    }
}
