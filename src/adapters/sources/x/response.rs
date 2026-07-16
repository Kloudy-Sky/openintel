use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::domain::entities::pulse::PulsePost;
use crate::domain::entities::social_post::PostText;
use crate::domain::error::DomainError;

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    data: Vec<Tweet>,
    #[serde(default)]
    includes: Includes,
}

#[derive(Debug, Deserialize)]
struct Tweet {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    author_id: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    public_metrics: Option<Metrics>,
}

#[derive(Debug, Deserialize, Default)]
struct Metrics {
    #[serde(default)]
    like_count: Option<i64>,
    #[serde(default)]
    retweet_count: Option<i64>,
    #[serde(default)]
    reply_count: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct Includes {
    #[serde(default)]
    users: Vec<User>,
}

#[derive(Debug, Deserialize)]
struct User {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    username: Option<String>,
}

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "x".into(),
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
) -> Result<Vec<PulsePost>, DomainError> {
    let resp: SearchResponse =
        serde_json::from_str(body).map_err(|e| fail(format!("malformed response: {e}")))?;

    if limit == 0 {
        return Ok(Vec::new());
    }

    // author_id -> username join table from `includes.users`.
    let users: std::collections::HashMap<String, String> = resp
        .includes
        .users
        .into_iter()
        .filter_map(|u| Some((u.id?, u.username?)))
        .collect();

    let mut posts = Vec::new();
    for tweet in resp.data {
        let id = match tweet.id {
            Some(i) if !i.is_empty() => i,
            _ => continue,
        };
        let text = match PostText::parse(&tweet.text.unwrap_or_default()) {
            Ok(t) => t,
            Err(_) => continue, // empty text -> skip, not fatal
        };
        let author = tweet
            .author_id
            .and_then(|aid| users.get(&aid).cloned())
            .unwrap_or_else(|| "[unknown]".to_string());
        let created_at = tweet
            .created_at
            .as_deref()
            .and_then(parse_rfc3339)
            .unwrap_or(fetched_at);
        let m = tweet.public_metrics.unwrap_or_default();
        let engagement = [m.like_count, m.retweet_count, m.reply_count]
            .iter()
            .map(|c| c.unwrap_or(0).max(0) as u64)
            .sum::<u64>()
            .min(u32::MAX as u64) as u32;

        posts.push(PulsePost {
            id,
            author,
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
        Utc.with_ymd_and_hms(2026, 7, 16, 12, 0, 0).unwrap()
    }

    const HAPPY: &str = r#"{
        "data":[
            {"id":"1","text":"Chips made in America will be TAXED at ZERO","author_id":"u1",
             "created_at":"2026-07-16T09:00:00.000Z",
             "public_metrics":{"like_count":100,"retweet_count":40,"reply_count":10}},
            {"id":"2","text":"Blackwell Ultra shipping at scale","author_id":"u2"}
        ],
        "includes":{"users":[
            {"id":"u1","username":"realDonaldTrump"},
            {"id":"u2","username":"jensenhuang"}
        ]}
    }"#;

    #[test]
    fn happy_joins_authors_and_sums_engagement() {
        let posts = parse_posts(HAPPY, 50, at()).unwrap();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].author, "realDonaldTrump");
        assert_eq!(posts[0].engagement, 150);
        assert_eq!(
            posts[0].created_at,
            Utc.with_ymd_and_hms(2026, 7, 16, 9, 0, 0).unwrap()
        );
        assert_eq!(posts[1].author, "jensenhuang");
        assert_eq!(posts[1].engagement, 0); // no public_metrics
        assert_eq!(posts[1].created_at, at()); // no created_at -> fetched_at
    }

    #[test]
    fn unknown_author_and_skips() {
        let body = r#"{"data":[
            {"id":"1","text":"no author id"},
            {"id":"2","text":"   "},
            {"text":"no id"}
        ]}"#;
        let posts = parse_posts(body, 50, at()).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].author, "[unknown]");
    }

    #[test]
    fn limit_truncates_and_zero_is_empty() {
        assert_eq!(parse_posts(HAPPY, 1, at()).unwrap().len(), 1);
        assert!(parse_posts(HAPPY, 0, at()).unwrap().is_empty());
    }

    #[test]
    fn empty_data_and_malformed() {
        assert!(parse_posts(r#"{"data":[]}"#, 50, at()).unwrap().is_empty());
        assert!(parse_posts(r#"{}"#, 50, at()).unwrap().is_empty()); // data defaults
        assert!(parse_posts("nope", 50, at()).is_err());
    }
}
