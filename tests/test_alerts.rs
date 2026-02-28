//! Tests for the AlertsUseCase — tag concentration, volume spikes,
//! actionable clusters.

mod common;

use common::setup;
use openintel::domain::values::category::Category;
use openintel::domain::values::source_type::SourceType;

#[tokio::test]
async fn test_alerts_empty_db() {
    let oi = setup();
    let scan = oi.scan_alerts(24).unwrap();
    assert_eq!(scan.total_entries, 0);
    assert!(scan.alerts.is_empty());
}

#[tokio::test]
async fn test_alerts_tag_concentration() {
    let oi = setup();

    for i in 0..12 {
        oi.add_intel(
            Category::Market,
            format!("Entry about AAPL #{i}"),
            format!("Apple news item {i}"),
            None,
            vec!["AAPL".into()],
            Some(0.7),
            Some(true),
            SourceType::External,
            None,
            true,
        )
        .await
        .unwrap();
    }

    let scan = oi.scan_alerts(24).unwrap();
    assert_eq!(scan.total_entries, 12);
    assert!(
        !scan.alerts.is_empty(),
        "12 entries with same tag should trigger alerts"
    );
}

#[tokio::test]
async fn test_alerts_window_hours() {
    let oi = setup();

    for i in 0..5 {
        oi.add_intel(
            Category::Market,
            format!("Recent {i}"),
            "Body".into(),
            None,
            vec!["test-tag".into()],
            None,
            Some(true),
            SourceType::External,
            None,
            true,
        )
        .await
        .unwrap();
    }

    let scan_24h = oi.scan_alerts(24).unwrap();
    let scan_1h = oi.scan_alerts(1).unwrap();

    assert_eq!(scan_24h.total_entries, 5);
    assert_eq!(scan_1h.total_entries, 5);
    assert_eq!(scan_24h.window_hours, 24);
    assert_eq!(scan_1h.window_hours, 1);
}

#[tokio::test]
async fn test_alerts_actionable_cluster() {
    let oi = setup();

    for i in 0..8 {
        oi.add_intel(
            Category::Market,
            format!("Actionable signal {i}"),
            format!("Strong buy signal for NVDA #{i}"),
            None,
            vec!["NVDA".into(), "buy-signal".into()],
            Some(0.9),
            Some(true),
            SourceType::External,
            None,
            true,
        )
        .await
        .unwrap();
    }

    let scan = oi.scan_alerts(24).unwrap();
    assert_eq!(scan.total_entries, 8);
    assert!(
        !scan.alerts.is_empty(),
        "8 actionable entries on same topic should trigger an alert"
    );
}

#[tokio::test]
async fn test_alerts_severity_values_are_valid() {
    let oi = setup();

    for i in 0..15 {
        oi.add_intel(
            Category::Market,
            format!("Signal {i}"),
            format!("Body {i}"),
            None,
            vec!["concentrated-tag".into()],
            Some(0.9),
            Some(true),
            SourceType::External,
            None,
            true,
        )
        .await
        .unwrap();
    }

    let scan = oi.scan_alerts(24).unwrap();
    assert!(
        !scan.alerts.is_empty(),
        "15 entries with same tag should produce alerts"
    );
    for alert in &scan.alerts {
        let json = serde_json::to_string(&alert.severity).unwrap();
        assert!(
            json == "\"info\"" || json == "\"warning\"" || json == "\"critical\"",
            "Invalid severity: {}",
            json
        );
    }
}
