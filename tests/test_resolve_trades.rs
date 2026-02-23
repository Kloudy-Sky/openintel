use openintel::domain::values::trade_direction::TradeDirection;
use openintel::domain::values::trade_outcome::TradeOutcome;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

#[test]
fn test_pending_no_trades() {
    let oi = setup();
    let report = oi.pending_trades().unwrap();
    assert_eq!(report.checked, 0);
    assert!(report.unresolved.is_empty());
}

#[test]
fn test_pending_with_open_trades() {
    let oi = setup();

    oi.trade_add("AAPL".into(), None, TradeDirection::Long, 10, 150.0, Some("Bullish".into()))
        .unwrap();
    oi.trade_add("TSLA".into(), None, TradeDirection::Short, 5, 200.0, None)
        .unwrap();

    let report = oi.pending_trades().unwrap();
    assert_eq!(report.checked, 2);
    assert_eq!(report.unresolved.len(), 2);
    assert!(report.resolved.is_empty());
}

#[test]
fn test_pending_excludes_resolved() {
    let oi = setup();

    let t1 = oi
        .trade_add("AAPL".into(), None, TradeDirection::Long, 10, 150.0, None)
        .unwrap();
    oi.trade_add("TSLA".into(), None, TradeDirection::Short, 5, 200.0, None)
        .unwrap();

    // Resolve one trade
    oi.trade_resolve(&t1.id, TradeOutcome::Win, 5000, Some(155.0)).unwrap();

    let report = oi.pending_trades().unwrap();
    assert_eq!(report.checked, 1); // Only TSLA
    assert_eq!(report.unresolved.len(), 1);
    assert_eq!(report.unresolved[0].ticker, "TSLA");
}

#[tokio::test]
async fn test_resolve_with_no_sources() {
    let oi = setup();
    oi.trade_add("BTC".into(), None, TradeDirection::Long, 1, 50000.0, None)
        .unwrap();

    // No resolution sources — all trades stay unresolved
    let report = oi.resolve_trades(&[]).await.unwrap();
    assert_eq!(report.checked, 1);
    assert_eq!(report.unresolved.len(), 1);
    assert!(report.resolved.is_empty());
}
