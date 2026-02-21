use openintel::domain::values::category::Category;
use openintel::domain::values::confidence::Confidence;
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
        None,
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
        None,
    )
    .await
    .unwrap();

    let results = oi.query(Some(Category::Market), None, None, None).unwrap();
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
        None,
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
        None,
    )
    .await
    .unwrap();

    let results = oi
        .query(Some(Category::Market), Some("alpha".into()), None, None)
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
            None,
        )
        .await
        .unwrap();
    }

    let stats = oi.stats().unwrap();
    assert_eq!(stats.total_entries, 5);
    assert_eq!(stats.actionable_count, 3); // 0,2,4
}

#[test]
fn test_confidence_validation() {
    assert!(Confidence::new(0.0).is_ok());
    assert!(Confidence::new(1.0).is_ok());
    assert!(Confidence::new(0.5).is_ok());
    assert!(Confidence::new(-0.1).is_err());
    assert!(Confidence::new(1.1).is_err());
}
