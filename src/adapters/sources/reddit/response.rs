#![allow(dead_code)]

use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::domain::entities::social_post::{PostText, SocialPost};
use crate::domain::error::DomainError;
use crate::domain::values::source_kind::SourceKind;

const MAX_TEXT_CHARS: usize = 10_000;

#[derive(Debug, Deserialize)]
struct Listing {
    data: ListingData,
}

#[derive(Debug, Deserialize)]
struct ListingData {
    #[serde(default)]
    children: Vec<Child>,
}

#[derive(Debug, Deserialize)]
struct Child {
    data: ChildData,
}

#[derive(Debug, Deserialize)]
struct ChildData {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    selftext: Option<String>,
    #[serde(default)]
    score: Option<i64>,
    #[serde(default)]
    created_utc: Option<f64>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "reddit".into(),
        message: message.into(),
    }
}

pub(crate) fn parse_posts(
    body: &str,
    limit: usize,
    fetched_at: DateTime<Utc>,
) -> Result<Vec<SocialPost>, DomainError> {
    let listing: Listing =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed response: {e}")))?;

    let mut posts = Vec::new();
    for child in listing.data.children {
        let d = child.data;
        let id = match d.name.or(d.id) {
            Some(id) if !id.is_empty() => id,
            _ => continue,
        };
        let title = d.title.unwrap_or_default();
        let selftext = d.selftext.unwrap_or_default();
        let combined = if selftext.trim().is_empty() {
            title
        } else {
            format!("{title}\n{selftext}")
        };
        let truncated: String = combined.chars().take(MAX_TEXT_CHARS).collect();
        let text = match PostText::parse(&truncated) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let created_at = d
            .created_utc
            .and_then(|s| Utc.timestamp_opt(s as i64, 0).single())
            .unwrap_or(fetched_at);

        posts.push(SocialPost {
            id,
            source: SourceKind::Reddit,
            author: d.author.unwrap_or_else(|| "[unknown]".to_string()),
            text,
            created_at,
            engagement: d.score.unwrap_or(0).max(0) as u32,
        });
        if posts.len() >= limit {
            break;
        }
    }
    Ok(posts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::source_kind::SourceKind;
    use chrono::{DateTime, TimeZone, Utc};

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 2, 0, 0, 0).unwrap()
    }

    const HAPPY: &str = r#"{"kind":"Listing","data":{"children":[
        {"kind":"t3","data":{"name":"t3_aaa","author":"wsbtrader","title":"$AAPL calls printing","selftext":"loading more","score":420,"created_utc":1782504000.0}},
        {"kind":"t3","data":{"name":"t3_bbb","author":"[deleted]","title":"AAPL puts","selftext":"","score":-5,"created_utc":1782500000.0}}
    ]}}"#;

    const EMPTY: &str = r#"{"kind":"Listing","data":{"children":[]}}"#;

    #[test]
    fn happy_maps_posts() {
        let posts = parse_posts(HAPPY, 50, at()).unwrap();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].id, "t3_aaa");
        assert_eq!(posts[0].author, "wsbtrader");
        assert_eq!(posts[0].text.as_str(), "$AAPL calls printing\nloading more");
        assert_eq!(posts[0].engagement, 420);
        assert_eq!(posts[0].source, SourceKind::Reddit);
        assert_eq!(
            posts[0].created_at,
            Utc.timestamp_opt(1782504000, 0).single().unwrap()
        );
    }

    #[test]
    fn empty_selftext_is_title_only_and_deleted_author_kept() {
        let posts = parse_posts(HAPPY, 50, at()).unwrap();
        assert_eq!(posts[1].text.as_str(), "AAPL puts");
        assert_eq!(posts[1].author, "[deleted]");
    }

    #[test]
    fn negative_score_clamps_to_zero() {
        let posts = parse_posts(HAPPY, 50, at()).unwrap();
        assert_eq!(posts[1].engagement, 0);
    }

    #[test]
    fn limit_is_honored() {
        assert_eq!(parse_posts(HAPPY, 1, at()).unwrap().len(), 1);
    }

    #[test]
    fn empty_children_is_empty() {
        assert!(parse_posts(EMPTY, 50, at()).unwrap().is_empty());
    }

    #[test]
    fn missing_created_utc_falls_back_to_fetched_at() {
        let body = r#"{"kind":"Listing","data":{"children":[
            {"kind":"t3","data":{"name":"t3_c","author":"a","title":"AAPL","score":1}}
        ]}}"#;
        assert_eq!(parse_posts(body, 50, at()).unwrap()[0].created_at, at());
    }

    #[test]
    fn overlong_text_is_truncated() {
        let big = "A".repeat(20_000);
        let body = format!(
            r#"{{"kind":"Listing","data":{{"children":[{{"kind":"t3","data":{{"name":"t3_d","author":"a","title":"{big}","score":1,"created_utc":1.0}}}}]}}}}"#
        );
        let posts = parse_posts(&body, 50, at()).unwrap();
        assert_eq!(posts[0].text.as_str().chars().count(), 10_000);
    }

    #[test]
    fn post_with_no_id_is_skipped() {
        let body = r#"{"kind":"Listing","data":{"children":[
            {"kind":"t3","data":{"author":"a","title":"AAPL","score":1}}
        ]}}"#;
        assert!(parse_posts(body, 50, at()).unwrap().is_empty());
    }

    #[test]
    fn malformed_json_is_source_failure() {
        assert!(parse_posts("not json", 50, at()).is_err());
    }
}
