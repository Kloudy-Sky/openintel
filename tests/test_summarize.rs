use openintel::domain::values::category::Category;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

#[tokio::test]
async fn test_summarize_empty_db() {
    let oi = setup();
    let summary = oi.summarize(24).unwrap();
    assert_eq!(summary.total_entries, 0);
    assert!(summary.by_category.is_empty());
    assert!(summary.top_tags.is_empty());
    assert!(summary.actionable_items.is_empty());
}

#[tokio::test]
async fn test_summarize_with_entries() {
    let oi = setup();

    // Add some entries
    oi.add_intel(
        Category::Market,
        "BTC breaks 100k".into(),
        "Bitcoin surges past $100k on ETF inflows".into(),
        Some("twitter".into()),
        vec!["btc".into(), "crypto".into()],
        Some(0.9),
        Some(true),
        None,
    )
    .await
    .unwrap();

    oi.add_intel(
        Category::Market,
        "ETH staking rewards up".into(),
        "Ethereum staking APY increases to 5%".into(),
        None,
        vec!["eth".into(), "crypto".into()],
        Some(0.7),
        Some(false),
        None,
    )
    .await
    .unwrap();

    oi.add_intel(
        Category::Newsletter,
        "Morning Brew highlights".into(),
        "Fed meeting, tech earnings, AI developments".into(),
        Some("morningbrew".into()),
        vec!["fed".into(), "ai".into()],
        None,
        Some(true),
        None,
    )
    .await
    .unwrap();

    let summary = oi.summarize(24).unwrap();

    assert_eq!(summary.total_entries, 3);

    // Market should have 2 entries
    let market = summary.by_category.iter().find(|c| c.category == "market");
    assert!(market.is_some());
    assert_eq!(market.unwrap().count, 2);

    // 'crypto' tag should appear twice
    let crypto_tag = summary.top_tags.iter().find(|t| t.tag == "crypto");
    assert!(crypto_tag.is_some());
    assert_eq!(crypto_tag.unwrap().count, 2);

    // 2 actionable items
    assert_eq!(summary.actionable_items.len(), 2);

    // Actionable items should be sorted by confidence desc
    assert!(summary.actionable_items[0].confidence >= summary.actionable_items[1].confidence);
}

#[tokio::test]
async fn test_summarize_respects_time_window() {
    let oi = setup();

    // Add an entry (it will have current timestamp)
    oi.add_intel(
        Category::General,
        "Recent entry".into(),
        "Just now".into(),
        None,
        vec![],
        None,
        None,
        None,
    )
    .await
    .unwrap();

    // Summary with 1 hour window should find it
    let summary = oi.summarize(1).unwrap();
    assert_eq!(summary.total_entries, 1);
}
