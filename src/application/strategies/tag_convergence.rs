//! Tag convergence strategy.
//!
//! Detects when multiple intel entries from different source types converge
//! on the same topic (measured by shared tags) within a time window.
//! Higher source diversity = higher confidence.

use std::collections::{HashMap, HashSet};

use chrono::Utc;

use crate::domain::error::DomainError;
use crate::domain::ports::strategy::{DetectionContext, Opportunity, Strategy};

/// Detects cross-source convergence on shared topics.
pub struct TagConvergenceStrategy;

impl Strategy for TagConvergenceStrategy {
    fn name(&self) -> &'static str {
        "tag_convergence"
    }

    fn detect(&self, ctx: &DetectionContext) -> Result<Vec<Opportunity>, DomainError> {
        // Group entries by tag, tracking source diversity
        let mut tag_clusters: HashMap<String, Vec<(String, String, String, String)>> =
            HashMap::new();

        // Skip overly generic tags
        let skip_tags: HashSet<&str> = [
            "market", "signal", "update", "analysis", "news", "general", "trade",
        ]
        .into_iter()
        .collect();

        for entry in &ctx.entries {
            let source_type = entry.source_type.to_string();
            for tag in &entry.tags {
                let tag_lower = tag.to_lowercase();
                if skip_tags.contains(tag_lower.as_str()) || tag_lower.len() < 2 {
                    continue;
                }
                tag_clusters.entry(tag_lower).or_default().push((
                    entry.id.clone(),
                    entry.title.clone(),
                    source_type.clone(),
                    entry.source.clone().unwrap_or_default(),
                ));
            }
        }

        let mut opportunities = Vec::new();

        for (tag, entries) in &tag_clusters {
            if entries.len() < 3 {
                continue;
            }

            // Count unique source types
            let unique_sources: HashSet<&str> =
                entries.iter().map(|(_, _, st, _)| st.as_str()).collect();

            let source_diversity = unique_sources.len();

            // Need at least 2 different source types for convergence
            if source_diversity < 2 {
                continue;
            }

            // Confidence: base from entry count, boosted by source diversity
            let base_confidence = (entries.len() as f64 / 8.0).min(0.8);
            let diversity_boost = 1.0 + 0.15 * (source_diversity as f64 - 1.0);
            let confidence = (base_confidence * diversity_boost).min(1.0);

            if confidence < 0.35 {
                continue;
            }

            let supporting: Vec<String> = entries.iter().map(|(id, _, _, _)| id.clone()).collect();

            let mut source_list: Vec<&str> = unique_sources.into_iter().collect();
            source_list.sort();

            let score = Opportunity::compute_score(confidence, None, None);

            opportunities.push(Opportunity {
                strategy: self.name().to_string(),
                signal_type: "tag_convergence".to_string(),
                title: format!(
                    "Convergence on '{}' — {} entries from {} source types",
                    tag,
                    entries.len(),
                    source_diversity
                ),
                description: format!(
                    "Tag '{}' appears in {} entries across sources: {}. Multi-source convergence suggests higher signal reliability.",
                    tag,
                    entries.len(),
                    source_list.join(", ")
                ),
                confidence,
                edge_cents: None,
                market_ticker: None,
                suggested_direction: None,
                suggested_action: Some(format!(
                    "Investigate '{}' for tradeable thesis",
                    tag
                )),
                supporting_entries: supporting,
                score,
                liquidity: None,
                market_price: None,
                suggested_size_cents: None,
                detected_at: Utc::now(),
            });
        }

        // No sorting or truncation here — OpportunitiesUseCase handles final ranking and limits.
        Ok(opportunities)
    }
}
