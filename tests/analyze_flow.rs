use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use openintel::adapters::market::mock_market::MockMarketSource;
use openintel::cli::run::analyze;
use openintel::config::settings::{AppConfig, OutputFormat};
use openintel::domain::entities::social_post::{PostText, SocialPost};
use openintel::domain::entities::ticker::Ticker;
use openintel::domain::error::DomainError;
use openintel::domain::ports::social_data_source::SocialDataSource;
use openintel::domain::values::source_kind::SourceKind;
use openintel::domain::values::speculation::Alignment;

struct FixtureSource {
    kind: SourceKind,
    rows: &'static [(&'static str, &'static str, &'static str, u32)],
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

fn cfg(reddit: bool, bluesky: bool, no_market: bool) -> AppConfig {
    AppConfig::new(
        "AAPL".into(),
        reddit,
        bluesky,
        no_market,
        50,
        OutputFormat::Json,
    )
}

fn fixture_social() -> Vec<Box<dyn SocialDataSource>> {
    vec![
        Box::new(FixtureSource {
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
        }),
        Box::new(FixtureSource {
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
        }),
    ]
}

#[tokio::test]
async fn end_to_end_all_sources_with_market() {
    let (report, json) = analyze(
        &cfg(false, false, false),
        &fixture_social(),
        Some(&MockMarketSource),
    )
    .await
    .unwrap();
    // 4 reddit + 6 bluesky fixture posts (>= min_sample of 10)
    assert_eq!(report.social.total_mentions, 10);
    assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
    assert!(report.market.is_some());
    assert!(json.contains("\"alignment\": \"confirming_bullish\""));
    assert!(json.contains("Not financial advice"));
}

#[tokio::test]
async fn single_source_only() {
    let (report, _) = analyze(
        &cfg(true, false, false),
        &fixture_social(),
        Some(&MockMarketSource),
    )
    .await
    .unwrap();
    assert_eq!(report.social.total_mentions, 4); // reddit fixtures only
}

#[tokio::test]
async fn social_only_when_market_disabled() {
    let (report, _) = analyze(&cfg(false, false, true), &fixture_social(), None)
        .await
        .unwrap();
    assert!(report.market.is_none());
    assert_eq!(report.fusion.alignment, Alignment::Quiet);
}
