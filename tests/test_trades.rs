use openintel::domain::values::trade_direction::TradeDirection;
use openintel::domain::values::trade_outcome::TradeOutcome;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

#[test]
fn test_add_and_list_trade() {
    let oi = setup();
    let trade = oi
        .trade_add(
            "AAPL".into(),
            None,
            TradeDirection::Long,
            10,
            150.0,
            Some("Bullish on earnings".into()),
        )
        .unwrap();

    assert_eq!(trade.ticker, "AAPL");
    assert!(!trade.is_resolved());

    let trades = oi.trade_list(Some(10), None, None).unwrap();
    assert_eq!(trades.len(), 1);
}

#[test]
fn test_resolve_trade() {
    let oi = setup();
    let trade = oi
        .trade_add("TSLA".into(), None, TradeDirection::Short, 5, 200.0, None)
        .unwrap();
    oi.trade_resolve(&trade.id, TradeOutcome::Win, 5000, None)
        .unwrap();

    let trades = oi.trade_list(Some(10), None, Some(true)).unwrap();
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].outcome, Some(TradeOutcome::Win));
    assert_eq!(trades[0].pnl_cents, Some(5000));
}

#[test]
fn test_filter_unresolved() {
    let oi = setup();
    let t1 = oi
        .trade_add("A".into(), None, TradeDirection::Long, 1, 10.0, None)
        .unwrap();
    oi.trade_add("B".into(), None, TradeDirection::Short, 1, 20.0, None)
        .unwrap();
    oi.trade_resolve(&t1.id, TradeOutcome::Loss, -100, None)
        .unwrap();

    let unresolved = oi.trade_list(None, None, Some(false)).unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].ticker, "B");
}
