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

/// Filings on/after `since` from a submissions body. Form/date arrays are
/// zipped positionally per EDGAR's column-oriented format. Malformed rows are
/// an ERROR, not a skip — a silently dropped filing could hide a catalyst,
/// and callers treat an error as "could not verify" (fail closed).
pub(crate) fn parse_recent_filings(
    body: &str,
    since: NaiveDate,
) -> Result<Vec<Filing>, DomainError> {
    let subs: Submissions =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed submissions: {e}")))?;
    let recent = subs.filings.recent;
    if recent.form.len() != recent.filing_date.len() {
        return Err(fail(format!(
            "submissions column mismatch: {} forms vs {} dates",
            recent.form.len(),
            recent.filing_date.len()
        )));
    }
    let mut filings = Vec::new();
    for (form, date) in recent.form.into_iter().zip(recent.filing_date) {
        let filed_on = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map_err(|_| fail(format!("unparseable filing date '{date}' for form {form}")))?;
        if filed_on >= since {
            filings.push(Filing { form, filed_on });
        }
    }
    Ok(filings)
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
            "form":["8-K","4","10-Q"],
            "filingDate":["2026-08-14","2026-08-13","2026-07-31"]
        }}}"#;
        let since = NaiveDate::from_ymd_opt(2026, 8, 13).unwrap();
        let filings = parse_recent_filings(body, since).unwrap();
        assert_eq!(filings.len(), 2); // 10-Q too old
        assert_eq!(filings[0].form, "8-K");
        assert_eq!(filings[1].form, "4");
        assert!(parse_recent_filings("nope", since).is_err());
    }

    #[test]
    fn malformed_rows_fail_closed_instead_of_skipping() {
        let since = NaiveDate::from_ymd_opt(2026, 8, 13).unwrap();
        // bogus date: an error, never a silent skip (could hide a catalyst)
        let bogus_date = r#"{"filings":{"recent":{
            "form":["424B5"],"filingDate":["bogus"]
        }}}"#;
        assert!(parse_recent_filings(bogus_date, since).is_err());
        // column length mismatch: same
        let mismatch = r#"{"filings":{"recent":{
            "form":["8-K","4"],"filingDate":["2026-08-14"]
        }}}"#;
        assert!(parse_recent_filings(mismatch, since).is_err());
    }
}
