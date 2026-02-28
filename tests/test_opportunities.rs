//! Tests for the OpportunitiesUseCase — strategy orchestration, scoring,
//! filtering, and Kelly sizing integration.

mod common;

use common::setup;
use openintel::domain::values::category::Category;
use openintel::domain::values::source_type::SourceType;

#[tokio::test]
async fn test_opportunities_empty_db() {
    let oi = setup();
    let scan = oi.opportunities(24, None, None, None).unwrap();
    assert_eq!(scan.entries_scanned, 0);
    assert_eq!(scan.total_opportunities, 0);
    assert_eq!(scan.strategies_run, 4);
    assert_eq!(scan.strategies_failed, 0);
}

#[tokio::test]
async fn test_opportunities_scans_all_strategies() {
    let oi = setup();

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
    assert_eq!(scan.strategies_failed, 0);
    assert_eq!(scan.window_hours, 24);
}

#[tokio::test]
async fn test_opportunities_min_score_filter() {
    let oi = setup();

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

    let filtered = oi.opportunities(24, Some(999.0), None, None).unwrap();
    assert_eq!(
        filtered.total_opportunities, 0,
        "min_score 999.0 should filter all opportunities"
    );
}

#[tokio::test]
async fn test_opportunities_result_limit() {
    let oi = setup();

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
async fn test_opportunities_with_kelly_sizing_no_market_price() {
    let oi = setup();

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

    let scan = oi
        .opportunities_with_sizing(24, None, None, None, Some(10000), None)
        .unwrap();

    // Without a resolver, no opportunities have market_price set,
    // so Kelly sizing should NOT produce suggested_size_cents
    for opp in &scan.opportunities {
        if opp.market_price.is_none() {
            assert!(
                opp.suggested_size_cents.is_none(),
                "No market_price → no Kelly sizing"
            );
        }
    }
}

#[tokio::test]
async fn test_opportunities_without_sizing_has_no_size() {
    let oi = setup();

    for i in 0..4 {
        let st = if i % 2 == 0 {
            SourceType::External
        } else {
            SourceType::Internal
        };
        oi.add_intel(
            Category::Market,
            format!("MSFT signal {i}"),
            format!("Microsoft {i} earnings analysis"),
            Some(format!("Source {i}")),
            vec!["MSFT".into(), "earnings".into()],
            Some(0.8),
            Some(true),
            st,
            None,
            true,
        )
        .await
        .unwrap();
    }

    let scan = oi
        .opportunities_with_sizing(24, None, None, None, None, None)
        .unwrap();

    // Without bankroll, no sizing at all
    for opp in &scan.opportunities {
        assert!(
            opp.suggested_size_cents.is_none(),
            "No bankroll → no suggested_size_cents"
        );
    }
}

#[tokio::test]
async fn test_opportunities_sorted_by_score_descending() {
    let oi = setup();

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
            "Opportunities should be sorted by score descending: {} >= {}",
            window[0].score,
            window[1].score
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
    assert!(low_liq < full_liq);
    assert!((low_liq - 20.0).abs() < 0.01, "sqrt(0.25) = 0.5, so 40 * 0.5 = 20");
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
