use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::domain::engine::config::EngineConfig;
use crate::domain::entities::market_snapshot::MarketSnapshot;
use crate::domain::entities::social_post::SocialPost;
use crate::domain::entities::speculation_report::{
    FusionSignals, MarketSummary, SocialSummary, SpeculationReport,
};
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::values::polarity::Polarity;
use crate::domain::values::post_signal::PostSignal;
use crate::domain::values::source_kind::SourceKind;
use crate::domain::values::speculation::{Alignment, Confidence, SpeculationIndex};

pub struct SpeculationEngine;

impl SpeculationEngine {
    pub fn aggregate(
        ticker: &Ticker,
        posts: &[SocialPost],
        signals: &[PostSignal],
        market: Option<&MarketSnapshot>,
        now: DateTime<Utc>,
        cfg: &EngineConfig,
    ) -> Result<SpeculationReport, DomainError> {
        if signals.len() != posts.len() {
            return Err(DomainError::AnalyzerMismatch {
                expected: posts.len(),
                got: signals.len(),
            });
        }

        if let Some(m) = market {
            if m.ticker.as_str() != ticker.as_str() {
                return Err(DomainError::MarketTickerMismatch {
                    expected: ticker.as_str().to_string(),
                    got: m.ticker.as_str().to_string(),
                });
            }
        }

        let mut notes: Vec<String> = Vec::new();
        let social = Self::social_summary(posts, signals, cfg);
        let market_summary = market.map(|m| Self::market_summary(m, &mut notes));
        let crowding = Self::crowding(&social, market_summary.as_ref(), cfg);
        let alignment = Self::alignment(&social, market_summary.as_ref(), cfg, &mut notes);
        let social_confidence = Confidence::from_sample(
            social.total_mentions,
            cfg.confidence_low,
            cfg.confidence_high,
        );

        Ok(SpeculationReport {
            ticker: ticker.clone(),
            generated_at: now,
            social,
            market: market_summary,
            fusion: FusionSignals {
                alignment,
                crowding,
                notes,
            },
            social_confidence,
        })
    }

    fn social_summary(
        posts: &[SocialPost],
        signals: &[PostSignal],
        cfg: &EngineConfig,
    ) -> SocialSummary {
        let total = posts.len();
        let mut by_source: BTreeMap<SourceKind, usize> = BTreeMap::new();
        for p in posts {
            *by_source.entry(p.source).or_insert(0) += 1;
        }

        let (mut bullish, mut bearish, mut neutral, mut spec_count) =
            (0usize, 0usize, 0usize, 0usize);
        let mut polarity_sum = 0.0f64;
        for s in signals {
            let v = s.polarity.value();
            polarity_sum += v;
            if v > cfg.bull_bear_threshold {
                bullish += 1;
            } else if v < -cfg.bull_bear_threshold {
                bearish += 1;
            } else {
                neutral += 1;
            }
            if s.speculative {
                spec_count += 1;
            }
        }

        let net = if total == 0 {
            0.0
        } else {
            polarity_sum / total as f64
        };
        let spec_index = if total == 0 {
            0.0
        } else {
            spec_count as f64 / total as f64
        };
        let bull_bear_ratio = if bearish == 0 {
            None
        } else {
            Some(bullish as f64 / bearish as f64)
        };

        SocialSummary {
            total_mentions: total,
            mentions_by_source: by_source,
            net_sentiment: Polarity::new(net),
            bullish,
            bearish,
            neutral,
            bull_bear_ratio,
            speculation_index: SpeculationIndex::new(spec_index),
        }
    }

    fn market_summary(m: &MarketSnapshot, notes: &mut Vec<String>) -> MarketSummary {
        let pct_change = if m.previous_close == 0.0 {
            notes.push("previous_close is 0; pct_change set to 0".to_string());
            0.0
        } else {
            (m.last_price - m.previous_close) / m.previous_close * 100.0
        };
        let rvol = if m.avg_volume == 0 {
            notes.push("avg_volume is 0; rvol unavailable".into());
            None
        } else {
            Some(m.volume as f64 / m.avg_volume as f64)
        };
        MarketSummary {
            last_price: m.last_price,
            pct_change,
            rvol,
            realized_vol: m.realized_vol,
            put_call_ratio: m.put_call_ratio,
            iv_rank: m.iv_rank,
        }
    }

    /// Weighted blend of available components, renormalized over present weights.
    fn crowding(social: &SocialSummary, market: Option<&MarketSummary>, cfg: &EngineConfig) -> f64 {
        let mut weighted = 0.0f64;
        let mut weight_sum = 0.0f64;

        if social.total_mentions > 0 {
            weighted += cfg.crowding_weight_spec * social.speculation_index.value();
            weight_sum += cfg.crowding_weight_spec;
        }
        if let Some(m) = market {
            if let Some(rvol) = m.rvol {
                let rvol_norm = (rvol / cfg.rvol_cap).clamp(0.0, 1.0);
                weighted += cfg.crowding_weight_rvol * rvol_norm;
                weight_sum += cfg.crowding_weight_rvol;
            }
            if let Some(iv) = m.iv_rank {
                weighted += cfg.crowding_weight_iv * iv.clamp(0.0, 1.0);
                weight_sum += cfg.crowding_weight_iv;
            }
        }

        if weight_sum == 0.0 {
            0.0
        } else {
            (weighted / weight_sum).clamp(0.0, 1.0)
        }
    }

