//! SEC EDGAR filings source (keyless, official). Two feeds: the ticker→CIK
//! map (fetched once per process, cached) and per-company submissions.
//! SEC's fair-access policy requires a descriptive User-Agent with contact
//! info and ≤10 requests/second — callers must bound their concurrency.

mod response;

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::NaiveDate;
use tokio::sync::OnceCell;

use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::filings_source::FilingsSource;
use crate::domain::values::filing::Filing;

const TICKER_MAP_URL: &str = "https://www.sec.gov/files/company_tickers.json";
const SUBMISSIONS_BASE: &str = "https://data.sec.gov/submissions";
const TIMEOUT_SECS: u64 = 10;
/// Overridable contact for SEC's fair-access UA policy.
const CONTACT_ENV: &str = "OPENINTEL_SEC_CONTACT";
const DEFAULT_CONTACT: &str = "+https://github.com/kloudysky/openintel";

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "edgar".into(),
        message: message.into(),
    }
}

pub struct EdgarSource {
    client: reqwest::Client,
    ticker_map: OnceCell<HashMap<String, u64>>,
}

impl EdgarSource {
    pub fn new() -> Result<Self, DomainError> {
        let contact = std::env::var(CONTACT_ENV).unwrap_or_else(|_| DEFAULT_CONTACT.to_string());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .user_agent(format!(
                "openintel/{} ({contact})",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|e| fail(format!("client build failed: {e}")))?;
        Ok(Self {
            client,
            ticker_map: OnceCell::new(),
        })
    }

    async fn fetch(&self, url: &str) -> Result<String, DomainError> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| fail(format!("request failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(fail(format!("HTTP {status} for {url}")));
        }
        resp.text()
            .await
            .map_err(|e| fail(format!("reading body failed: {e}")))
    }

    async fn cik_for(&self, ticker: &Ticker) -> Result<u64, DomainError> {
        let map = self
            .ticker_map
            .get_or_try_init(|| async {
                let body = self.fetch(TICKER_MAP_URL).await?;
                response::parse_ticker_map(&body)
            })
            .await?;
        map.get(ticker.as_str()).copied().ok_or_else(|| {
            fail(format!(
                "no SEC filer mapping for {} — cannot verify filings",
                ticker.as_str()
            ))
        })
    }
}

#[async_trait]
impl FilingsSource for EdgarSource {
    async fn recent_filings(
        &self,
        ticker: &Ticker,
        since: NaiveDate,
    ) -> Result<Vec<Filing>, DomainError> {
        let cik = self.cik_for(ticker).await?;
        let body = self
            .fetch(&format!("{SUBMISSIONS_BASE}/CIK{cik:010}.json"))
            .await?;
        response::parse_recent_filings(&body, since)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_with_default_contact_ua() {
        assert!(EdgarSource::new().is_ok());
    }

    #[tokio::test]
    #[ignore = "hits live SEC EDGAR (keyless, free); run with --ignored"]
    async fn live_apple_has_recent_filings() {
        let src = EdgarSource::new().unwrap();
        let since = chrono::Utc::now().date_naive() - chrono::Days::new(90);
        let filings = src
            .recent_filings(&Ticker::parse("AAPL").unwrap(), since)
            .await
            .unwrap();
        assert!(!filings.is_empty());
        assert!(filings.iter().all(|f| f.filed_on >= since));
    }

    #[tokio::test]
    #[ignore = "hits live SEC EDGAR (keyless, free); run with --ignored"]
    async fn live_unmapped_ticker_errors() {
        let src = EdgarSource::new().unwrap();
        let since = chrono::Utc::now().date_naive();
        // ZZZZZ is a valid Ticker shape but not an SEC filer
        assert!(src
            .recent_filings(&Ticker::parse("ZZZZZ").unwrap(), since)
            .await
            .is_err());
    }
}
