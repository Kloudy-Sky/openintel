use openintel::cli::run::analyze;
use openintel::config::settings::{AppConfig, OutputFormat};
use openintel::domain::values::speculation::Alignment;

fn cfg(reddit: bool, x: bool, bluesky: bool, no_market: bool) -> AppConfig {
    AppConfig::new(
        "AAPL".into(),
        reddit,
        x,
        bluesky,
        no_market,
        50,
        OutputFormat::Json,
    )
}

#[tokio::test]
async fn end_to_end_all_sources_with_market() {
    let (report, json) = analyze(&cfg(false, false, false, false)).await.unwrap();
    // 4 + 3 + 3 mock posts across reddit/x/bluesky (>= min_sample of 10)
    assert_eq!(report.social.total_mentions, 10);
    assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
    assert!(report.market.is_some());
    assert!(json.contains("\"alignment\": \"confirming_bullish\""));
    assert!(json.contains("Not financial advice"));
}

#[tokio::test]
async fn single_source_only() {
    let (report, _) = analyze(&cfg(true, false, false, false)).await.unwrap();
    assert_eq!(report.social.total_mentions, 4); // reddit fixtures only
}

#[tokio::test]
async fn social_only_when_market_disabled() {
    let (report, _) = analyze(&cfg(false, false, false, true)).await.unwrap();
    assert!(report.market.is_none());
    assert_eq!(report.fusion.alignment, Alignment::Quiet);
}
