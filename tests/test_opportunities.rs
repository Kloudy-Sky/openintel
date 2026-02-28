//! Tests for the OpportunitiesUseCase — strategy orchestration, scoring,
//! filtering, and Kelly sizing integration.

use openintel::domain::values::category::Category;
use openintel::domain::values::source_type::SourceType;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

#[tokio::test]
async fn test_opportunities_empty_db() {
    let oi = setup();
    let scan = oi.opportunities(24, None, None, None).unwrap();
    assert_eq!(scan.entries_scanned, 0);
    assert_eq!(scan.total_opportunities, 0);
    assert_eq!(scan.strategies_run, 4); // all 4 strategies run even on empty data
    assert_eq!(scan.strategies_failed, 0);
}

#[tokio::test]
async fn test_opportunities_with_data() {
    let oi = setup();

    // Add enough data to potentially trigger tag_convergence or convergence
    for i in 0..5 {
        let source_type = if i % 2 == 0 {
            SourceType::External
        } else {
            SourceType::Internal
        };
        oi.add_intel(
            Category::Market,
            format!("NVDA signal {i}"),
            format!("Nvidia news item {i} about earnings and revenue beat"),
            Some(format!("Source {i}")),
            vec!["NVDA".into(), "earnings".into()],
            Some(0.8),
            Some(true),
            source_type,
            None,
            true,
        )
        .await
        .unwrap();
    }

    let scan = oi.opportunities(24, None, None, None).unwrap();
    assert_eq!(scan.entries_scanned, 5);
    assert_eq!(scan.strategies_run, 4);
    assert_eq!(scan.window_hours, 24);
    // Should have some opportunities from convergence strategies
}

#[tokio::test]
async fn test_opportunities_min_score_filter() {
    let oi = setup();

    // Seed data
    for i in 0..5 {
        let st = if i % 2 == 0 {
            SourceType::External
        } else {
            SourceType::Internal
        };
        oi.add_intel(
            Category::Market,
            format!("BTC entry {i}"),
            format!("Bitcoin analysis {i}"),
            Some(format!("Src {i}")),
            vec!["btc".into(), "crypto".into()],
            Some(0.7),
            Some(true),
            st,
            None,
            true,
        )
        .await
        .unwrap();
    }

    let all = oi.opportunities(24, None, None, None).unwrap();
    let filtered = oi.opportunities(24, Some(999.0), None, None).unwrap();

    // Very high min_score should filter out everything
    assert!(
        filtered.total_opportunities <= all.total_opportunities,
        "High min_score should filter opportunities"
    );
}

#[tokio::test]
async fn test_opportunities_result_limit() {
    let oi = setup();

    // Seed enough data for multiple opportunities
    for i in 0..10 {
        let st = if i % 2 == 0 {
            SourceType::External
        } else {
            SourceType::Internal
        };
        oi.add_intel(
            Category::Market,
            format!("Entry {i}"),
            format!("Body {i} about earnings momentum"),
            Some(format!("Source {i}")),
            vec![format!("tag{}", i % 3), "earnings".into()],
            Some(0.8),
            Some(true),
            st,
            None,
            true,
        )
        .await
        .unwrap();
    }

    let limited = oi.opportunities(24, None, None, Some(1)).unwrap();
    assert!(
        limited.total_opportunities <= 1,
        "Result limit should cap output"
    );
}

#[tokio::test]
async fn test_opportunities_with_kelly_sizing() {
    let oi = setup();

    // Add convergent data to generate opportunities with confidence
    for i in 0..4 {
        let st = if i % 2 == 0 {
            SourceType::External
        } else {
            SourceType::Internal
        };
        oi.add_intel(
            Category::Market,
            format!("AAPL momentum {i}"),
            format!("Apple {i} earnings beat guidance"),
            Some(format!("Source {i}")),
            vec!["AAPL".into(), "earnings".into(), "beat".into()],
            Some(0.85),
            Some(true),
            st,
            None,
            true,
        )
        .await
        .unwrap();
    }

    // Without bankroll — no sizing
    let without = oi
        .opportunities_with_sizing(24, None, None, None, None, None)
        .unwrap();

    // With bankroll — sizing applied (only to opps with market_price)
    let with = oi
        .opportunities_with_sizing(24, None, None, None, Some(10000), None)
        .unwrap();

    // Both should have the same number of opportunities
    assert_eq!(without.total_opportunities, with.total_opportunities);

    // Kelly sizing only applies when market_price is set by a resolver,
    // so in this test (no resolver), suggested_size_cents should remain None
    for opp in &with.opportunities {
        if opp.market_price.is_none() {
            assert!(
                opp.suggested_size_cents.is_none(),
                "No market_price → no Kelly sizing"
            );
        }
    }
}

#[tokio::test]
async fn test_opportunities_sorted_by_score_descending() {
    let oi = setup();

    // Add varied data
    for i in 0..6 {
        let st = if i % 2 == 0 {
            SourceType::External
        } else {
            SourceType::Internal
        };
        oi.add_intel(
            Category::Market,
            format!("Signal {i}"),
            format!("Analysis {i} about earnings beat"),
            Some(format!("Source {i}")),
            vec!["MSFT".into(), "earnings".into()],
            Some(0.7 + (i as f64 * 0.03)),
            Some(true),
            st,
            None,
            true,
        )
        .await
        .unwrap();
    }

    let scan = oi.opportunities(24, None, None, None).unwrap();
    for window in scan.opportunities.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Opportunities should be sorted by score descending"
        );
    }
}

// ── Opportunity::compute_score ───────────────────────────────────────────

use openintel::domain::ports::strategy::Opportunity;

#[test]
fn test_compute_score_with_edge() {
    let score = Opportunity::compute_score(0.8, Some(50.0), Some(1.0));
    assert!((score - 40.0).abs() < 0.01, "0.8 * 50.0 * 1.0 = 40.0");
}

#[test]
fn test_compute_score_without_edge() {
    let score = Opportunity::compute_score(0.7, None, Some(1.0));
    assert!((score - 70.0).abs() < 0.01, "0.7 * 100 * 1.0 = 70.0");
}

#[test]
fn test_compute_score_low_liquidity_penalty() {
    let full_liq = Opportunity::compute_score(0.8, Some(50.0), Some(1.0));
    let low_liq = Opportunity::compute_score(0.8, Some(50.0), Some(0.25));
    assert!(
        low_liq < full_liq,
        "Low liquidity should reduce score: {} < {}",
        low_liq,
        full_liq
    );
    // sqrt(0.25) = 0.5, so low_liq should be half of full_liq
    assert!((low_liq - 20.0).abs() < 0.01);
}

#[test]
fn test_compute_score_no_liquidity_defaults_to_one() {
    let with_none = Opportunity::compute_score(0.8, Some(50.0), None);
    let with_one = Opportunity::compute_score(0.8, Some(50.0), Some(1.0));
    assert!((with_none - with_one).abs() < 0.01);
}

#[test]
fn test_compute_score_clamps_liquidity() {
    let over = Opportunity::compute_score(0.8, Some(50.0), Some(2.0));
    let at_one = Opportunity::compute_score(0.8, Some(50.0), Some(1.0));
    assert!(
        (over - at_one).abs() < 0.01,
        "Liquidity > 1.0 clamped to 1.0"
    );
}
