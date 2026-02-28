//! Tests for domain execution types — serialization, display, and value semantics.

use openintel::domain::values::execution::{ExecutionMode, ExecutionResult, SkippedOpportunity, TradePlan};

#[test]
fn test_execution_mode_display() {
    assert_eq!(ExecutionMode::DryRun.to_string(), "dry_run");
    assert_eq!(ExecutionMode::Live.to_string(), "live");
}

#[test]
fn test_execution_mode_serialization() {
    let dry = serde_json::to_string(&ExecutionMode::DryRun).unwrap();
    assert_eq!(dry, "\"dry_run\"");

    let live = serde_json::to_string(&ExecutionMode::Live).unwrap();
    assert_eq!(live, "\"live\"");
}

#[test]
fn test_execution_mode_deserialization() {
    let dry: ExecutionMode = serde_json::from_str("\"dry_run\"").unwrap();
    assert!(matches!(dry, ExecutionMode::DryRun));

    let live: ExecutionMode = serde_json::from_str("\"live\"").unwrap();
    assert!(matches!(live, ExecutionMode::Live));
}

#[test]
fn test_trade_plan_serialization_roundtrip() {
    let plan = TradePlan {
        ticker: "KXHIGHNY-26FEB28-B44.5".into(),
        direction: "yes".into(),
        size_cents: 2500,
        confidence: 0.75,
        score: 42.0,
        edge_cents: Some(15.0),
        action: "BUY".into(),
        description: "Weather trade".into(),
    };

    let json = serde_json::to_string(&plan).unwrap();
    let back: TradePlan = serde_json::from_str(&json).unwrap();
    assert_eq!(back.ticker, "KXHIGHNY-26FEB28-B44.5");
    assert_eq!(back.size_cents, 2500);
    assert!((back.confidence - 0.75).abs() < 0.001);
    assert_eq!(back.edge_cents.unwrap(), 15.0);
}

#[test]
fn test_skipped_opportunity_serialization() {
    let skipped = SkippedOpportunity {
        title: "Fed rate signal".into(),
        confidence: 0.45,
        score: 30.0,
        reason: "Below minimum confidence threshold".into(),
    };

    let json = serde_json::to_string(&skipped).unwrap();
    assert!(json.contains("Below minimum confidence"));
    let back: SkippedOpportunity = serde_json::from_str(&json).unwrap();
    assert_eq!(back.title, "Fed rate signal");
}

#[test]
fn test_execution_result_serialization() {
    let result = ExecutionResult {
        timestamp: "2026-02-28T02:00:00Z".into(),
        mode: ExecutionMode::DryRun,
        bankroll_cents: 7700,
        feeds_ingested: 298,
        feed_errors: vec!["NWS timeout".into()],
        opportunities_scanned: 379,
        trades_qualified: 2,
        trades_skipped: 148,
        total_deployment_cents: 2695,
        trades: vec![TradePlan {
            ticker: "KXINXY-S&P".into(),
            direction: "yes".into(),
            size_cents: 1500,
            confidence: 0.8,
            score: 55.0,
            edge_cents: Some(20.0),
            action: "BUY".into(),
            description: "S&P signal".into(),
        }],
        skipped: vec![SkippedOpportunity {
            title: "Low conf signal".into(),
            confidence: 0.3,
            score: 10.0,
            reason: "Confidence 0.30 < 0.65 threshold".into(),
        }],
    };

    let json = serde_json::to_string_pretty(&result).unwrap();
    assert!(json.contains("dry_run"));
    assert!(json.contains("7700"));
    assert!(json.contains("NWS timeout"));

    let back: ExecutionResult = serde_json::from_str(&json).unwrap();
    assert_eq!(back.feeds_ingested, 298);
    assert_eq!(back.trades.len(), 1);
    assert_eq!(back.skipped.len(), 1);
    assert_eq!(back.bankroll_cents, 7700);
}

#[test]
fn test_execution_result_empty() {
    let result = ExecutionResult {
        timestamp: "2026-02-28T00:00:00Z".into(),
        mode: ExecutionMode::DryRun,
        bankroll_cents: 0,
        feeds_ingested: 0,
        feed_errors: vec![],
        opportunities_scanned: 0,
        trades_qualified: 0,
        trades_skipped: 0,
        total_deployment_cents: 0,
        trades: vec![],
        skipped: vec![],
    };

    let json = serde_json::to_string(&result).unwrap();
    let back: ExecutionResult = serde_json::from_str(&json).unwrap();
    assert!(back.trades.is_empty());
    assert!(back.feed_errors.is_empty());
}
