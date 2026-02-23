use crate::domain::values::category::Category;
use chrono::{DateTime, Utc};

/// Half-life in hours for each category. After this many hours,
/// an entry's effective confidence drops to 50% of its original value.
pub fn half_life_hours(category: &Category) -> f64 {
    match category {
        // Fast-decaying: market signals are stale within days
        Category::Market => 72.0,       // 3 days
        Category::Crypto => 48.0,       // 2 days
        Category::Trading => 120.0,     // 5 days
        Category::Catalyst => 96.0,     // 4 days
        Category::Economic => 24.0,     // 1 day (events pass quickly)
        Category::Signal => 72.0,       // 3 days

        // Medium-decaying: news and social signals
        Category::Newsletter => 168.0,  // 7 days
        Category::Social => 120.0,      // 5 days
        Category::Sentiment => 120.0,   // 5 days
        Category::Macro => 336.0,       // 14 days
        Category::Weather => 24.0,      // 1 day

        // Slow-decaying: strategic intel stays relevant longer
        Category::Competitor => 720.0,  // 30 days
        Category::Opportunity => 336.0, // 14 days
        Category::General => 336.0,     // 14 days
        Category::Research => 720.0,    // 30 days

        // Portfolio/position tracking
        Category::Portfolio => 720.0,   // 30 days
        Category::Position => 168.0,    // 7 days
        Category::Trade => 168.0,       // 7 days

        // System
        Category::Heartbeat => 24.0,    // 1 day
    }
}

/// Calculate decayed confidence using exponential decay.
/// Returns `confidence * 0.5^(age_hours / half_life_hours)`.
/// Minimum floor of 0.01 to avoid zero-confidence entries.
pub fn decayed_confidence(
    confidence: f64,
    category: &Category,
    created_at: &DateTime<Utc>,
) -> f64 {
    let now = Utc::now();
    let age_hours = (now - *created_at).num_minutes() as f64 / 60.0;
    if age_hours <= 0.0 {
        return confidence;
    }
    let half_life = half_life_hours(category);
    let decay_factor = 0.5_f64.powf(age_hours / half_life);
    (confidence * decay_factor).max(0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_decay_for_fresh_entry() {
        let now = Utc::now();
        let result = decayed_confidence(0.8, &Category::Market, &now);
        assert!((result - 0.8).abs() < 0.01);
    }

    #[test]
    fn test_half_decay_at_half_life() {
        let half_life = half_life_hours(&Category::Market); // 72 hours
        let created = Utc::now() - chrono::Duration::hours(half_life as i64);
        let result = decayed_confidence(1.0, &Category::Market, &created);
        assert!((result - 0.5).abs() < 0.05);
    }

    #[test]
    fn test_minimum_floor() {
        let created = Utc::now() - chrono::Duration::days(365);
        let result = decayed_confidence(1.0, &Category::Market, &created);
        assert!(result >= 0.01);
    }

    #[test]
    fn test_competitor_decays_slower_than_market() {
        let created = Utc::now() - chrono::Duration::days(7);
        let market = decayed_confidence(1.0, &Category::Market, &created);
        let competitor = decayed_confidence(1.0, &Category::Competitor, &created);
        assert!(competitor > market);
    }
}
