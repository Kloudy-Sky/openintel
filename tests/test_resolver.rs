//! Tests for the IntelResolver — market price resolution from intel DB.

use openintel::domain::ports::market_resolver::MarketResolver;
use openintel::domain::values::category::Category;
use openintel::domain::values::source_type::SourceType;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

#[tokio::test]
async fn test_resolver_empty_db_kalshi() {
    let oi = setup();
    let resolver = oi.market_resolver();
    let result = resolver.resolve("KXHIGHNY").await;
    assert!(result.is_none(), "Empty DB should resolve nothing");
}

#[tokio::test]
async fn test_resolver_empty_db_stock() {
    let oi = setup();
    let resolver = oi.market_resolver();
    let result = resolver.resolve("AAPL").await;
    assert!(result.is_none(), "Empty DB should resolve nothing");
}

#[tokio::test]
async fn test_resolver_stock_from_yahoo_feed() {
    let oi = setup();

    // Add a yahoo-feed entry for IONQ
    let meta = serde_json::json!({
        "price": 38.50,
        "change_pct": 2.1,
        "volume": 5000000
    });
    oi.add_intel(
        Category::Market,
        "IONQ @ $38.50".into(),
        "IonQ Inc current price".into(),
        Some("Yahoo Finance".into()),
        vec!["IONQ".into(), "yahoo-feed".into()],
        Some(0.9),
        Some(false),
        SourceType::Internal,
        Some(meta),
        true,
    )
    .await
    .unwrap();

    let resolver = oi.market_resolver();
    let result = resolver.resolve("IONQ").await;

    assert!(result.is_some(), "Should resolve IONQ from yahoo-feed");
    let resolved = result.unwrap();
    assert_eq!(resolved.contract_ticker, "IONQ");
    // Price should be in cents: $38.50 → 3850
    assert!((resolved.price_cents - 3850.0).abs() < 1.0);
    assert_eq!(
        format!("{}", resolved.exchange),
        "equity"
    );
}

#[tokio::test]
async fn test_resolver_kalshi_from_feed() {
    let oi = setup();

    // Add a kalshi-feed entry for KXHIGHNY
    let meta = serde_json::json!({
        "ticker": "KXHIGHNY-26FEB28-B44.5",
        "midpoint": 45.0,
        "yes_bid": 42.0,
        "yes_ask": 48.0,
        "volume_24h": 500,
        "open_interest": 1200
    });
    oi.add_intel(
        Category::Market,
        "KXHIGHNY B44.5 @ 45¢".into(),
        "NYC high temp band 44-45°F".into(),
        Some("Kalshi".into()),
        vec!["KXHIGHNY".into(), "kalshi-feed".into()],
        Some(0.9),
        Some(false),
        SourceType::Internal,
        Some(meta),
        true,
    )
    .await
    .unwrap();

    let resolver = oi.market_resolver();
    let result = resolver.resolve("KXHIGHNY").await;

    assert!(result.is_some(), "Should resolve KXHIGHNY from kalshi-feed");
    let resolved = result.unwrap();
    assert_eq!(resolved.contract_ticker, "KXHIGHNY-26FEB28-B44.5");
    assert!((resolved.price_cents - 45.0).abs() < 0.01);
    assert_eq!(format!("{}", resolved.exchange), "kalshi");
}

