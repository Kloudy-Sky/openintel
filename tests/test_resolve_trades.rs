use async_trait::async_trait;
use openintel::domain::entities::trade::Trade;
use openintel::domain::error::DomainError;
use openintel::domain::ports::resolution_source::{ResolutionResult, ResolutionSource};
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

    oi.trade_add(
        "AAPL".into(),
        None,
        TradeDirection::Long,
        10,
        150.0,
        Some("Bullish".into()),
    )
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
    oi.trade_resolve(&t1.id, TradeOutcome::Win, 5000, Some(155.0))
        .unwrap();

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

/// Mock resolution source that resolves any trade it sees.
struct MockResolutionSource;

#[async_trait]
impl ResolutionSource for MockResolutionSource {
    fn name(&self) -> &str {
        "mock"
    }

    async fn check(&self, _trade: &Trade) -> Result<Option<ResolutionResult>, DomainError> {
        Ok(Some(ResolutionResult {
            outcome: TradeOutcome::Win,
            pnl_cents: 5000,
            exit_price: Some(155.0),
            reason: "Mock resolved".into(),
        }))
    }
}

/// Mock source that never resolves anything.
struct NeverResolvesSource;

#[async_trait]
impl ResolutionSource for NeverResolvesSource {
    fn name(&self) -> &str {
        "never"
    }

    async fn check(&self, _trade: &Trade) -> Result<Option<ResolutionResult>, DomainError> {
        Ok(None)
    }
}

#[tokio::test]
async fn test_resolve_with_mock_source() {
    let oi = setup();
    oi.trade_add("AAPL".into(), None, TradeDirection::Long, 10, 150.0, None)
        .unwrap();

    let sources: Vec<Arc<dyn ResolutionSource>> = vec![Arc::new(MockResolutionSource)];
    let report = oi.resolve_trades(&sources).await.unwrap();

    assert_eq!(report.checked, 1);
    assert_eq!(report.resolved.len(), 1);
    assert!(report.unresolved.is_empty());
    assert_eq!(report.resolved[0].ticker, "AAPL");
    assert_eq!(report.resolved[0].outcome, "win");
    assert_eq!(report.resolved[0].pnl_cents, 5000);
    assert_eq!(report.resolved[0].source, "mock");

    // Trade should no longer appear in pending
    let pending = oi.pending_trades().unwrap();
    assert_eq!(pending.checked, 0);
}

#[tokio::test]
async fn test_first_source_wins() {
    let oi = setup();
    oi.trade_add("TSLA".into(), None, TradeDirection::Short, 5, 200.0, None)
        .unwrap();

    // NeverResolves comes first, MockResolution comes second — mock should win
    let sources: Vec<Arc<dyn ResolutionSource>> = vec![
        Arc::new(NeverResolvesSource),
        Arc::new(MockResolutionSource),
    ];
    let report = oi.resolve_trades(&sources).await.unwrap();

    assert_eq!(report.resolved.len(), 1);
    assert_eq!(report.resolved[0].source, "mock");
}