    fn alignment(
        social: &SocialSummary,
        market: Option<&MarketSummary>,
        cfg: &EngineConfig,
        notes: &mut Vec<String>,
    ) -> Alignment {
        let market = match market {
            None => {
                notes.push("social-only, no price reference".to_string());
                return Alignment::Quiet;
            }
            Some(m) => m,
        };
        if social.total_mentions < cfg.min_sample {
            return Alignment::Quiet;
        }

        let s = social.net_sentiment.value();
        let p = market.pct_change;
        let sentiment_meaningful = s.abs() >= cfg.net_sentiment_threshold;
        let price_meaningful = p.abs() >= cfg.price_move_threshold;
        if !sentiment_meaningful || !price_meaningful {
            return Alignment::Quiet;
        }

        match (s > 0.0, p > 0.0) {
            (true, true) => Alignment::ConfirmingBullish,
            (false, false) => Alignment::ConfirmingBearish,
            _ => Alignment::Diverging,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::social_post::PostText;
    use chrono::TimeZone;

    fn ticker() -> Ticker {
        Ticker::parse("AAPL").unwrap()
    }
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 24, 0, 0, 0).unwrap()
    }
    fn post(source: SourceKind) -> SocialPost {
        SocialPost {
            id: "x".into(),
            source,
            author: "a".into(),
            text: PostText::parse("placeholder").unwrap(),
            created_at: now(),
            engagement: 0,
        }
    }
    fn sig(polarity: f64, speculative: bool) -> PostSignal {
        PostSignal {
            polarity: Polarity::new(polarity),
            speculative,
        }
    }
    fn snapshot(last: f64, prev: f64, vol: u64, avg: u64, iv: Option<f64>) -> MarketSnapshot {
        MarketSnapshot {
            ticker: ticker(),
            as_of: now(),
            last_price: last,
            previous_close: prev,
            volume: vol,
            avg_volume: avg,
            realized_vol: None,
            put_call_ratio: None,
            iv_rank: iv,
        }
    }
    /// 12 posts: 9 bullish (+0.8), 3 neutral (0.0) — net ≈ 0.6, all reddit.
    fn bullish_batch() -> (Vec<SocialPost>, Vec<PostSignal>) {
        let posts: Vec<_> = (0..12).map(|_| post(SourceKind::Reddit)).collect();
        let mut signals = vec![sig(0.8, true); 9];
        signals.extend(vec![sig(0.0, false); 3]);
        (posts, signals)
    }

