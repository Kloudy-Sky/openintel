use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::error::DomainError;
use crate::domain::ports::intel_repository::{IntelRepository, QueryFilter};
use chrono::Utc;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

pub struct AlertsUseCase {
    repo: Arc<dyn IntelRepository>,
}

#[derive(Debug, Serialize)]
pub struct AlertScan {
    pub scanned_at: chrono::DateTime<Utc>,
    pub window_hours: u32,
    pub total_entries: usize,
    pub alerts: Vec<Alert>,
}

#[derive(Debug, Serialize)]
pub struct Alert {
    pub severity: AlertSeverity,
    pub kind: String,
    pub title: String,
    pub detail: String,
    /// Related entry IDs
    pub entry_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

impl AlertsUseCase {
    pub fn new(repo: Arc<dyn IntelRepository>) -> Self {
        Self { repo }
    }

    pub fn scan(&self, window_hours: u32) -> Result<AlertScan, DomainError> {
        let now = Utc::now();
        let since = now - chrono::Duration::hours(window_hours as i64);

        let entries = self.repo.query(&QueryFilter {
            category: None,
            tag: None,
            since: Some(since),
            limit: None,
        })?;

        let total_entries = entries.len();
        let mut alerts = Vec::new();

        // 1. Ticker/tag concentration: same tag appears N+ times
        self.detect_tag_concentration(&entries, &mut alerts);

        // 2. Volume spike: category has unusually many entries
        self.detect_volume_spike(&entries, window_hours, &mut alerts);

        // 3. Actionable cluster: multiple high-confidence actionable items
        self.detect_actionable_cluster(&entries, &mut alerts);

        // Sort by severity (critical first)
        alerts.sort_by_key(|a| match a.severity {
            AlertSeverity::Critical => 0,
            AlertSeverity::Warning => 1,
            AlertSeverity::Info => 2,
        });

        Ok(AlertScan {
            scanned_at: now,
            window_hours,
            total_entries,
            alerts,
        })
    }

    /// Detect when a tag appears 3+ times in the window (potential trending signal).
    fn detect_tag_concentration(&self, entries: &[IntelEntry], alerts: &mut Vec<Alert>) {
        let mut tag_entries: HashMap<String, Vec<&IntelEntry>> = HashMap::new();
        for entry in entries {
            for tag in &entry.tags {
                let key = tag.to_lowercase();
                tag_entries.entry(key).or_default().push(entry);
            }
        }

        for (tag, entries) in &tag_entries {
            let count = entries.len();
            let (severity, threshold) = if count >= 10 {
                (AlertSeverity::Critical, 10)
            } else if count >= 5 {
                (AlertSeverity::Warning, 5)
            } else if count >= 3 {
                (AlertSeverity::Info, 3)
            } else {
                continue;
            };

            let titles: Vec<String> = entries.iter().take(5).map(|e| e.title.clone()).collect();
            let entry_ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();

            alerts.push(Alert {
                severity,
                kind: "tag_concentration".into(),
                title: format!(
                    "Tag '{}' mentioned {} times (threshold: {})",
                    tag, count, threshold
                ),
                detail: format!("Related entries: {}", titles.join("; ")),
                entry_ids,
            });
        }
    }

    /// Detect volume spikes: more than expected entries per category.
    /// Baseline: 2 entries per category per 24h window is "normal".
    fn detect_volume_spike(
        &self,
        entries: &[IntelEntry],
        window_hours: u32,
        alerts: &mut Vec<Alert>,
    ) {
        let mut cat_entries: HashMap<String, Vec<&IntelEntry>> = HashMap::new();
        for entry in entries {
            cat_entries
                .entry(entry.category.to_string())
                .or_default()
                .push(entry);
        }

        // Scale baseline by window size (2 per 24h)
        let baseline = (2.0 * window_hours as f64 / 24.0).max(1.0);
        let spike_threshold = (baseline * 3.0) as usize; // 3x normal = spike

        for (category, cat_entries) in &cat_entries {
            let count = cat_entries.len();
            if count >= spike_threshold {
                let severity = if count >= spike_threshold * 2 {
                    AlertSeverity::Critical
                } else {
                    AlertSeverity::Warning
                };

                let entry_ids: Vec<String> = cat_entries.iter().map(|e| e.id.clone()).collect();
                alerts.push(Alert {
                    severity,
                    kind: "volume_spike".into(),
                    title: format!(
                        "Volume spike in '{}': {} entries ({}x baseline)",
                        category,
                        count,
                        count as f64 / baseline
                    ),
                    detail: format!(
                        "Expected ~{:.0} entries in {}h window",
                        baseline, window_hours
                    ),
                    entry_ids,
                });
            }
        }
    }

    /// Detect clusters of high-confidence actionable items.
    fn detect_actionable_cluster(&self, entries: &[IntelEntry], alerts: &mut Vec<Alert>) {
        let actionable: Vec<&IntelEntry> = entries
            .iter()
            .filter(|e| e.actionable && e.confidence.value() >= 0.7)
            .collect();

        if actionable.len() >= 3 {
            let severity = if actionable.len() >= 5 {
                AlertSeverity::Critical
            } else {
                AlertSeverity::Warning
            };

            let titles: Vec<String> = actionable
                .iter()
                .take(5)
                .map(|e| format!("{} ({:.0}%)", e.title, e.confidence.value() * 100.0))
                .collect();
            let entry_ids: Vec<String> = actionable.iter().map(|e| e.id.clone()).collect();

            alerts.push(Alert {
                severity,
                kind: "actionable_cluster".into(),
                title: format!(
                    "{} high-confidence actionable items detected",
                    actionable.len()
                ),
                detail: format!("Items: {}", titles.join("; ")),
                entry_ids,
            });
        }
    }
}
