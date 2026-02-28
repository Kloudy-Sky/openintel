//! End-to-end tests exercising the full pipeline: add → query → search → 
//! opportunities → alerts → summarize → resolver.

use openintel::domain::ports::market_resolver::MarketResolver;
use openintel::domain::values::category::Category;
use openintel::domain::values::source_type::SourceType;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

/// Full pipeline: ingest market data, run opportunities, check alerts, summarize.
#[tokio::test]
async fn test_full_pipeline() {
    let oi = setup();

    // 1. Ingest diverse data simulating a feed run
    let tickers = ["NVDA", "AAPL", "MSFT"];
    for (i, ticker) in tickers.iter().enumerate() {
        // Yahoo-feed style entry
        let meta = serde_json::json!({
            "price": 100.0 + (i as f64 * 50.0),
            "change_pct": 2.0 + (i as f64),
            "volume": 1000000 * (i + 1)
        });
        oi.add_intel(
            Category::Market,
            format!("{} @ ${:.2}", ticker, 100.0 + (i as f64 * 50.0)),
            format!("{} current price from Yahoo Finance", ticker),
            Some("Yahoo Finance".into()),
            vec![ticker.to_string(), "yahoo-feed".into()],
            Some(0.9),
            Some(false),
            SourceType::Internal,
            Some(meta),
            true,
        )
        .await
        .unwrap();

        // Newsletter mention
        oi.add_intel(
            Category::Newsletter,
            format!("{} earnings momentum", ticker),
            format!("{} beat expectations with strong guidance, earnings growth", ticker),
            Some("Morning Brew".into()),
            vec![ticker.to_string(), "earnings".into()],
            Some(0.8),
            Some(true),
            SourceType::External,
            None,
            true,
        )
        .await
        .unwrap();

        // Social signal
        oi.add_intel(
            Category::Social,
            format!("{} trending on social", ticker),
            format!("Strong bullish sentiment on {} from multiple sources", ticker),
            Some("BlueSky".into()),
            vec![ticker.to_string(), "bullish".into()],
            Some(0.6),
            Some(true),
            SourceType::External,
            None,
            true,
        )
        .await
        .unwrap();
    }

    // Add a Kalshi feed entry
    let kalshi_meta = serde_json::json!({
        "ticker": "KXINXY-26DEC31-B7700",
        "midpoint": 11.0,
        "yes_bid": 9.0,
        "yes_ask": 13.0,
        "volume_24h": 500,
        "open_interest": 2000
    });
    oi.add_intel(
        Category::Market,
        "KXINXY B7700 @ 11¢".into(),
        "S&P 500 above 7700 by Dec 31".into(),
        Some("Kalshi".into()),
        vec!["KXINXY".into(), "kalshi-feed".into()],
        Some(0.9),
        Some(false),
        SourceType::Internal,
        Some(kalshi_meta),
        true,
    )
    .await
    .unwrap();

    // 2. Verify stats
    let stats = oi.stats().unwrap();
    assert_eq!(stats.total_entries, 10); // 3 tickers × 3 entries + 1 Kalshi

    // 3. Run keyword search
    let results = oi.keyword_search("earnings", 10).unwrap();
    assert!(results.len() >= 3, "Should find earnings entries for all 3 tickers");

    // 4. Run opportunities
    let scan = oi.opportunities(24, None, None, None).unwrap();
    assert_eq!(scan.entries_scanned, 10);
    assert_eq!(scan.strategies_run, 4);
    assert_eq!(scan.strategies_failed, 0);

    // 5. Run alerts
    let alerts = oi.scan_alerts(24).unwrap();
    assert_eq!(alerts.total_entries, 10);

    // 6. Summarize
    let summary = oi.summarize(24).unwrap();
    assert_eq!(summary.total_entries, 10);

    // 7. Resolve stock prices
    let resolver = oi.market_resolver();
    for ticker in &tickers {
        let result = resolver.resolve(ticker).await;
        assert!(result.is_some(), "Should resolve {}", ticker);
    }

    // 8. Resolve Kalshi
    let kalshi = resolver.resolve("KXINXY").await;
    assert!(kalshi.is_some(), "Should resolve KXINXY");
    let k = kalshi.unwrap();
    assert_eq!(k.contract_ticker, "KXINXY-26DEC31-B7700");
    assert!((k.price_cents - 11.0).abs() < 0.01);
}

/// Test trade lifecycle: add → list → resolve → verify.
#[tokio::test]
async fn test_trade_lifecycle() {
    let oi = setup();
    use openintel::domain::values::trade_direction::TradeDirection;
    use openintel::domain::values::trade_outcome::TradeOutcome;

    // Add a trade
    let trade = oi
        .trade_add(
            "KXHIGHNY-26FEB28-B44.5".into(),
            Some("KXHIGHNY".into()),
            TradeDirection::Yes,
            50,
            33.0,
            Some("Weather edge: NWS forecast 45°F".into()),
        )
        .unwrap();

    assert_eq!(trade.contracts, 50);
    assert!(!trade.is_resolved());

    // List open trades
    let open = oi.trade_list(None, None, None, Some(false)).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, trade.id);

    // Resolve the trade
    oi.trade_resolve(&trade.id, TradeOutcome::Win, 3350, Some(100.0))
        .unwrap();

    // Should now be resolved
    let resolved = oi.trade_list(None, None, None, Some(true)).unwrap();
    assert_eq!(resolved.len(), 1);
    assert!(resolved[0].is_resolved());

    // No more open trades
    let open = oi.trade_list(None, None, None, Some(false)).unwrap();
    assert!(open.is_empty());
}

/// Test opportunities with Kelly sizing through the public API.
#[tokio::test]
async fn test_opportunities_kelly_integration() {
    let oi = setup();

    // Seed enough data
    for i in 0..6 {
        let st = if i % 2 == 0 {
            SourceType::External
        } else {
            SourceType::Internal
        };
        oi.add_intel(
            Category::Market,
            format!("GOOGL signal {i}"),
            format!("Google earnings beat analysis {i}"),
            Some(format!("Source {i}")),
            vec!["GOOGL".into(), "earnings".into(), "beat".into()],
            Some(0.85),
            Some(true),
            st,
            None,
            true,
        )
        .await
        .unwrap();
    }

    // Run with Kelly
    let scan = oi
        .opportunities_with_sizing(24, None, None, None, Some(10000), None)
        .unwrap();

    // Verify no sizing without market_price
    for opp in &scan.opportunities {
        if opp.market_price.is_none() {
            assert!(opp.suggested_size_cents.is_none());
        }
    }
}

/// Test dedup across categories.
#[tokio::test]
async fn test_dedup_cross_category() {
    let oi = setup();

    // Same title, different categories — should NOT dedup
    oi.add_intel(
        Category::Market,
        "Fed rate decision".into(),
        "Fed holds rates".into(),
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
            "Fed rate decision".into(),
            "Newsletter coverage of Fed".into(),
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

    assert!(!r2.deduplicated, "Different categories should not dedup");
}
