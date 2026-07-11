//! Deterministic in-memory social sources for tests (replaces the deleted
//! mock adapters — production wiring has no fakes; see the 2026-07-10 spec).

use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use crate::domain::entities::social_post::{PostText, SocialPost};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::domain::values::source_kind::SourceKind;

type Row = (&'static str, &'static str, &'static str, u32);

pub(crate) struct FixtureSource {
    pub kind: SourceKind,
    pub rows: &'static [Row],
}

#[async_trait]
impl SocialDataSource for FixtureSource {
    fn kind(&self) -> SourceKind {
        self.kind
    }

    async fn fetch(&self, ticker: &Ticker, limit: usize) -> Result<Vec<SocialPost>, DomainError> {
        let sym = ticker.as_str();
        self.rows
            .iter()
            .take(limit)
            .map(|(id, author, template, engagement)| {
                Ok(SocialPost {
                    id: (*id).to_string(),
                    source: self.kind,
                    author: (*author).to_string(),
                    text: PostText::parse(&template.replace("{sym}", sym))?,
                    created_at: Utc.with_ymd_and_hms(2026, 6, 24, 15, 0, 0).unwrap(),
                    engagement: *engagement,
                })
            })
            .collect()
    }
}

/// The old MockRedditSource rows, verbatim (4 posts).
pub(crate) fn reddit_fixture() -> FixtureSource {
    FixtureSource {
        kind: SourceKind::Reddit,
        rows: &[
            (
                "reddit-1",
                "dudebro",
                "{sym} to the moon, loading calls all day",
                420,
            ),
            (
                "reddit-2",
                "valuepicker",
                "{sym} earnings look strong, going long here",
                88,
            ),
            (
                "reddit-3",
                "chartwatcher",
                "{sym} breakout confirmed, rocket time",
                51,
            ),
            (
                "reddit-4",
                "shortking",
                "{sym} is going to dump, buying puts",
                31,
            ),
        ],
    }
}

/// The old MockBlueskySource rows plus the old MockXSource rows re-homed to
/// Bluesky (6 posts) — keeps the all-fixtures total at 10 (= min_sample) with
/// identical analyzer inputs, so fusion assertions are unchanged.
pub(crate) fn bluesky_fixture() -> FixtureSource {
    FixtureSource {
        kind: SourceKind::Bluesky,
        rows: &[
            (
                "bsky-1",
                "indexfan",
                "{sym} looking bullish into the print",
                22,
            ),
            (
                "bsky-2",
                "skeptic",
                "not sold on {sym}, might sell my shares",
                9,
            ),
            ("bsky-3", "daytripper", "{sym} green day, up big", 14),
            (
                "bsky-4",
                "quanttrader",
                "${sym} squeeze incoming, buying calls",
                1200,
            ),
            (
                "bsky-5",
                "macroowl",
                "watching ${sym} but staying cautious",
                64,
            ),
            ("bsky-6", "trendrider", "${sym} rally looks strong", 240),
        ],
    }
}

pub(crate) fn fixture_social() -> Vec<Box<dyn SocialDataSource>> {
    vec![Box::new(reddit_fixture()), Box::new(bluesky_fixture())]
}
