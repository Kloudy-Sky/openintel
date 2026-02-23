use openintel::application::alerts::{AlertKind, AlertSeverity};
use openintel::domain::values::category::Category;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

#[tokio::test]
async fn test_scan_empty_db() {
    let oi = setup();
    let scan = oi.scan_alerts(24).unwrap();
    assert_eq!(scan.total_entries, 0);
    assert!(scan.alerts.is_empty());
}

#[tokio::test]
async fn test_tag_concentration_alert() {
    let oi = setup();

    // Add 5 entries all tagged "btc" → Warning severity
    for i in 0..5 {
        oi.add_intel(
            Category::Market,
            format!("BTC signal {i}"),
            format!("Bitcoin analysis {i}"),
            None,
            vec!["btc".into()],
            None,
            None,
            None,
        )
        .await
        .unwrap();
    }

    let scan = oi.scan_alerts(24).unwrap();
    assert!(!scan.alerts.is_empty());

    let tag_alert = scan
        .alerts
        .iter()
        .find(|a| a.kind == AlertKind::TagConcentration)
        .expect("Should have tag concentration alert");
    assert!(tag_alert.title.contains("btc"));
    assert!(tag_alert.title.contains("5"));
    assert_eq!(tag_alert.severity, AlertSeverity::Warning);
}

#[tokio::test]
async fn test_tag_concentration_critical() {
    let oi = setup();

    // Add 10 entries all tagged "crash" → Critical severity
    for i in 0..10 {
        oi.add_intel(
            Category::Market,
            format!("Crash signal {i}"),
            format!("Market crash indicator {i}"),
            None,
            vec!["crash".into()],
            None,
            None,
            None,
        )
        .await
        .unwrap();
    }

    let scan = oi.scan_alerts(24).unwrap();
    let tag_alert = scan
        .alerts
        .iter()
        .find(|a| a.kind == AlertKind::TagConcentration)
        .expect("Should have tag concentration alert");
    assert_eq!(tag_alert.severity, AlertSeverity::Critical);
    assert!(tag_alert.title.contains("10"));
}

#[tokio::test]
async fn test_volume_spike_alert() {
    let oi = setup();

    // Add 7 entries in one category (baseline ~2 per 24h, spike threshold = 6)
    for i in 0..7 {
        oi.add_intel(
            Category::Market,
            format!("Market entry {i}"),
            format!("Content {i}"),
            None,
            vec![],
            None,
            None,
            None,
        )
        .await
        .unwrap();
    }

    let scan = oi.scan_alerts(24).unwrap();
    let volume_alert = scan
        .alerts
        .iter()
        .find(|a| a.kind == AlertKind::VolumeSpike)
        .expect("Should detect volume spike with 7 entries");
    assert_eq!(volume_alert.severity, AlertSeverity::Warning);
}

#[tokio::test]
async fn test_actionable_cluster_alert() {
    let oi = setup();

    // Add 3 high-confidence actionable entries → Warning severity
    for i in 0..3 {
        oi.add_intel(
            Category::Market,
            format!("Actionable signal {i}"),
            format!("High confidence trade idea {i}"),
            None,
            vec![],
            Some(0.9),
            Some(true),
            None,
        )
        .await
        .unwrap();
    }

    let scan = oi.scan_alerts(24).unwrap();
    let cluster_alert = scan
        .alerts
        .iter()
        .find(|a| a.kind == AlertKind::ActionableCluster)
        .expect("Should detect actionable cluster");
    assert_eq!(cluster_alert.severity, AlertSeverity::Warning);
}

#[tokio::test]
async fn test_actionable_cluster_critical() {
    let oi = setup();

    // Add 5 high-confidence actionable entries → Critical severity
    for i in 0..5 {
        oi.add_intel(
            Category::Market,
            format!("Critical signal {i}"),
            format!("Urgent trade idea {i}"),
            None,
            vec![],
            Some(0.95),
            Some(true),
            None,
        )
        .await
        .unwrap();
    }

    let scan = oi.scan_alerts(24).unwrap();
    let cluster_alert = scan
        .alerts
        .iter()
        .find(|a| a.kind == AlertKind::ActionableCluster)
        .expect("Should detect actionable cluster");
    assert_eq!(cluster_alert.severity, AlertSeverity::Critical);
}

#[tokio::test]
async fn test_no_alerts_below_thresholds() {
    let oi = setup();

    // Add 1 entry — should not trigger any alerts
    oi.add_intel(
        Category::General,
        "Single entry".into(),
        "Just one".into(),
        None,
        vec!["lonely".into()],
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let scan = oi.scan_alerts(24).unwrap();
    assert!(scan.alerts.is_empty());
}
