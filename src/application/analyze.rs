use chrono::Utc;
use futures::future::join_all;

use crate::adapters::analyzer::lexicon::LexiconAnalyzer;
use crate::adapters::market::mock_market::MockMarketSource;
use crate::adapters::sources::mock_bluesky::MockBlueskySource;
use crate::adapters::sources::mock_reddit::MockRedditSource;
use crate::adapters::sources::mock_x::MockXSource;
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
use crate::domain::values::source_kind::SourceKind;

fn build_sources(req: &AnalysisRequest) -> Vec<Box<dyn SocialDataSource>> {
    req.enabled_sources
        .iter()
        .map(|kind| -> Box<dyn SocialDataSource> {
            match kind {
                SourceKind::Reddit => Box::new(MockRedditSource),
                SourceKind::X => Box::new(MockXSource),
                SourceKind::Bluesky => Box::new(MockBlueskySource),
            }
        })
        .collect()
}

pub async fn analyze(req: &AnalysisRequest) -> Result<SpeculationReport, DomainError> {
    let ticker = Ticker::parse(&req.ticker)?;
    let sources = build_sources(req);

    let fetches = sources.iter().map(|source| {
        let ticker = ticker.clone();
        async move { (source.kind(), source.fetch(&ticker, req.limit).await) }
    });
    let results = join_all(fetches).await;

    let mut posts: Vec<SocialPost> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (kind, result) in results {
        match result {
            Ok(mut fetched) => posts.append(&mut fetched),
            Err(e) => notes.push(format!("source {} failed: {e}", kind.as_str())),
        }
    }

    let market: Option<MarketSnapshot> = if req.market_enabled {
        match MockMarketSource.snapshot(&ticker).await {
            Ok(snapshot) => Some(snapshot),
            Err(e) => {
                notes.push(format!("market source failed: {e}"));
                None
            }
        }
    } else {
        None
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
        let report = analyze(&req("AAPL", true)).await.unwrap();
        assert_eq!(report.social.total_mentions, 10);
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
        assert!(report.market.is_some());
    }

    #[tokio::test]
    async fn invalid_ticker_errors() {
        assert!(analyze(&req("$$$", true)).await.is_err());
    }
}
