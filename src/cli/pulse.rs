//! CLI leaf for `openintel pulse` — orchestrates the pulse and renders it.
//! Returns strings; `main.rs` prints (stdout discipline).

use chrono::{DateTime, Utc};

use crate::adapters::sources::x::XPulseSource;
use crate::application::pulse::X_COST_PER_READ_USD;
use crate::application::DISCLAIMER;
use crate::cli::args::{FormatArg, PulseArgs};
use crate::config::secrets::Credentials;
use crate::domain::entities::pulse::PulseReport;
use crate::domain::error::DomainError;

pub fn not_configured_text() -> String {
    "x is not configured — the pulse needs an X API bearer token (paid API).\n\
     Run:  openintel setup x"
        .to_string()
}

pub async fn run(args: &PulseArgs, credentials: &Credentials) -> Result<String, DomainError> {
    let bearer = credentials
        .x_bearer
        .clone()
        .ok_or_else(|| DomainError::SourceFailure {
            name: "x".into(),
            message: "not configured".into(),
        })?;
    let feed = XPulseSource::new(bearer)?;
    let report = crate::application::pulse::pulse(
        &args.ticker,
        &args.accounts,
        args.hours,
        args.limit,
        &feed,
        Utc::now(),
    )
    .await?;
    Ok(match args.format {
        FormatArg::Table => render_table(&report, Utc::now()),
        FormatArg::Json => render_json(&report)?,
    })
}

fn render_json(report: &PulseReport) -> Result<String, DomainError> {
    #[derive(serde::Serialize)]
    struct Out<'a> {
        report: &'a PulseReport,
        disclaimer: &'static str,
    }
    serde_json::to_string_pretty(&Out {
        report,
        disclaimer: DISCLAIMER,
    })
    .map_err(|e| DomainError::SourceFailure {
        name: "x".into(),
        message: format!("render failed: {e}"),
    })
}

/// "3h ago" / "45m ago" / "2d ago"
fn age(now: DateTime<Utc>, created_at: DateTime<Utc>) -> String {
    let mins = (now - created_at).num_minutes().max(0);
    if mins < 60 {
        format!("{mins}m ago")
    } else if mins < 48 * 60 {
        format!("{}h ago", mins / 60)
    } else {
        format!("{}d ago", mins / (24 * 60))
    }
}

fn render_table(report: &PulseReport, now: DateTime<Utc>) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "=== OpenIntel X Pulse — {} ===", report.ticker);
    let _ = writeln!(
        out,
        "window: last {}h · accounts: {}",
        report.hours_back,
        report.accounts.join(", ")
    );
    let _ = writeln!(
        out,
        "generated: {}\n",
        report
            .generated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    );

    if report.posts.is_empty() {
        let _ = writeln!(out, "⚡ no posts from these accounts in the window");
    } else {
        let _ = writeln!(out, "⚡ {} post(s)\n", report.posts.len());
        for p in &report.posts {
            let _ = writeln!(
                out,
                "  [{}] @{} (eng {})",
                age(now, p.created_at),
                p.author,
                p.engagement
            );
            let _ = writeln!(out, "    {}\n", p.text.as_str());
        }
    }

    let _ = writeln!(
        out,
        "cost: {} posts read (≈ ${:.2} at ${}/read; X dedupes re-reads for 24h)",
        report.posts_read, report.estimated_cost_usd, X_COST_PER_READ_USD
    );
    if report.posts_read as usize > report.posts.len() {
        let _ = writeln!(
            out,
            "note: X returned {} post(s) (billed); {} shown after limit/filtering",
            report.posts_read,
            report.posts.len()
        );
    }
    let _ = writeln!(out, "\n{DISCLAIMER}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::pulse::PulsePost;
    use crate::domain::entities::social_post::PostText;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 16, 12, 0, 0).unwrap()
    }

    fn report(posts: Vec<PulsePost>) -> PulseReport {
        let posts_read = posts.len() as u32;
        PulseReport {
            ticker: "NVDA".into(),
            accounts: vec!["jensenhuang".into(), "elonmusk".into()],
            hours_back: 24,
            posts,
            posts_read,
            estimated_cost_usd: f64::from(posts_read) * X_COST_PER_READ_USD,
            generated_at: at(),
        }
    }

    fn post(hours_old: i64) -> PulsePost {
        PulsePost {
            id: "1".into(),
            author: "jensenhuang".into(),
            text: PostText::parse("Blackwell Ultra shipping at scale").unwrap(),
            created_at: at() - chrono::Duration::hours(hours_old),
            engagement: 48210,
        }
    }

    #[test]
    fn table_renders_posts_cost_and_disclaimer() {
        let rendered = render_table(&report(vec![post(3)]), at());
        assert!(rendered.contains("=== OpenIntel X Pulse — NVDA ==="));
        assert!(rendered.contains("window: last 24h · accounts: jensenhuang, elonmusk"));
        assert!(rendered.contains("[3h ago] @jensenhuang (eng 48210)"));
        assert!(rendered.contains("Blackwell Ultra shipping at scale"));
        assert!(rendered.contains("cost: 1 posts read (≈ $0.01 at $0.005/read"));
        assert!(rendered.contains("Not financial advice"));
        assert!(!rendered.contains("billed")); // read == shown -> no note
    }

    #[test]
    fn table_zero_posts_is_quiet_not_error() {
        let rendered = render_table(&report(vec![]), at());
        assert!(rendered.contains("no posts from these accounts in the window"));
        assert!(rendered.contains("cost: 0 posts read"));
        assert!(!rendered.contains("billed"));
    }

    #[test]
    fn table_notes_when_billed_reads_exceed_shown_posts() {
        let mut r = report(vec![post(1), post(2)]);
        r.posts_read = 10;
        r.estimated_cost_usd = 10.0 * X_COST_PER_READ_USD;
        let rendered = render_table(&r, at());
        assert!(rendered
            .contains("note: X returned 10 post(s) (billed); 2 shown after limit/filtering"));
    }

    #[test]
    fn age_buckets() {
        assert_eq!(age(at(), at() - chrono::Duration::minutes(45)), "45m ago");
        assert_eq!(age(at(), at() - chrono::Duration::hours(3)), "3h ago");
        assert_eq!(age(at(), at() - chrono::Duration::days(3)), "3d ago");
        assert_eq!(age(at(), at() + chrono::Duration::minutes(5)), "0m ago"); // clock skew clamps
    }

    #[test]
    fn json_contains_report_and_disclaimer() {
        let json = render_json(&report(vec![post(1)])).unwrap();
        assert!(json.contains("\"posts_read\": 1"));
        assert!(json.contains("\"estimated_cost_usd\""));
        assert!(json.contains("Not financial advice"));
    }
}
