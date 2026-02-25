//! Earnings momentum strategy.
//!
//! Detects when multiple intel entries about the same ticker show strong
//! directional signals from earnings (beats, misses, guidance).

use std::collections::HashMap;

use chrono::Utc;

use crate::domain::error::DomainError;
use crate::domain::ports::strategy::{DetectionContext, Opportunity, Strategy};

/// Detects earnings momentum when multiple signals converge on a ticker.
pub struct EarningsMomentumStrategy;

impl Strategy for EarningsMomentumStrategy {
    fn name(&self) -> &'static str {
        "earnings_momentum"
    }

    fn detect(&self, ctx: &DetectionContext) -> Result<Vec<Opportunity>, DomainError> {
        // Group entries by ticker mentions in tags
        let mut ticker_signals: HashMap<String, Vec<(String, String, String)>> =
            HashMap::new();

        let earnings_keywords = [
            "earnings", "beat", "miss", "guidance", "revenue", "eps", "q1", "q2", "q3", "q4",
        ];

        for entry in &ctx.entries {
            let text_lower = format!("{} {}", entry.title, entry.body).to_lowercase();

            // Check if entry is earnings-related
            let is_earnings = earnings_keywords
                .iter()
                .any(|kw| text_lower.contains(kw));
            if !is_earnings {
                continue;
            }

            // Extract ticker-like tags (uppercase, 1-5 chars)
            for tag in &entry.tags {
                let tag_upper = tag.to_uppercase();
                if tag_upper.len() >= 1
                    && tag_upper.len() <= 5
                    && tag_upper.chars().all(|c| c.is_ascii_alphabetic())
                {
                    ticker_signals
                        .entry(tag_upper)
                        .or_default()
                        .push((entry.title.clone(), entry.body.clone(), entry.id.clone()));
                }
            }
        }

        let mut opportunities = Vec::new();

        for (ticker, signals) in &ticker_signals {
            if signals.len() < 2 {
                continue;
            }

            // Determine direction from signal content
            let mut bullish_count = 0i32;
            let mut bearish_count = 0i32;

            let bullish_words = ["beat", "surge", "jump", "rally", "strong", "raised"];
            let bearish_words = [
                "miss",
                "drop",
                "fell",
                "weak",
                "lowered",
                "disappointing",
                "cut",
            ];

            for (title, body, _) in signals {
                let text = format!("{} {}", title, body).to_lowercase();
                for word in &bullish_words {
                    if text.contains(word) {
                        bullish_count += 1;
                    }
                }
                for word in &bearish_words {
                    if text.contains(word) {
                        bearish_count += 1;
                    }
                }
            }

            let total_sentiment = bullish_count + bearish_count;
            if total_sentiment == 0 {
                continue;
            }

            let direction = if bullish_count > bearish_count {
                "bullish"
            } else {
                "bearish"
            };

            let alignment =
                (bullish_count - bearish_count).unsigned_abs() as f64 / total_sentiment as f64;

            // Confidence based on signal count and alignment
            let base_confidence = (signals.len() as f64 / 5.0).min(1.0);
            let confidence = (base_confidence * (0.5 + 0.5 * alignment)).min(1.0);

            if confidence < 0.3 {
                continue;
            }

            let supporting: Vec<String> = signals.iter().map(|(_, _, id)| id.clone()).collect();

            let score = Opportunity::compute_score(confidence, None);

            opportunities.push(Opportunity {
                strategy: self.name().to_string(),
                signal_type: "earnings_momentum".to_string(),
                title: format!(
                    "{} — {} earnings momentum ({} signals)",
                    ticker,
                    direction,
                    signals.len()
                ),
                description: format!(
                    "{} entries point {} for {} (alignment: {:.0}%)",
                    signals.len(),
                    direction,
                    ticker,
                    alignment * 100.0
                ),
                confidence,
                edge_cents: None,
                market_ticker: Some(ticker.clone()),
                suggested_direction: Some(direction.to_string()),
                suggested_action: None,
                supporting_entries: supporting,
                score,
                detected_at: Utc::now(),
            });
        }

        // Sort by score descending
        opportunities.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        Ok(opportunities)
    }
}
