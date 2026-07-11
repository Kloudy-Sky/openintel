use chrono::Utc;
use futures::future::join_all;

use crate::adapters::analyzer::lexicon::LexiconAnalyzer;
use crate::application::request::AnalysisRequest;
use crate::domain::engine::speculation_engine::SpeculationEngine;
use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::social_post::SocialPost;
use crate::domain::entities::speculation_report::SpeculationReport;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::ports::market_data_source::MarketDataSource;
use crate::domain::ports::post_analyzer::PostAnalyzer;
use crate::domain::ports::social_data_source::SocialDataSource;

pub async fn analyze(
    req: &AnalysisRequest,
    social_sources: &[Box<dyn SocialDataSource>],
    market_source: Option<&dyn MarketDataSource>,
) -> Result<SpeculationReport, DomainError> {
    let ticker = Ticker::parse(&req.ticker)?;

    let mut notes: Vec<String> = Vec::new();
    for kind in &req.enabled_sources {
        if !social_sources.iter().any(|s| s.kind() == *kind) {
            notes.push(format!("{} enabled but not configured", kind.as_str()));
        }
    }

    let fetches = social_sources
        .iter()
        .filter(|s| req.enabled_sources.contains(&s.kind()))
        .map(|source| {
            let ticker = ticker.clone();
            async move { (source.kind(), source.fetch(&ticker, req.limit).await) }
        });
    let results = join_all(fetches).await;

    let mut posts: Vec<SocialPost> = Vec::new();
    for (kind, result) in results {
        match result {
            Ok(mut fetched) => posts.append(&mut fetched),
            Err(e) => notes.push(format!("source {} failed: {e}", kind.as_str())),
        }
    }

    let market: Option<MarketSnapshot> = match (req.market_enabled, market_source) {
        (true, Some(source)) => match source.snapshot(&ticker).await {
            Ok(snapshot) => Some(snapshot),
            Err(e) => {
                notes.push(format!("market source failed: {e}"));
                None
            }
        },
        _ => None,
    };

    if posts.is_empty() && market.is_none() {
        return Err(DomainError::NoData);
    }

    let analyzer = LexiconAnalyzer::new();
    let signals = analyzer.analyze(&posts).await?;

    let now = Utc::now();
    let mut report =
        SpeculationEngine::aggregate(&ticker, &posts, &signals, market.as_ref(), now, &req.engine)?;

    let engine_notes = std::mem::take(&mut report.fusion.notes);
    report.fusion.notes = notes.into_iter().chain(engine_notes).collect();

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::market::mock_market::MockMarketSource;
    use crate::adapters::sources::test_fixtures::fixture_social;
    use crate::domain::values::source_kind::SourceKind;

    fn req(ticker: &str, market: bool) -> AnalysisRequest {
        AnalysisRequest {
            ticker: ticker.into(),
            enabled_sources: SourceKind::ALL.to_vec(),
            market_enabled: market,
            limit: 50,
            engine: crate::domain::engine::config::EngineConfig::default(),
        }
    }

    #[tokio::test]
    async fn analyzes_default_request_confirming_bullish() {
        use crate::domain::values::speculation::Alignment;
        let report = analyze(
            &req("AAPL", true),
            &fixture_social(),
            Some(&MockMarketSource),
        )
        .await
        .unwrap();
        assert_eq!(report.social.total_mentions, 10);
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
        assert!(report.market.is_some());
    }

    #[tokio::test]
    async fn invalid_ticker_errors() {
        assert!(analyze(
            &req("$$$", true),
            &fixture_social(),
            Some(&MockMarketSource)
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn social_only_when_no_source_provided() {
        use crate::domain::values::speculation::Alignment;
        let report = analyze(&req("AAPL", false), &fixture_social(), None)
            .await
            .unwrap();
        assert!(report.market.is_none());
        assert_eq!(report.fusion.alignment, Alignment::Quiet);
    }

    #[tokio::test]
    async fn enabled_source_absent_is_noted() {
        // reddit enabled but not wired -> note + the bluesky fixture still counted (6 posts)
        let social: Vec<Box<dyn SocialDataSource>> = vec![Box::new(
            crate::adapters::sources::test_fixtures::bluesky_fixture(),
        )];
        let report = analyze(&req("AAPL", false), &social, None).await.unwrap();
        assert_eq!(report.social.total_mentions, 6);
        assert!(report
            .fusion
            .notes
            .iter()
            .any(|n| n.contains("reddit enabled but not configured")));
    }
}