#[tokio::test]
async fn test_resolver_kalshi_picks_most_liquid() {
    let oi = setup();

    // Add two contracts — one with more liquidity
    let meta_low = serde_json::json!({
        "ticker": "KXHIGHNY-26FEB28-B42.5",
        "midpoint": 30.0,
        "yes_bid": 28.0,
        "yes_ask": 32.0,
        "volume_24h": 50,
        "open_interest": 100
    });
    oi.add_intel(
        Category::Market,
        "KXHIGHNY B42.5 low liquidity".into(),
        "Low volume contract".into(),
        Some("Kalshi".into()),
        vec!["KXHIGHNY".into(), "kalshi-feed".into()],
        Some(0.9),
        Some(false),
        SourceType::Internal,
        Some(meta_low),
        true,
    )
    .await
    .unwrap();

    let meta_high = serde_json::json!({
        "ticker": "KXHIGHNY-26FEB28-B44.5",
        "midpoint": 55.0,
        "yes_bid": 52.0,
        "yes_ask": 58.0,
        "volume_24h": 2000,
        "open_interest": 5000
    });
    oi.add_intel(
        Category::Market,
        "KXHIGHNY B44.5 high liquidity".into(),
        "High volume contract".into(),
        Some("Kalshi".into()),
        vec!["KXHIGHNY".into(), "kalshi-feed".into()],
        Some(0.9),
        Some(false),
        SourceType::Internal,
        Some(meta_high),
        true,
    )
    .await
    .unwrap();

    let resolver = oi.market_resolver();
    let result = resolver.resolve("KXHIGHNY").await.unwrap();
    assert_eq!(
        result.contract_ticker, "KXHIGHNY-26FEB28-B44.5",
        "Should pick the most liquid contract"
    );
}

#[tokio::test]
async fn test_resolver_skips_band_sum_entries() {
    let oi = setup();

    // Band-sum entry (aggregation, not a real contract)
    let meta = serde_json::json!({
        "ticker": "KXFED-AGGREGATE",
        "midpoint": 50.0,
        "yes_bid": 48.0,
        "yes_ask": 52.0,
        "volume_24h": 10000,
        "open_interest": 20000
    });
    oi.add_intel(
        Category::Market,
        "KXFED band sum".into(),
        "Fed rate bands summed".into(),
        Some("Kalshi".into()),
        vec!["KXFED".into(), "kalshi-feed".into(), "band-sum".into()],
        Some(0.9),
        Some(false),
        SourceType::Internal,
        Some(meta),
        true,
    )
    .await
    .unwrap();

    let resolver = oi.market_resolver();
    let result = resolver.resolve("KXFED").await;
    assert!(
        result.is_none(),
        "Band-sum entries should be skipped by resolver"
    );
}

#[tokio::test]
async fn test_resolver_skips_invalid_midpoint() {
    let oi = setup();

    // Contract with midpoint 0 (no real price)
    let meta = serde_json::json!({
        "ticker": "KXHIGHNY-26FEB28-T80",
        "midpoint": 0.0,
        "yes_bid": 0.0,
        "yes_ask": 1.0,
        "volume_24h": 100,
        "open_interest": 50
    });
    oi.add_intel(
        Category::Market,
        "KXHIGHNY T80".into(),
        "Extreme temp contract".into(),
        Some("Kalshi".into()),
        vec!["KXHIGHNY".into(), "kalshi-feed".into()],
        Some(0.9),
        Some(false),
        SourceType::Internal,
        Some(meta),
        true,
    )
    .await
    .unwrap();

    let resolver = oi.market_resolver();
    let result = resolver.resolve("KXHIGHNY").await;
    assert!(result.is_none(), "Midpoint 0 should be skipped");
}

#[tokio::test]
async fn test_resolver_ticker_case_insensitive() {
    let oi = setup();

    let meta = serde_json::json!({
        "price": 150.0,
        "change_pct": 1.0,
        "volume": 1000000
    });
    oi.add_intel(
        Category::Market,
        "AAPL @ $150".into(),
        "Apple current price".into(),
        Some("Yahoo Finance".into()),
        vec!["AAPL".into(), "yahoo-feed".into()],
        Some(0.9),
        Some(false),
        SourceType::Internal,
        Some(meta),
        true,
    )
    .await
    .unwrap();

    let resolver = oi.market_resolver();

    // Should resolve with lowercase input
    let result = resolver.resolve("aapl").await;
    // The resolver normalizes to uppercase, but tags are stored as-is
    // So "AAPL" tag matches "AAPL" (uppercased input)
    assert!(result.is_some(), "Should resolve case-insensitively");
}

#[tokio::test]
async fn test_resolver_name() {
    let oi = setup();
    let resolver = oi.market_resolver();
    assert_eq!(resolver.name(), "intel-db");
}
