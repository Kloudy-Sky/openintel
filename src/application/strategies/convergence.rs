//! Cross-intel convergence detection strategy.
//!
//! Detects when multiple independent signals from different source types
//! converge on the same thesis within a time window. This is where real
//! alpha lives: no single source is reliable, but when newsletter intel +
//! market signals + social posts all point the same direction, confidence
//! should be much higher.
//!
//! Implements Issue #21.
//!
//! **Note on overlap with `TagConvergenceStrategy`:** Both strategies cluster
//! entries by tag, but this strategy adds directional alignment scoring,
//! time-decay weighting, and position-awareness. Some overlap in output is
//! expected — cross-strategy deduplication is planned for a future PR.

use std::collections::{HashMap, HashSet};

use chrono::Utc;

use crate::domain::error::DomainError;
use crate::domain::ports::strategy::{DetectionContext, Direction, Opportunity, Strategy};

/// Minimum number of entries in a cluster to consider it.
const MIN_CLUSTER_SIZE: usize = 3;
/// Minimum source diversity (unique source types) for convergence.
const MIN_SOURCE_DIVERSITY: usize = 2;
/// Minimum confidence threshold to emit an opportunity.
const MIN_CONFIDENCE: f64 = 0.4;

/// Tracks a cluster of related intel entries converging on a topic.
struct Cluster {
    /// Canonical topic name (tag or entity).
    topic: String,
    /// Entry IDs in this cluster.
    entry_ids: Vec<String>,
    /// Unique source types seen.
    source_types: HashSet<String>,
    /// Unique sources (e.g., "Morning Brew", "Yahoo Finance").
    sources: HashSet<String>,
    /// Bullish/positive signal count (time-weighted).
    bullish: f64,
    /// Bearish/negative signal count (time-weighted).
    bearish: f64,
    /// Ticker symbols found in entries (only tags that were already uppercase).
    tickers: HashSet<String>,
    /// Titles for description building.
    titles: Vec<String>,
}

impl Cluster {
    fn new(topic: String) -> Self {
        Self {
            topic,
            entry_ids: Vec::new(),
            source_types: HashSet::new(),
            sources: HashSet::new(),
            bullish: 0.0,
            bearish: 0.0,
            tickers: HashSet::new(),
            titles: Vec::new(),
        }
    }

    fn source_diversity(&self) -> usize {
        self.source_types.len()
    }

    fn directional_alignment(&self) -> f64 {
        let total = self.bullish + self.bearish;
        if total < 0.01 {
            return 0.5; // neutral — no directional signal
        }
        (self.bullish - self.bearish).abs() / total
    }

    fn dominant_direction(&self) -> Option<Direction> {
        if self.bullish < 0.01 && self.bearish < 0.01 {
            return None;
        }
        if self.bullish > self.bearish {
            Some(Direction::Bullish)
        } else if self.bearish > self.bullish {
            Some(Direction::Bearish)
        } else {
            None // mixed signals
        }
    }

    /// Get tickers sorted for deterministic output.
    fn sorted_tickers(&self) -> Vec<String> {
        let mut t: Vec<String> = self.tickers.iter().cloned().collect();
        t.sort();
        t
    }
}

/// Detects cross-source convergence on shared topics with directional alignment.
///
/// Unlike `TagConvergenceStrategy` which only counts co-occurrence,
/// this strategy also:
/// - Applies time-decay weighting (recent entries contribute more to sentiment)
/// - Checks directional alignment (bullish/bearish consensus)
/// - Provides richer confidence scoring based on source diversity + alignment
/// - Identifies tradeable tickers within convergence clusters
/// - Detects overlap with existing open positions
pub struct ConvergenceStrategy;

const BULLISH_WORDS: &[&str] = &[
    "beat", "surge", "jump", "rally", "strong", "raised", "bullish", "soar", "boom", "gain",
    "growth", "positive", "upside", "momentum", "buy", "higher",
];

const BEARISH_WORDS: &[&str] = &[
    "miss", "drop", "fell", "weak", "lowered", "disappointing", "cut", "bearish", "crash",
    "decline", "loss", "negative", "downside", "sell", "lower", "warning", "risk",
];

/// Tags to skip when clustering (too generic to be useful).
const SKIP_TAGS: &[&str] = &[
    "market", "signal", "update", "analysis", "news", "general", "trade", "stock", "stocks",
    "economy", "finance", "investing", "investment",
];

