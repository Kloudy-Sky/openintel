mod news;
mod response;
mod screener;

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::bar_source::BarSource;
use crate::domain::ports::market_data_source::MarketDataSource;
use crate::domain::ports::movers_source::MoversSource;
use crate::domain::ports::news_source::{NewsFetch, NewsSource};
use crate::domain::values::bar::Bar;
use crate::domain::values::mover::MoverRow;

const BASE_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const SCREENER_URL: &str = "https://query1.finance.yahoo.com/v1/finance/screener/predefined/saved";
const SEARCH_URL: &str = "https://query1.finance.yahoo.com/v1/finance/search";
const MAX_SCREENER_ROWS: usize = 100;
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

    /// Issue the chart request and return the HTTP status alongside the raw
    /// body. Shared by `snapshot` (which needs the status to enrich parse
    /// failures) and `fetch_chart_body` (which does not).
    async fn fetch_chart(
        &self,
        ticker: &Ticker,
    ) -> Result<(reqwest::StatusCode, String), DomainError> {
        let url = format!("{BASE_URL}/{}?range=3mo&interval=1d", ticker.as_str());

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

        Ok((status, body))
    }

    /// The chart body alone, for consumers (like `bars`) that don't need
    /// HTTP-status-aware error enrichment.
    async fn fetch_chart_body(&self, ticker: &Ticker) -> Result<String, DomainError> {
        self.fetch_chart(ticker).await.map(|(_, body)| body)
    }

    async fn fetch_body(&self, name: &str, url: String) -> Result<String, DomainError> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| DomainError::SourceFailure {
                name: name.into(),
                message: format!("request failed: {e}"),
            })?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| DomainError::SourceFailure {
            name: name.into(),
            message: format!("reading body failed (HTTP {status}): {e}"),
        })?;
        if !status.is_success() {
            return Err(DomainError::SourceFailure {
                name: name.into(),
                message: format!("HTTP {status}"),
            });
        }
        Ok(body)
    }
}

#[async_trait]
impl MarketDataSource for YahooMarketSource {
    fn name(&self) -> &'static str {
        "yahoo"
    }

    async fn snapshot(&self, ticker: &Ticker) -> Result<MarketSnapshot, DomainError> {
        let fetched_at = Utc::now();
        let (status, body) = self.fetch_chart(ticker).await?;
        to_snapshot(status, &body, ticker, fetched_at)
    }
}

#[async_trait]
impl BarSource for YahooMarketSource {
    async fn bars(&self, ticker: &Ticker) -> Result<Vec<Bar>, DomainError> {
        let body = self.fetch_chart_body(ticker).await?;
        response::parse_bars(&body)
    }
}

#[async_trait]
impl MoversSource for YahooMarketSource {
    fn name(&self) -> &str {
        "yahoo-screener"
    }

    async fn day_losers(&self, count: usize) -> Result<Vec<MoverRow>, DomainError> {
        let count = count.clamp(1, MAX_SCREENER_ROWS);
        let url = format!("{SCREENER_URL}?scrIds=day_losers&count={count}");
        let body = self.fetch_body("yahoo-screener", url).await?;
        screener::parse_movers(&body).map(|(rows, _skipped)| rows)
    }
}

#[async_trait]
impl NewsSource for YahooMarketSource {
    async fn headlines(&self, ticker: &Ticker, count: usize) -> Result<NewsFetch, DomainError> {
        // quotesCount=2 so the matching quote's company names ride along
        // (the catalyst gate uses them to tell company news from roundups).
        let url = format!(
            "{SEARCH_URL}?q={}&newsCount={count}&quotesCount=2",
            ticker.as_str()
        );
        let body = self.fetch_body("yahoo-news", url).await?;
        news::parse_news(&body, ticker.as_str())
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
        assert_eq!(MarketDataSource::name(&src), "yahoo");
        assert_eq!(MoversSource::name(&src), "yahoo-screener");
    }

    #[tokio::test]
    #[ignore = "hits live Yahoo Finance; run with `cargo test -- --ignored`"]
    async fn live_snapshot_has_positive_prices() {
        let src = YahooMarketSource::new().unwrap();
        let snap = src.snapshot(&Ticker::parse("AAPL").unwrap()).await.unwrap();
        assert!(snap.last_price > 0.0, "last_price = {}", snap.last_price);
        assert!(snap.previous_close > 0.0);
    }

    #[tokio::test]
    #[ignore = "hits live Yahoo (keyless, free); run with --ignored"]
    async fn live_bars_have_sane_ohlc() {
        let src = YahooMarketSource::new().unwrap();
        let bars = src.bars(&Ticker::parse("AAPL").unwrap()).await.unwrap();
        assert!(bars.len() >= 15);
        for b in &bars {
            assert!(b.high >= b.low);
        }
        // dates strictly increase (session dates, deduped by the venue)
        for w in bars.windows(2) {
            assert!(w[1].date > w[0].date);
        }
    }

    #[tokio::test]
    #[ignore = "hits live Yahoo (keyless, free); run with --ignored"]
    async fn live_day_losers_returns_rows() {
        let src = YahooMarketSource::new().unwrap();
        let rows = src.day_losers(100).await.unwrap();
        assert!(rows.len() >= 50, "got {}", rows.len());
        assert!(rows.iter().all(|r| r.change_pct < 0.0));
    }

    #[tokio::test]
    #[ignore = "hits live Yahoo (keyless, free); run with --ignored"]
    async fn live_headlines_parse() {
        let src = YahooMarketSource::new().unwrap();
        // AAPL always has news; content varies — assert shape only.
        let fetch = NewsSource::headlines(&src, &Ticker::parse("AAPL").unwrap(), 5)
            .await
            .unwrap();
        assert!(!fetch.headlines.is_empty());
        assert!(fetch.headlines.iter().all(|h| !h.title.is_empty()));
        assert!(fetch
            .company_names
            .iter()
            .any(|n| n.to_lowercase().contains("apple")));
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