    #[test]
    fn confirming_bullish_when_sentiment_and_price_agree() {
        let (posts, signals) = bullish_batch();
        let m = snapshot(110.0, 100.0, 1, 1, Some(0.5)); // +10%
        let report = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            Some(&m),
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBullish);
        assert_eq!(report.social.bullish, 9);
        assert_eq!(report.social_confidence, Confidence::Medium); // 12 mentions
        assert!(report.market.is_some());
    }

    #[test]
    fn diverging_when_sentiment_up_but_price_down() {
        let (posts, signals) = bullish_batch();
        let m = snapshot(90.0, 100.0, 1, 1, None); // -10%
        let report = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            Some(&m),
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert_eq!(report.fusion.alignment, Alignment::Diverging);
    }

    #[test]
    fn empty_input_is_quiet_and_zeroed() {
        let report = SpeculationEngine::aggregate(
            &ticker(),
            &[],
            &[],
            None,
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert_eq!(report.social.total_mentions, 0);
        assert_eq!(report.social.net_sentiment.value(), 0.0);
        assert_eq!(report.social.speculation_index.value(), 0.0);
        assert_eq!(report.fusion.alignment, Alignment::Quiet);
        assert_eq!(report.fusion.crowding, 0.0);
        assert_eq!(report.social_confidence, Confidence::Low);
    }

    #[test]
    fn no_market_forces_quiet_alignment() {
        let (posts, signals) = bullish_batch();
        let report = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            None,
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert!(report.market.is_none());
        assert_eq!(report.fusion.alignment, Alignment::Quiet);
        assert!(report
            .fusion
            .notes
            .iter()
            .any(|n| n.contains("social-only")));
    }

    #[test]
    fn length_mismatch_errors() {
        let posts = vec![post(SourceKind::Reddit), post(SourceKind::Reddit)];
        let signals = vec![sig(0.5, false)];
        let err = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            None,
            now(),
            &EngineConfig::default(),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            DomainError::AnalyzerMismatch {
                expected: 2,
                got: 1
            }
        ));
    }

    #[test]
    fn bull_bear_ratio_is_none_without_bears() {
        let posts = vec![post(SourceKind::Reddit)];
        let signals = vec![sig(0.9, false)];
        let report = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            None,
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert_eq!(report.social.bull_bear_ratio, None);
    }

    #[test]
    fn rvol_guarded_when_avg_volume_zero() {
        let posts = vec![post(SourceKind::Reddit)];
        let signals = vec![sig(0.0, false)];
        let m = snapshot(100.0, 100.0, 10, 0, None);
        let report = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            Some(&m),
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert!(report.market.unwrap().rvol.is_none());
        assert!(report.fusion.notes.iter().any(|n| n.contains("avg_volume")));
    }

    #[test]
    fn crowding_renormalizes_when_rvol_unavailable() {
        // 1 speculative post (spec_index 1.0), avg_volume=0 so rvol omitted, iv_rank=None.
        // Only spec weight present: weighted = 0.5*1.0, weight_sum = 0.5 → crowding = 1.0.
        let posts = vec![post(SourceKind::Reddit)];
        let signals = vec![sig(0.0, true)];
        let m = snapshot(100.0, 100.0, 0, 0, None);
        let report = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            Some(&m),
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert!(
            (report.fusion.crowding - 1.0).abs() < 1e-9,
            "got {}",
            report.fusion.crowding
        );
    }

    #[test]
    fn market_ticker_mismatch_errors() {
        let msft = MarketSnapshot {
            ticker: Ticker::parse("MSFT").unwrap(),
            as_of: now(),
            last_price: 100.0,
            previous_close: 100.0,
            volume: 1,
            avg_volume: 1,
            realized_vol: None,
            put_call_ratio: None,
            iv_rank: None,
        };
        let err = SpeculationEngine::aggregate(
            &ticker(), // AAPL
            &[],
            &[],
            Some(&msft),
            now(),
            &EngineConfig::default(),
        )
        .unwrap_err();
        assert!(
            matches!(err, DomainError::MarketTickerMismatch { .. }),
            "expected MarketTickerMismatch, got {err:?}"
        );
    }

    #[test]
    fn crowding_renormalizes_without_market() {
        // social-only: every post speculative -> speculation_index 1.0 -> crowding == 1.0
        let posts: Vec<_> = (0..3).map(|_| post(SourceKind::Reddit)).collect();
        let signals = vec![sig(0.0, true); 3];
        let report = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            None,
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert_eq!(report.fusion.crowding, 1.0);
    }

    #[test]
    fn confirming_bearish_when_sentiment_and_price_agree_down() {
        let posts: Vec<_> = (0..12).map(|_| post(SourceKind::Reddit)).collect();
        let mut signals = vec![sig(-0.8, true); 9];
        signals.extend(vec![sig(0.0, false); 3]);
        let m = snapshot(90.0, 100.0, 1, 1, None); // -10%
        let report = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            Some(&m),
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert_eq!(report.fusion.alignment, Alignment::ConfirmingBearish);
    }

    #[test]
    fn min_sample_gate_quiet_even_with_agreeing_market() {
        // 5 mentions < min_sample (10): Quiet despite sentiment+price agreeing, market present.
        let posts: Vec<_> = (0..5).map(|_| post(SourceKind::Reddit)).collect();
        let signals = vec![sig(0.8, true); 5];
        let m = snapshot(110.0, 100.0, 1, 1, Some(0.5)); // +10%
        let report = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            Some(&m),
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert!(report.market.is_some());
        assert_eq!(report.fusion.alignment, Alignment::Quiet);
    }

    #[test]
    fn previous_close_zero_guarded() {
        let posts = vec![post(SourceKind::Reddit)];
        let signals = vec![sig(0.0, false)];
        let m = snapshot(100.0, 0.0, 10, 10, None);
        let report = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            Some(&m),
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert_eq!(report.market.unwrap().pct_change, 0.0);
        assert!(report
            .fusion
            .notes
            .iter()
            .any(|n| n.contains("previous_close")));
    }

    #[test]
    fn crowding_uses_market_and_iv_branch_and_renormalizes() {
        // 1 non-speculative post (spec_index 0), market rvol = 10/10 = 1.0 -> rvol_norm = 1/3.
        let posts = vec![post(SourceKind::Reddit)];
        let signals = vec![sig(0.0, false)];
        // iv present: weighted = 0.5*0 + 0.3*(1/3) + 0.2*0.5 = 0.2 ; weight_sum = 1.0 -> crowding 0.2
        let with_iv = snapshot(100.0, 100.0, 10, 10, Some(0.5));
        let r1 = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            Some(&with_iv),
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert!(
            (r1.fusion.crowding - 0.2).abs() < 1e-9,
            "got {}",
            r1.fusion.crowding
        );
        // iv absent: weighted = 0.1 ; weight_sum = 0.8 -> crowding 0.125 (renormalized, NOT deflated to 0.1)
        let no_iv = snapshot(100.0, 100.0, 10, 10, None);
        let r2 = SpeculationEngine::aggregate(
            &ticker(),
            &posts,
            &signals,
            Some(&no_iv),
            now(),
            &EngineConfig::default(),
        )
        .unwrap();
        assert!(
            (r2.fusion.crowding - 0.125).abs() < 1e-9,
            "got {}",
            r2.fusion.crowding
        );
    }
}