impl Strategy for ConvergenceStrategy {
    fn name(&self) -> &'static str {
        "convergence"
    }

    fn detect(&self, ctx: &DetectionContext) -> Result<Vec<Opportunity>, DomainError> {
        let now = Utc::now();
        let skip_tags: HashSet<&str> = SKIP_TAGS.iter().copied().collect();

        // Build clusters from tags
        let mut clusters: HashMap<String, Cluster> = HashMap::new();

        for entry in &ctx.entries {
            let source_type = entry.source_type.to_string();
            let source = entry.source.clone().unwrap_or_default();
            let text = format!("{} {}", entry.title, entry.body).to_lowercase();

            // Time-decay weight: recent entries contribute more to sentiment.
            // Half-life ~35 hours (decay constant 0.02/hr).
            let age_hours = (now - entry.created_at).num_hours().max(0) as f64;
            let time_weight = (-0.02 * age_hours).exp();

            // Count sentiment, weighted by recency
            let bullish =
                BULLISH_WORDS.iter().filter(|w| text.contains(**w)).count() as f64 * time_weight;
            let bearish =
                BEARISH_WORDS.iter().filter(|w| text.contains(**w)).count() as f64 * time_weight;

            for tag in &entry.tags {
                let tag_lower = tag.to_lowercase();
                if skip_tags.contains(tag_lower.as_str()) || tag_lower.len() < 2 {
                    continue;
                }

                let cluster = clusters
                    .entry(tag_lower.clone())
                    .or_insert_with(|| Cluster::new(tag_lower.clone()));

                // Deduplicate entries within cluster
                if cluster.entry_ids.contains(&entry.id) {
                    continue;
                }

                cluster.entry_ids.push(entry.id.clone());
                cluster.source_types.insert(source_type.clone());
                if !source.is_empty() {
                    cluster.sources.insert(source.clone());
                }
                cluster.bullish += bullish;
                cluster.bearish += bearish;
                cluster.titles.push(entry.title.clone());

                // Only treat tags that were ALREADY uppercase as tickers.
                // This avoids false positives like "china" → "CHINA".
                if !tag.is_empty()
                    && tag.len() <= 5
                    && tag.chars().all(|c| c.is_ascii_uppercase())
                {
                    cluster.tickers.insert(tag.to_string());
                }
            }
        }

        // Check for existing positions to avoid duplicate signals
        let active_tickers: HashSet<String> = ctx
            .open_trades
            .iter()
            .map(|t| t.ticker.to_uppercase())
            .collect();

        let mut opportunities = Vec::new();

        for cluster in clusters.values() {
            if cluster.entry_ids.len() < MIN_CLUSTER_SIZE {
                continue;
            }
            if cluster.source_diversity() < MIN_SOURCE_DIVERSITY {
                continue;
            }

            let alignment = cluster.directional_alignment();

            // Confidence: base from entry count, boosted by source diversity and alignment.
            // Formula: base × (1 + 0.15 × (source_diversity - 1)) × alignment_factor
            // The -1 ensures a single source type gets no diversity boost.
            let base_confidence = (cluster.entry_ids.len() as f64 / 10.0).min(0.7);
            let diversity_boost = 1.0 + 0.15 * (cluster.source_diversity() as f64 - 1.0);
            let alignment_factor = 0.5 + 0.5 * alignment;
            let confidence = (base_confidence * diversity_boost * alignment_factor).min(1.0);

            if confidence < MIN_CONFIDENCE {
                continue;
            }

            // Check if we already have a position in any of the cluster's tickers
            let has_position = cluster.tickers.iter().any(|t| active_tickers.contains(t));

            let direction = cluster.dominant_direction();
            let direction_label = match &direction {
                Some(Direction::Bullish) => "bullish",
                Some(Direction::Bearish) => "bearish",
                _ => "mixed",
            };

            // Deterministic ticker selection (sorted)
            let sorted_tickers = cluster.sorted_tickers();
            let primary_ticker = sorted_tickers.first().cloned();

            let mut action_parts = Vec::new();
            if has_position {
                // Intersection is guaranteed non-empty when has_position is true
                let overlap_ticker = cluster
                    .tickers
                    .intersection(&active_tickers)
                    .next()
                    .expect("has_position guarantees non-empty intersection");
                action_parts.push(format!("⚠️ Already have position in {}", overlap_ticker));
            }
            action_parts.push(format!(
                "Investigate '{}' — {} signals from {} sources",
                cluster.topic,
                direction_label,
                cluster.source_diversity()
            ));

            let score = Opportunity::compute_score(confidence, None, None);

            let source_list = {
                let mut s: Vec<&str> =
                    cluster.source_types.iter().map(|s| s.as_str()).collect();
                s.sort();
                s.join(", ")
            };

            let sample_titles: Vec<&str> =
                cluster.titles.iter().take(3).map(|s| s.as_str()).collect();

            opportunities.push(Opportunity {
                strategy: self.name().to_string(),
                signal_type: "cross_intel_convergence".to_string(),
                title: format!(
                    "Convergence: '{}' — {} entries, {} sources, {} alignment",
                    cluster.topic,
                    cluster.entry_ids.len(),
                    cluster.source_diversity(),
                    direction_label,
                ),
                description: format!(
                    "{} entries from [{}] converge on '{}' (alignment: {:.0}%). Sample: {}",
                    cluster.entry_ids.len(),
                    source_list,
                    cluster.topic,
                    alignment * 100.0,
                    sample_titles.join("; "),
                ),
                confidence,
                edge_cents: None,
                market_ticker: primary_ticker,
                suggested_direction: direction,
                suggested_action: Some(action_parts.join(". ")),
                supporting_entries: cluster.entry_ids.clone(),
                score,
                liquidity: None,
                detected_at: now,
            });
        }

        Ok(opportunities)
    }
}
