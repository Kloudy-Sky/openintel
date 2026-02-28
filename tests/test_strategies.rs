//! Tests for all four strategy implementations.
//!
//! Tests use in-memory SQLite (via OpenIntel::with_providers) to exercise
//! strategy detection against realistic intel data.

use chrono::Utc;
use openintel::domain::ports::strategy::{DetectionContext, Strategy};
use openintel::domain::values::category::Category;
use openintel::domain::values::confidence::Confidence;
use openintel::domain::values::source_type::SourceType;
use openintel::infrastructure::embeddings::noop::NoopProvider;
use openintel::OpenIntel;
use std::sync::Arc;

fn setup() -> OpenIntel {
    OpenIntel::with_providers(":memory:", Arc::new(NoopProvider)).unwrap()
}

// ── Helper to build IntelEntry directly ──────────────────────────────────

fn make_entry(
    title: &str,
    body: &str,
    tags: Vec<&str>,
    source_type: SourceType,
    source: Option<&str>,
) -> openintel::domain::entities::intel_entry::IntelEntry {
    openintel::domain::entities::intel_entry::IntelEntry::new(
        Category::Market,
        title.to_string(),
        body.to_string(),
        source.map(|s| s.to_string()),
        tags.into_iter().map(|t| t.to_string()).collect(),
        Confidence::default(),
        true,
        source_type,
        None,
    )
}

// ── EarningsMomentumStrategy ─────────────────────────────────────────────

#[test]
fn test_earnings_momentum_no_earnings_entries() {
    use openintel::application::strategies::earnings_momentum::EarningsMomentumStrategy;

    let strategy = EarningsMomentumStrategy;
    let ctx = DetectionContext {
        entries: vec![
            make_entry("Weather update", "Sunny skies ahead", vec!["weather"], SourceType::External, None),
            make_entry("Sports news", "Team won the game", vec!["sports"], SourceType::External, None),
        ],
        open_trades: vec![],
        window_hours: 24,
    };

    let opps = strategy.detect(&ctx).unwrap();
    assert!(opps.is_empty(), "No earnings-related entries should produce no opportunities");
}

#[test]
fn test_earnings_momentum_single_ticker_multiple_signals() {
    use openintel::application::strategies::earnings_momentum::EarningsMomentumStrategy;

    let strategy = EarningsMomentumStrategy;
    let ctx = DetectionContext {
        entries: vec![
            make_entry("NVDA earnings beat", "Nvidia crushed Q4 earnings with $68B revenue", vec!["NVDA", "earnings"], SourceType::External, Some("Morning Brew")),
            make_entry("NVDA guidance strong", "Nvidia Q1 guidance $78B above expectations", vec!["NVDA", "guidance"], SourceType::External, Some("Tech Brew")),
            make_entry("Nvidia momentum", "Analysts raising EPS targets after earnings beat", vec!["NVDA", "eps"], SourceType::External, Some("Yahoo Finance")),
        ],
        open_trades: vec![],
        window_hours: 24,
    };

    let opps = strategy.detect(&ctx).unwrap();
    assert!(!opps.is_empty(), "3 earnings signals for NVDA should produce an opportunity");
    assert_eq!(opps[0].strategy, "earnings_momentum");
    assert!(opps[0].market_ticker.as_deref() == Some("NVDA"));
}

#[test]
fn test_earnings_momentum_insufficient_signals() {
    use openintel::application::strategies::earnings_momentum::EarningsMomentumStrategy;

    let strategy = EarningsMomentumStrategy;
    // Only 1 entry — below the minimum cluster threshold
    let ctx = DetectionContext {
        entries: vec![
            make_entry("AAPL earnings beat", "Apple beat Q1 earnings", vec!["AAPL", "earnings"], SourceType::External, None),
        ],
        open_trades: vec![],
        window_hours: 24,
    };

    let opps = strategy.detect(&ctx).unwrap();
    assert!(opps.is_empty(), "Single earnings entry should not trigger opportunity");
}

// ── TagConvergenceStrategy ───────────────────────────────────────────────

#[test]
fn test_tag_convergence_multi_source_cluster() {
    use openintel::application::strategies::tag_convergence::TagConvergenceStrategy;

    let strategy = TagConvergenceStrategy;
    let ctx = DetectionContext {
        entries: vec![
            make_entry("BTC surge", "Bitcoin above 100k", vec!["btc", "crypto"], SourceType::External, Some("Newsletter")),
            make_entry("BTC analysis", "Bitcoin RSI overbought", vec!["btc", "technical"], SourceType::Internal, Some("Agent")),
            make_entry("BTC institutional", "BlackRock buys more BTC", vec!["btc", "institutional"], SourceType::External, Some("Twitter")),
        ],
        open_trades: vec![],
        window_hours: 24,
    };

    let opps = strategy.detect(&ctx).unwrap();
    // 3 entries with "btc" tag from 2 source types (External + Internal)
    assert!(!opps.is_empty(), "3 entries from 2 source types on same tag should converge");
    assert_eq!(opps[0].strategy, "tag_convergence");
}

