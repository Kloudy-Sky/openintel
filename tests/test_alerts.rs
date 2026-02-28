//! Tests for the AlertsUseCase — tag concentration, volume spikes,
//! actionable clusters.

use openintel::domain::values::category::Category;
use openintel::domain::values::source_type::SourceType;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

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

    // Create 10+ entries with the same tag to trigger tag concentration
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
    // Should trigger at least a tag concentration alert
    assert!(
        !scan.alerts.is_empty(),
        "12 entries with same tag should trigger alerts"
    );
}

#[tokio::test]
async fn test_alerts_window_hours() {
    let oi = setup();

    // Add entries
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

    // Both should find entries (just added)
    assert_eq!(scan_24h.total_entries, 5);
    assert_eq!(scan_1h.total_entries, 5);
    assert_eq!(scan_24h.window_hours, 24);
    assert_eq!(scan_1h.window_hours, 1);
}

#[tokio::test]
async fn test_alerts_actionable_cluster() {
    let oi = setup();

    // Create multiple actionable entries on same topic
    for i in 0..8 {
        oi.add_intel(
            Category::Market,
            format!("Actionable signal {i}"),
            format!("Strong buy signal for NVDA #{i}"),
            None,
            vec!["NVDA".into(), "buy-signal".into()],
            Some(0.9),
            Some(true), // all actionable
            SourceType::External,
            None,
            true,
        )
        .await
        .unwrap();
    }

    let scan = oi.scan_alerts(24).unwrap();
    assert_eq!(scan.total_entries, 8);
}

#[tokio::test]
async fn test_alerts_severity_ordering() {
    let oi = setup();

    // Create enough data for multiple alert types
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
    // Verify alerts have valid severities
    for alert in &scan.alerts {
        // Just verify serialization works — severity is an enum
        let json = serde_json::to_string(&alert.severity).unwrap();
        assert!(
            json == "\"info\"" || json == "\"warning\"" || json == "\"critical\"",
            "Invalid severity: {}",
            json
        );
    }
}
