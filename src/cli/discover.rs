//! CLI leaf for `openintel discover` — returns rendered Strings; main prints.

use std::fmt::Write as _;

use chrono::Utc;

use crate::adapters::filings::edgar::EdgarSource;
use crate::adapters::market::yahoo::YahooMarketSource;
use crate::application::chatter::{chatter, ChatterReport, ChatterRequest, DEFAULT_FREE_LIMIT};
use crate::application::discover::{
    discover, Candidate, DiscoverDeps, DiscoverReport, DiscoverRequest, FRAMING,
};
use crate::application::DISCLAIMER;
use crate::cli::args::{DiscoverArgs, FormatArg, ScreenArg};
use crate::config::secrets::Credentials;
use crate::domain::dip::GateStatus;
use crate::domain::error::DomainError;

pub async fn run(args: &DiscoverArgs, credentials: &Credentials) -> Result<String, DomainError> {
    let yahoo = YahooMarketSource::new()?;
    if args.chatter {
        return run_chatter(args, credentials, &yahoo).await;
    }
    let edgar = EdgarSource::new()?;
    let social = crate::adapters::sources::build_social_sources(credentials);
    let deps = DiscoverDeps {
        movers: &yahoo,
        bars: &yahoo,
        news: &yahoo,
        filings: &edgar,
        social: &social,
    };
    let mut req = DiscoverRequest {
        count: args.count,
        deep: args.deep,
        ..DiscoverRequest::default()
    };
    if !args.screens.is_empty() {
        req.screens = args.screens.iter().map(|s| s.kind()).collect();
    }
    let report = discover(&req, &deps, Utc::now()).await?;
    Ok(match args.format {
        FormatArg::Table => render_table(&report),
        FormatArg::Json => render_json(&report)?,
    })
}

impl ScreenArg {
    pub fn kind(&self) -> crate::domain::values::mover::ScreenKind {
        use crate::domain::values::mover::ScreenKind;
        match self {
            ScreenArg::Gainers => ScreenKind::DayGainers,
            ScreenArg::Losers => ScreenKind::DayLosers,
            ScreenArg::Actives => ScreenKind::MostActives,
        }
    }
}

async fn run_chatter(
    args: &DiscoverArgs,
    credentials: &Credentials,
    yahoo: &YahooMarketSource,
) -> Result<String, DomainError> {
    let listening_path =
        crate::config::listening::default_path().ok_or_else(|| DomainError::SourceFailure {
            name: "chatter".into(),
            message: "cannot resolve a home directory".into(),
        })?;
    let (listening, created) = crate::config::listening::load_or_seed(&listening_path)?;

    let free = crate::adapters::sources::build_free_listening_feeds(credentials);
    let x_feed = if args.x {
        match credentials.x_bearer.clone() {
            Some(bearer) => Some(crate::adapters::sources::x::XPulseSource::new(bearer)?),
            None => {
                return Err(DomainError::SourceFailure {
                    name: "chatter".into(),
                    message: "--x needs X credentials — run `openintel setup x`".into(),
                })
            }
        }
    } else {
        None
    };
    let mut feeds: Vec<&dyn crate::domain::ports::listening_feed::ListeningFeed> =
        free.iter().map(|f| f.as_ref()).collect();
    if let Some(x) = &x_feed {
        feeds.push(x);
    }

    let req = ChatterRequest {
        listening,
        hours: args.hours.clamp(1, 167),
        free_limit: DEFAULT_FREE_LIMIT,
        x_read_cap: args.x_limit.clamp(1, 100),
        baseline_path: crate::application::chatter::default_baseline_path(),
    };
    let report = chatter(&req, &feeds, Some(yahoo), Utc::now()).await?;
    let mut out = match args.format {
        FormatArg::Table => render_chatter_table(&report),
        FormatArg::Json => render_chatter_json(&report)?,
    };
    if created && args.format == FormatArg::Table {
        out.push_str(&format!(
            "
seeded {} with the macro defaults — edit it to curate your listening set
",
            listening_path.display()
        ));
    }
    Ok(out)
}

