//! Parsers for SEC EDGAR's two keyless JSON feeds: the ticker→CIK map and
//! the per-company submissions index.

use std::collections::HashMap;

use chrono::NaiveDate;
use serde::Deserialize;

use crate::domain::error::DomainError;
use crate::domain::values::filing::Filing;

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "edgar".into(),
        message: message.into(),
    }
}

#[derive(Debug, Deserialize)]
struct TickerEntry {
    cik_str: u64,
    ticker: String,
}

/// `company_tickers.json` is a map of arbitrary numeric keys to entries.
pub(crate) fn parse_ticker_map(body: &str) -> Result<HashMap<String, u64>, DomainError> {
    let entries: HashMap<String, TickerEntry> =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed ticker map: {e}")))?;
    if entries.is_empty() {
        return Err(fail("empty ticker map"));
    }
    Ok(entries
        .into_values()
        .map(|e| (e.ticker.to_ascii_uppercase(), e.cik_str))
        .collect())
}

#[derive(Debug, Deserialize)]
struct Submissions {
    filings: Filings,
}

#[derive(Debug, Deserialize)]
struct Filings {
    recent: Recent,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Recent {
    #[serde(default)]
    form: Vec<String>,
    #[serde(default)]
    filing_date: Vec<String>,
}

/// Filings on/after `since` from a submissions body. Rows with unparseable
/// dates are skipped; form/date arrays are zipped positionally per EDGAR's
/// column-oriented format.
pub(crate) fn parse_recent_filings(
    body: &str,
    since: NaiveDate,
) -> Result<Vec<Filing>, DomainError> {
    let subs: Submissions =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed submissions: {e}")))?;
    let recent = subs.filings.recent;
    Ok(recent
        .form
        .into_iter()
        .zip(recent.filing_date)
        .filter_map(|(form, date)| {
            let filed_on = NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok()?;
            (filed_on >= since).then_some(Filing { form, filed_on })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticker_map_uppercases_symbols() {
        let body = r#"{"0":{"cik_str":320193,"ticker":"aapl","title":"Apple Inc."},
                       "1":{"cik_str":1045810,"ticker":"NVDA","title":"NVIDIA CORP"}}"#;
        let map = parse_ticker_map(body).unwrap();
        assert_eq!(map.get("AAPL"), Some(&320193));
        assert_eq!(map.get("NVDA"), Some(&1045810));
        assert!(parse_ticker_map("{}").is_err());
        assert!(parse_ticker_map("nope").is_err());
    }

    #[test]
    fn filings_filter_by_date_and_zip_positionally() {
        let body = r#"{"filings":{"recent":{
            "form":["8-K","4","10-Q","424B5"],
            "filingDate":["2026-08-14","2026-08-13","2026-07-31","bogus"]
        }}}"#;
        let since = NaiveDate::from_ymd_opt(2026, 8, 13).unwrap();
        let filings = parse_recent_filings(body, since).unwrap();
        assert_eq!(filings.len(), 2); // 10-Q too old, bogus date skipped
        assert_eq!(filings[0].form, "8-K");
        assert_eq!(filings[1].form, "4");
        assert!(parse_recent_filings("nope", since).is_err());
    }
}
