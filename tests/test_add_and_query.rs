use openintel::domain::values::category::Category;
use openintel::domain::values::confidence::Confidence;
use openintel::domain::values::source_type::SourceType;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

#[tokio::test]
async fn test_add_and_query_by_category() {
    let oi = setup();
    oi.add_intel(
        Category::Market,
        "BTC rally".into(),
        "Bitcoin surging past 100k".into(),
        Some("twitter".into()),
        vec!["btc".into(), "crypto".into()],
        Some(0.8),
        Some(true),
        SourceType::External,
        None,
        false,
    )
    .await
    .unwrap();

    oi.add_intel(
        Category::Newsletter,
        "Weekly digest".into(),
        "Summary of macro events".into(),
        None,
        vec!["macro".into()],
        None,
        None,
        SourceType::External,
        None,
        false,
    )
    .await
    .unwrap();

    let results = oi
        .query(Some(Category::Market), None, None, None, None)
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "BTC rally");
    assert!(results[0].actionable);
}

#[tokio::test]
async fn test_query_by_tag() {
    let oi = setup();
    oi.add_intel(
        Category::Market,
        "Tagged entry".into(),
        "Body".into(),
        None,
        vec!["alpha".into(), "beta".into()],
        None,
        None,
        SourceType::External,
        None,
        false,
    )
    .await
    .unwrap();

    oi.add_intel(
        Category::Market,
        "Other entry".into(),
        "Body".into(),
        None,
        vec!["gamma".into()],
        None,
        None,
        SourceType::External,
        None,
        false,
    )
    .await
    .unwrap();

    let results = oi
        .query(
            Some(Category::Market),
            Some("alpha".into()),
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Tagged entry");
}

#[tokio::test]
async fn test_stats() {
    let oi = setup();
    for i in 0..5 {
        oi.add_intel(
            Category::Market,
            format!("Entry {i}"),
            "Body".into(),
            None,
            vec!["tag1".into()],
            None,
            Some(i % 2 == 0),
            SourceType::External,
            None,
            true, // skip dedup since titles are unique anyway
        )
        .await
        .unwrap();
    }

    let stats = oi.stats().unwrap();
    assert_eq!(stats.total_entries, 5);
    assert_eq!(stats.actionable_count, 3); // 0,2,4
}

#[tokio::test]
async fn test_dedup_same_title_same_category() {
    let oi = setup();
    let r1 = oi
        .add_intel(
            Category::Market,
            "Fed cuts rates".into(),
            "First report".into(),
            None,
            vec![],
            None,
            None,
            SourceType::External,
            None,
            false,
        )
        .await
        .unwrap();
    assert!(!r1.deduplicated);

    // Same title, same category — should dedup
    let r2 = oi
        .add_intel(
            Category::Market,
            "Fed cuts rates".into(),
            "Second report from different source".into(),
            None,
            vec![],
            None,
            None,
            SourceType::External,
            None,
            false,
        )
        .await
        .unwrap();
    assert!(r2.deduplicated);
    assert_eq!(r2.entry.id, r1.entry.id);

    let all = oi
        .query(Some(Category::Market), None, None, None, None)
        .unwrap();
    assert_eq!(all.len(), 1);
}

#[tokio::test]
async fn test_dedup_case_insensitive() {
    let oi = setup();
    let r1 = oi
        .add_intel(
            Category::Market,
            "BTC Rally".into(),
            "Body".into(),
            None,
            vec![],
            None,
            None,
            SourceType::External,
            None,
            false,
        )
        .await
        .unwrap();
    assert!(!r1.deduplicated);

    let r2 = oi
        .add_intel(
            Category::Market,
            "btc rally".into(),
            "Different body".into(),
            None,
            vec![],
            None,
            None,
            SourceType::External,
            None,
            false,
        )
        .await
        .unwrap();
    assert!(r2.deduplicated);
}

#[tokio::test]
async fn test_dedup_different_category_not_deduped() {
    let oi = setup();
    oi.add_intel(
        Category::Market,
        "Fed cuts rates".into(),
        "Body".into(),
        None,
        vec![],
        None,
        None,
        SourceType::External,
        None,
        false,
    )
    .await
    .unwrap();

    let r2 = oi
        .add_intel(
            Category::Newsletter,
            "Fed cuts rates".into(),
            "Same title different category".into(),
            None,
            vec![],
            None,
            None,
            SourceType::External,
            None,
            false,
        )
        .await
        .unwrap();
    assert!(!r2.deduplicated);
}

#[tokio::test]
async fn test_skip_dedup_flag() {
    let oi = setup();
    oi.add_intel(
        Category::Market,
        "Repeating signal".into(),
        "First".into(),
        None,
        vec![],
        None,
        None,
        SourceType::External,
        None,
        false,
    )
    .await
    .unwrap();

    // With skip_dedup=true, should add regardless
    let r2 = oi
        .add_intel(
            Category::Market,
            "Repeating signal".into(),
            "Force add".into(),
            None,
            vec![],
            None,
            None,
            SourceType::External,
            None,
            true,
        )
        .await
        .unwrap();
    assert!(!r2.deduplicated);

    let all = oi
        .query(Some(Category::Market), None, None, None, None)
        .unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn test_source_type_filtering() {
    let oi = setup();
    oi.add_intel(
        Category::Market,
        "External signal".into(),
        "From newsletter".into(),
        None,
        vec![],
        None,
        None,
        SourceType::External,
        None,
        false,
    )
    .await
    .unwrap();

    oi.add_intel(
        Category::Market,
        "Internal note".into(),
        "Agent heartbeat log".into(),
        None,
        vec![],
        None,
        None,
        SourceType::Internal,
        None,
        false,
    )
    .await
    .unwrap();

    // All entries
    let all = oi
        .query(Some(Category::Market), None, None, None, None)
        .unwrap();
    assert_eq!(all.len(), 2);

    // Exclude internal
    let external_only = oi
        .query(
            Some(Category::Market),
            None,
            None,
            None,
            Some(SourceType::Internal),
        )
        .unwrap();
    assert_eq!(external_only.len(), 1);
    assert_eq!(external_only[0].title, "External signal");
}

#[test]
fn test_confidence_validation() {
    assert!(Confidence::new(0.0).is_ok());
    assert!(Confidence::new(1.0).is_ok());
    assert!(Confidence::new(0.5).is_ok());
    assert!(Confidence::new(-0.1).is_err());
    assert!(Confidence::new(1.1).is_err());
}

#[tokio::test]
async fn test_query_with_date_range() {
    let oi = setup();

    // Add entries
    oi.add_intel(
        Category::Market,
        "Recent entry".into(),
        "Just happened".into(),
        None,
        vec!["recent".into()],
        None,
        None,
        None,
    )
    .await
    .unwrap();

    // Query with --from (since) set to 1 hour ago should return the entry
    let one_hour_ago = chrono::Utc::now() - chrono::Duration::hours(1);
    let results = oi
        .query(Some(Category::Market), None, Some(one_hour_ago), None, None)
        .unwrap();
    assert_eq!(results.len(), 1);

    // Query with --from set to 1 hour in the future should return nothing
    let one_hour_future = chrono::Utc::now() + chrono::Duration::hours(1);
    let results = oi
        .query(
            Some(Category::Market),
            None,
            Some(one_hour_future),
            None,
            None,
        )
        .unwrap();
    assert_eq!(results.len(), 0);

    // Query with --to (until) set to 1 hour ago should return nothing
    let results = oi
        .query(Some(Category::Market), None, None, Some(one_hour_ago), None)
        .unwrap();
    assert_eq!(results.len(), 0);

    // Query with --to set to 1 hour in the future should return the entry
    let results = oi
        .query(
            Some(Category::Market),
            None,
            None,
            Some(one_hour_future),
            None,
        )
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_keyword_search_with_time() {
    let oi = setup();

    oi.add_intel(
        Category::Market,
        "Bitcoin analysis".into(),
        "BTC is looking bullish".into(),
        None,
        vec![],
        None,
        None,
        None,
    )
    .await
    .unwrap();

    // Search with time range that includes the entry
    let one_hour_ago = chrono::Utc::now() - chrono::Duration::hours(1);
    let results = oi
        .keyword_search_with_time("Bitcoin", 10, Some(one_hour_ago), None)
        .unwrap();
    assert_eq!(results.len(), 1);

    // Search with time range that excludes the entry
    let one_hour_future = chrono::Utc::now() + chrono::Duration::hours(1);
    let results = oi
        .keyword_search_with_time("Bitcoin", 10, Some(one_hour_future), None)
        .unwrap();
    assert_eq!(results.len(), 0);
}