fn render_chatter_json(report: &ChatterReport) -> Result<String, DomainError> {
    #[derive(serde::Serialize)]
    struct Out<'a> {
        report: &'a ChatterReport,
        framing: &'static str,
        disclaimer: &'static str,
    }
    serde_json::to_string_pretty(&Out {
        report,
        framing: crate::application::chatter::FRAMING,
        disclaimer: DISCLAIMER,
    })
    .map_err(|e| DomainError::SourceFailure {
        name: "chatter".into(),
        message: format!("render failed: {e}"),
    })
}

fn render_chatter_table(r: &ChatterReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== OpenIntel Discover — chatter ({}h window) ===
",
        r.hours
    );
    for p in &r.platforms {
        let cost = p
            .estimated_cost_usd
            .map(|c| format!(" · cost ≈ ${c:.2}"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "[{}] {} posts scanned ({} returned){cost}",
            p.platform, p.posts_scanned, p.posts_returned
        );
        for t in &p.tickers {
            let ratio = t
                .velocity
                .ratio
                .map(|x| format!("{x:.1}x baseline"))
                .unwrap_or_else(|| "no velocity claim".into());
            let tape = match (t.change_pct, t.rvol) {
                (Some(c), Some(v)) => format!(" · day {c:+.1}% rvol {v:.1}"),
                _ => String::new(),
            };
            let btc = match t.before_the_chart {
                Some(true) => "  ← chatter leading the chart",
                _ => "",
            };
            let _ = writeln!(
                out,
                "  {}  {} mentions ({:.0}% of posts) · {ratio}{tape}{btc}",
                t.ticker,
                t.velocity.mentions,
                t.velocity.share * 100.0
            );
            for f in &t.velocity.flags {
                let _ = writeln!(out, "    note: {f}");
            }
        }
        for n in &p.notes {
            let _ = writeln!(out, "  note: {n}");
        }
        if !p.recent_posts.is_empty() {
            let _ = writeln!(out, "  recent posts:");
            for post in &p.recent_posts {
                let text: String = post.text.as_str().chars().take(120).collect();
                let _ = writeln!(out, "    @{}: {}", post.author, text);
            }
        }
        let _ = writeln!(out);
    }
    for e in &r.errors {
        let _ = writeln!(out, "error: {e}");
    }
    for n in &r.notes {
        let _ = writeln!(out, "note: {n}");
    }
    let _ = writeln!(
        out,
        "
{}",
        crate::application::chatter::FRAMING
    );
    let _ = writeln!(
        out,
        "
{DISCLAIMER}"
    );
    out
}

fn render_json(report: &DiscoverReport) -> Result<String, DomainError> {
    #[derive(serde::Serialize)]
    struct Out<'a> {
        report: &'a DiscoverReport,
        framing: &'static str,
        disclaimer: &'static str,
    }
    serde_json::to_string_pretty(&Out {
        report,
        framing: FRAMING,
        disclaimer: DISCLAIMER,
    })
    .map_err(|e| DomainError::SourceFailure {
        name: "discover".into(),
        message: format!("render failed: {e}"),
    })
}

fn gate_char(status: &GateStatus) -> &'static str {
    match status {
        GateStatus::Pass => "clear",
        GateStatus::Fail(_) => "CONFIRMED",
        GateStatus::Unknown(_) => "unknown",
    }
}

fn render_candidate(out: &mut String, c: &Candidate) {
    let screens: Vec<&str> = c.screens.iter().map(|s| s.label()).collect();
    let _ = writeln!(
        out,
        "  {}  {:+.1}%  @ {:.2}  [{}]",
        c.ticker,
        c.change_pct,
        c.price,
        screens.join(", ")
    );
    let mut tape: Vec<String> = Vec::new();
    if let Some(rvol) = c.rvol {
        tape.push(format!("rvol {rvol:.1}"));
    }
    if let Some(s) = c.stretch_atr {
        tape.push(format!("stretch {s:+.1} ATR"));
    }
    if let Some(r) = c.rsi14 {
        tape.push(format!("rsi {r:.0}"));
    }
    if !tape.is_empty() {
        let _ = writeln!(out, "    tape: {}", tape.join(" · "));
    }
    for (label, e) in [("3mo", &c.extremes_3mo), ("~1y", &c.extremes_1y)] {
        if let Some(e) = e {
            let _ = writeln!(
                out,
                "    {label} extremes ({}d): high {:.2} ({:.1} ATR above) · low {:.2} ({:.1} ATR below)",
                e.days, e.high, e.dist_to_high_atr, e.low, e.dist_to_low_atr
            );
        }
    }
    if let Some(cat) = &c.catalyst {
        let _ = writeln!(
            out,
            "    catalyst: filings {} · headlines {}",
            gate_char(&cat.filings),
            gate_char(&cat.headlines)
        );
        for e in &cat.evidence {
            let _ = writeln!(out, "      {e}");
        }
    }
    if let Some(s) = &c.sentiment {
        let _ = writeln!(
            out,
            "    attention: {} mentions · net sentiment {:+.2}",
            s.mentions, s.net_sentiment
        );
    }
    for n in &c.notes {
        let _ = writeln!(out, "    note: {n}");
    }
}

