use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::domain::entities::social_post::{PostText, SocialPost};
use crate::domain::error::DomainError;
use crate::domain::values::source_kind::SourceKind;

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    posts: Vec<PostView>,
}

#[derive(Debug, Deserialize)]
struct PostView {
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    author: Option<Author>,
    #[serde(default)]
    record: Option<Record>,
    #[serde(default, rename = "indexedAt")]
    indexed_at: Option<String>,
    #[serde(default, rename = "likeCount")]
    like_count: Option<i64>,
    #[serde(default, rename = "repostCount")]
    repost_count: Option<i64>,
    #[serde(default, rename = "replyCount")]
    reply_count: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct Author {
    #[serde(default)]
    handle: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Record {
    #[serde(default)]
    text: Option<String>,
    #[serde(default, rename = "createdAt")]
    created_at: Option<String>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "bluesky".into(),
        message: message.into(),
    }
}

fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub(crate) fn parse_posts(
    body: &str,
    limit: usize,
    fetched_at: DateTime<Utc>,
) -> Result<Vec<SocialPost>, DomainError> {
    let resp: SearchResponse =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed response: {e}")))?;

    if limit == 0 {
        return Ok(Vec::new());
    }

    let mut posts = Vec::new();
    for view in resp.posts {
        let id = match view.uri {
            Some(u) if !u.is_empty() => u,
            _ => continue,
        };
        let record = view.record.unwrap_or_default();
        let text = match PostText::parse(&record.text.unwrap_or_default()) {
            Ok(t) => t,
            Err(_) => continue, // empty/whitespace text -> skip, not fatal
        };
        let created_at = record
            .created_at
            .as_deref()
            .and_then(parse_rfc3339)
            .or_else(|| view.indexed_at.as_deref().and_then(parse_rfc3339))
            .unwrap_or(fetched_at);
        let engagement = [view.like_count, view.repost_count, view.reply_count]
            .iter()
            .map(|c| c.unwrap_or(0).max(0) as u64)
            .sum::<u64>()
            .min(u32::MAX as u64) as u32;

        posts.push(SocialPost {
            id,
            source: SourceKind::Bluesky,
            author: view
                .author
                .unwrap_or_default()
                .handle
                .unwrap_or_else(|| "[unknown]".to_string()),
            text,
            created_at,
            engagement,
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
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 10, 0, 0, 0).unwrap()
    }

    const HAPPY: &str = r#"{"posts":[
        {"uri":"at://did:plc:abc/app.bsky.feed.post/1","author":{"handle":"indexfan.bsky.social"},
         "record":{"text":"$AAPL calls printing","createdAt":"2026-07-09T15:30:00Z"},
         "indexedAt":"2026-07-09T15:31:00Z","likeCount":10,"repostCount":3,"replyCount":2},
        {"uri":"at://did:plc:def/app.bsky.feed.post/2","author":{"handle":"skeptic.bsky.social"},
         "record":{"text":"AAPL looks toppy, selling"},"likeCount":1}
    ]}"#;

    #[test]
    fn happy_maps_posts() {
        let posts = parse_posts(HAPPY, 50, at()).unwrap();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].id, "at://did:plc:abc/app.bsky.feed.post/1");
        assert_eq!(posts[0].author, "indexfan.bsky.social");
        assert_eq!(posts[0].text.as_str(), "$AAPL calls printing");
        assert_eq!(posts[0].engagement, 15); // 10 likes + 3 reposts + 2 replies
        assert_eq!(posts[0].source, SourceKind::Bluesky);
        assert_eq!(
            posts[0].created_at,
            Utc.with_ymd_and_hms(2026, 7, 9, 15, 30, 0).unwrap()
        );
    }

    #[test]
    fn missing_created_at_falls_back_to_fetched_at_and_missing_counts_are_zero() {
        let posts = parse_posts(HAPPY, 50, at()).unwrap();
        assert_eq!(posts[1].created_at, at()); // no createdAt, no indexedAt
        assert_eq!(posts[1].engagement, 1); // only likeCount present
    }

    #[test]
    fn indexed_at_is_fallback_when_created_at_missing() {
        let body =
            r#"{"posts":[{"uri":"u1","record":{"text":"hi"},"indexedAt":"2026-07-09T12:00:00Z"}]}"#;
        let posts = parse_posts(body, 50, at()).unwrap();
        assert_eq!(
            posts[0].created_at,
            Utc.with_ymd_and_hms(2026, 7, 9, 12, 0, 0).unwrap()
        );
        assert_eq!(posts[0].author, "[unknown]");
    }

    #[test]
    fn empty_text_and_missing_uri_are_skipped() {
        let body = r#"{"posts":[
            {"uri":"u1","record":{"text":"   "}},
            {"record":{"text":"no uri"}},
            {"uri":"u2","record":{"text":"kept"}}
        ]}"#;
        let posts = parse_posts(body, 50, at()).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].text.as_str(), "kept");
    }

    #[test]
    fn limit_truncates_and_zero_is_empty() {
        assert_eq!(parse_posts(HAPPY, 1, at()).unwrap().len(), 1);
        assert!(parse_posts(HAPPY, 0, at()).unwrap().is_empty());
    }

    #[test]
    fn engagement_saturates_at_u32_max() {
        let body = r#"{"posts":[{"uri":"u1","record":{"text":"big"},
            "likeCount":4294967295,"repostCount":4294967295,"replyCount":10}]}"#;
        let posts = parse_posts(body, 50, at()).unwrap();
        assert_eq!(posts[0].engagement, u32::MAX);
    }

    #[test]
    fn malformed_json_is_failure_and_empty_posts_ok() {
        assert!(parse_posts("nope", 50, at()).is_err());
        assert!(parse_posts(r#"{"posts":[]}"#, 50, at()).unwrap().is_empty());
    }
}
