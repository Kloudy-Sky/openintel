//! Parser for Yahoo's search endpoint news items (keyless). Reads the
//! headline fields the catalyst heuristic needs plus the matching quote's
//! company names (so the gate can tell company news from market roundups).

use chrono::{TimeZone, Utc};
use serde::Deserialize;

use crate::domain::error::DomainError;
use crate::domain::ports::news_source::NewsFetch;
use crate::domain::values::headline::Headline;

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    news: Vec<NewsItem>,
    #[serde(default)]
    quotes: Vec<QuoteItem>,
}

#[derive(Debug, Deserialize)]
struct QuoteItem {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    shortname: Option<String>,
    #[serde(default)]
    longname: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewsItem {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    publisher: Option<String>,
    #[serde(default)]
    provider_publish_time: Option<i64>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "yahoo-news".into(),
        message: message.into(),
    }
}

/// Headlines + company names from the search body. Only title-less items are
/// skipped (they carry no scannable evidence); a missing/invalid timestamp
/// yields `published_at: None` so the catalyst gate can treat the item
/// conservatively instead of losing it. An empty list is a valid result —
/// quiet tickers have no news. Company names come from the quote whose
/// symbol matches the queried ticker.
pub(crate) fn parse_news(body: &str, ticker: &str) -> Result<NewsFetch, DomainError> {
    let resp: SearchResponse =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed response: {e}")))?;
    let headlines = resp
        .news
        .into_iter()
        .filter_map(|n| {
            Some(Headline {
                title: n.title?,
                publisher: n.publisher.unwrap_or_else(|| "unknown".into()),
                published_at: n
                    .provider_publish_time
                    .and_then(|t| Utc.timestamp_opt(t, 0).single()),
            })
        })
        .collect();
    let mut company_names: Vec<String> = Vec::new();
    for q in resp.quotes {
        if q.symbol.as_deref() == Some(ticker) {
            for name in [q.shortname, q.longname].into_iter().flatten() {
                if !company_names.contains(&name) {
                    company_names.push(name);
                }
            }
        }
    }
    Ok(NewsFetch {
        headlines,
        company_names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headlines_keeps_undated_skips_titleless() {
        let body = r#"{"news":[
            {"title":"Company cuts guidance","publisher":"Wire","providerPublishTime":1786818192},
            {"title":"No timestamp","publisher":"Wire"},
            {"publisher":"Wire","providerPublishTime":1786818192}
        ],"quotes":[
            {"symbol":"UCTT","shortname":"Ultra Clean Holdings, Inc.","longname":"Ultra Clean Holdings, Inc."},
            {"symbol":"UCTX","shortname":"Corgi UCTT 2x Daily ETF"}
        ]}"#;
        let fetch = parse_news(body, "UCTT").unwrap();
        let h = &fetch.headlines;
        assert_eq!(h.len(), 2); // only the title-less item is dropped
        assert_eq!(h[0].title, "Company cuts guidance");
        assert_eq!(h[0].published_at.unwrap().timestamp(), 1786818192);
        assert_eq!(h[1].published_at, None); // undated survives for the gate
                                             // names deduped and only from the MATCHING symbol (not the 2x ETF)
        assert_eq!(fetch.company_names, vec!["Ultra Clean Holdings, Inc."]);
    }

    #[test]
    fn empty_news_is_ok_malformed_is_err() {
        assert!(parse_news(r#"{"news":[]}"#, "AAPL")
            .unwrap()
            .headlines
            .is_empty());
        let fetch = parse_news(r#"{}"#, "AAPL").unwrap();
        assert!(fetch.headlines.is_empty() && fetch.company_names.is_empty());
        assert!(parse_news("nope", "AAPL").is_err());
    }
}