fn render_table(report: &DiscoverReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "=== OpenIntel Discover — movers ===");
    let screens: Vec<String> = report
        .screens
        .iter()
        .map(|s| {
            format!(
                "{} {} (floor -{}, annotated {})",
                s.universe,
                s.screen.label(),
                s.floor_rejects,
                s.annotated
            )
        })
        .collect();
    let _ = writeln!(out, "{}\n", screens.join(" · "));
    if report.candidates.is_empty() {
        let _ = writeln!(out, "  no candidates survived the quality floor");
    }
    for c in &report.candidates {
        render_candidate(&mut out, c);
        let _ = writeln!(out);
    }
    for e in &report.errors {
        let _ = writeln!(out, "error: {e}");
    }
    for n in &report.notes {
        let _ = writeln!(out, "note: {n}");
    }
    let _ = writeln!(out, "\n{FRAMING}");
    let _ = writeln!(out, "\n{DISCLAIMER}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::discover::{CatalystCheck, ScreenSummary};
    use crate::domain::discover::PeriodExtremes;
    use crate::domain::values::mover::ScreenKind;
    use chrono::TimeZone;

    fn report() -> DiscoverReport {
        DiscoverReport {
            generated_at: Utc.with_ymd_and_hms(2026, 8, 21, 21, 0, 0).unwrap(),
            screens: vec![ScreenSummary {
                screen: ScreenKind::DayLosers,
                universe: 25,
                floor_rejects: 20,
                annotated: 1,
            }],
            candidates: vec![Candidate {
                ticker: "DOWN".into(),
                screens: vec![ScreenKind::DayLosers],
                change_pct: -9.0,
                price: 50.0,
                rvol: Some(2.0),
                stretch_atr: Some(1.4),
                rsi14: Some(31.0),
                extremes_3mo: Some(PeriodExtremes {
                    days: 63,
                    high: 70.0,
                    low: 48.0,
                    dist_to_high_atr: 10.0,
                    dist_to_low_atr: 1.0,
                }),
                extremes_1y: None,
                catalyst: Some(CatalystCheck {
                    filings: GateStatus::Pass,
                    headlines: GateStatus::Unknown("news unavailable".into()),
                    evidence: vec![],
                }),
                sentiment: None,
                notes: vec![
                    "long-range extremes cover only 63 trading days, not a full year".into(),
                ],
            }],
            errors: vec![],
            notes: vec![
                "losers shown here carry no verdict — run dip_scan for the gated dip-setup grading"
                    .into(),
            ],
        }
    }

    #[test]
    fn table_shows_evidence_and_framing() {
        let t = render_table(&report());
        assert!(t.contains("DOWN  -9.0%"));
        assert!(t.contains("rvol 2.0"));
        assert!(t.contains("low 48.00 (1.0 ATR below)"));
        assert!(t.contains("filings clear · headlines unknown"));
        assert!(t.contains("run dip_scan"));
        assert!(t.contains("never ranks"));
        assert!(t.contains("Not financial advice"));
    }

    #[test]
    fn json_carries_framing_and_disclaimer() {
        let j = render_json(&report()).unwrap();
        assert!(j.contains("\"ticker\": \"DOWN\""));
        assert!(j.contains("never ranks"));
        assert!(j.contains("Not financial advice"));
    }
}
