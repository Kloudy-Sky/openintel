//! Tests for RRF (Reciprocal Rank Fusion) scoring logic
//! Since we can't do real semantic search with NoopProvider,
//! we test the keyword path and verify the fusion doesn't crash.

use openintel::domain::values::category::Category;
use openintel::domain::values::source_type::SourceType;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

#[tokio::test]
async fn test_hybrid_search_falls_back_to_keyword() {
    let oi = setup();

    // Add entries
    for i in 0..5 {
        oi.add_intel(
            Category::Market,
            format!("Market signal {i}"),
            format!("Analysis of market trend {i}"),
            None,
            vec!["market".into()],
            None,
            None,
            SourceType::External,
            None,
            true, // skip dedup
        )
        .await
        .unwrap();
    }

    oi.add_intel(
        Category::General,
        "Unrelated entry".into(),
        "Nothing about markets".into(),
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

    // Hybrid search with noop embedder should still return keyword results
    let results = oi.hybrid_search("market signal", 3).await.unwrap();
    assert!(!results.is_empty());
    assert!(results.len() <= 3);
}

#[tokio::test]
async fn test_hybrid_search_empty_db() {
    let oi = setup();
    let results = oi.hybrid_search("anything", 10).await.unwrap();
    assert!(results.is_empty());
}