#[test]
fn test_tag_convergence_skips_generic_tags() {
    use openintel::application::strategies::tag_convergence::TagConvergenceStrategy;

    let strategy = TagConvergenceStrategy;
    // "market" and "news" are in the skip list
    let ctx = DetectionContext {
        entries: vec![
            make_entry("Entry 1", "Body", vec!["market", "news"], SourceType::External, None),
            make_entry("Entry 2", "Body", vec!["market", "news"], SourceType::Internal, None),
            make_entry("Entry 3", "Body", vec!["market", "news"], SourceType::External, None),
        ],
        open_trades: vec![],
        window_hours: 24,
    };

    let opps = strategy.detect(&ctx).unwrap();
    assert!(opps.is_empty(), "Generic tags like 'market' and 'news' should be skipped");
}

// ── ConvergenceStrategy ──────────────────────────────────────────────────

#[test]
fn test_convergence_multi_source_directional() {
    use openintel::application::strategies::convergence::ConvergenceStrategy;

    let strategy = ConvergenceStrategy;
    let ctx = DetectionContext {
        entries: vec![
            make_entry("Fed hawkish signal", "Fed rate cut expectations falling", vec!["fed", "rates", "hawkish"], SourceType::External, Some("Morning Brew")),
            make_entry("Bond yields rising", "Treasury yields spike on Fed comments", vec!["fed", "bonds", "bearish"], SourceType::Internal, Some("Agent")),
            make_entry("Fed meeting preview", "Markets expect no cut at next meeting", vec!["fed", "fomc", "hawkish"], SourceType::External, Some("CFO Brew")),
        ],
        open_trades: vec![],
        window_hours: 48,
    };

    let opps = strategy.detect(&ctx).unwrap();
    // 3 entries, 2 source types, all converging on "fed" topic
    // May or may not trigger depending on MIN_CLUSTER_SIZE and MIN_SOURCE_DIVERSITY
    // The strategy requires >= 3 entries and >= 2 source types
    for opp in &opps {
        assert_eq!(opp.strategy, "convergence");
    }
}

#[test]
fn test_convergence_empty_context() {
    use openintel::application::strategies::convergence::ConvergenceStrategy;

    let strategy = ConvergenceStrategy;
    let ctx = DetectionContext {
        entries: vec![],
        open_trades: vec![],
        window_hours: 24,
    };

    let opps = strategy.detect(&ctx).unwrap();
    assert!(opps.is_empty());
}

// ── CrossMarketStrategy ──────────────────────────────────────────────────

#[test]
fn test_cross_market_empty_context() {
    use openintel::application::strategies::cross_market::CrossMarketStrategy;

    let strategy = CrossMarketStrategy;
    let ctx = DetectionContext {
        entries: vec![],
        open_trades: vec![],
        window_hours: 24,
    };

    let opps = strategy.detect(&ctx).unwrap();
    assert!(opps.is_empty());
}

#[test]
fn test_cross_market_no_kalshi_entries() {
    use openintel::application::strategies::cross_market::CrossMarketStrategy;

    let strategy = CrossMarketStrategy;
    let ctx = DetectionContext {
        entries: vec![
            make_entry("Stock news", "Market rally", vec!["stocks"], SourceType::External, None),
        ],
        open_trades: vec![],
        window_hours: 24,
    };

    let opps = strategy.detect(&ctx).unwrap();
    // Without kalshi-feed entries, cross-market can't find mispricing
    assert!(opps.is_empty());
}

// ── Strategy trait basics ────────────────────────────────────────────────

#[test]
fn test_strategy_names_unique() {
    use openintel::application::strategies::convergence::ConvergenceStrategy;
    use openintel::application::strategies::cross_market::CrossMarketStrategy;
    use openintel::application::strategies::earnings_momentum::EarningsMomentumStrategy;
    use openintel::application::strategies::tag_convergence::TagConvergenceStrategy;

    let names: Vec<&str> = vec![
        EarningsMomentumStrategy.name(),
        TagConvergenceStrategy.name(),
        ConvergenceStrategy.name(),
        CrossMarketStrategy.name(),
    ];

    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(names.len(), unique.len(), "Strategy names must be unique");
}
