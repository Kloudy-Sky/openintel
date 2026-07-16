//! Pulse posts: catalyst events from specific high-impact X accounts.
//! Deliberately NOT `SocialPost` — pulse posts never enter the fusion
//! engine's sentiment averaging (see the 2026-07-16 spec).

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::entities::social_post::PostText;

#[derive(Debug, Clone, Serialize)]
pub struct PulsePost {
    pub id: String,
    pub author: String,
    pub text: PostText,
    pub created_at: DateTime<Utc>,
    pub engagement: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PulseReport {
    pub ticker: String,
    pub accounts: Vec<String>,
    pub hours_back: u32,
    pub posts: Vec<PulsePost>,
    pub posts_read: u32,
    pub estimated_cost_usd: f64,
    pub generated_at: DateTime<Utc>,
}
