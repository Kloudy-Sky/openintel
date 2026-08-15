//! Parser for Yahoo's search endpoint news items (keyless). Only the
//! headline fields the catalyst heuristic needs are read.

use chrono::{TimeZone, Utc};
use serde::Deserialize;

use crate::domain::error::DomainError;
use crate::domain::values::headline::Headline;

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    news: Vec<NewsItem>,
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

/// Headlines with all required fields; incomplete items are skipped (a
/// missing timestamp would defeat the same-day filter). An empty list is a
/// valid result — quiet tickers have no news.
pub(crate) fn parse_headlines(body: &str) -> Result<Vec<Headline>, DomainError> {
    let resp: SearchResponse =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed response: {e}")))?;
    Ok(resp
        .news
        .into_iter()
        .filter_map(|n| {
            Some(Headline {
                title: n.title?,
                publisher: n.publisher.unwrap_or_else(|| "unknown".into()),
                published_at: Utc.timestamp_opt(n.provider_publish_time?, 0).single()?,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headlines_and_skips_incomplete() {
        let body = r#"{"news":[
            {"title":"Company cuts guidance","publisher":"Wire","providerPublishTime":1786818192},
            {"title":"No timestamp","publisher":"Wire"},
            {"publisher":"Wire","providerPublishTime":1786818192}
        ]}"#;
        let h = parse_headlines(body).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].title, "Company cuts guidance");
        assert_eq!(h[0].published_at.timestamp(), 1786818192);
    }

    #[test]
    fn empty_news_is_ok_malformed_is_err() {
        assert!(parse_headlines(r#"{"news":[]}"#).unwrap().is_empty());
        assert!(parse_headlines(r#"{}"#).unwrap().is_empty());
        assert!(parse_headlines("nope").is_err());
    }
}
