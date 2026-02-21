use openintel::domain::values::category::Category;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

#[tokio::test]
async fn test_keyword_search() {
    let oi = setup();
    oi.add_intel.execute(
        Category::Market,
        "Bitcoin analysis".into(),
        "BTC is showing strength".into(),
        None, vec![], None, None, None,
    ).await.unwrap();

    oi.add_intel.execute(
        Category::Market,
        "Ethereum update".into(),
        "ETH gas fees dropping".into(),
        None, vec![], None, None, None,
    ).await.unwrap();

    let results = oi.search.keyword_search("Bitcoin", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].title.contains("Bitcoin"));
}

#[tokio::test]
async fn test_search_empty_results() {
    let oi = setup();
    let results = oi.search.keyword_search("nonexistent", 10).unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_search_body_match() {
    let oi = setup();
    oi.add_intel.execute(
        Category::General,
        "Title".into(),
        "The quick brown fox jumps".into(),
        None, vec![], None, None, None,
    ).await.unwrap();

    let results = oi.search.keyword_search("fox", 10).unwrap();
    assert_eq!(results.len(), 1);
}
