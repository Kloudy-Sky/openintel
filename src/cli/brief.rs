//! CLI leaf for `openintel brief` — returns rendered Strings; main prints.

use std::fmt::Write as _;

use chrono::Utc;

use crate::adapters::calendar::nasdaq::NasdaqCalendar;
use crate::adapters::filings::edgar::EdgarSource;
use crate::adapters::market::yahoo::YahooMarketSource;
use crate::application::brief::{brief, BriefDeps, BriefReport, BriefRequest, FRAMING};
use crate::application::DISCLAIMER;
use crate::cli::args::{BriefArgs, FormatArg};
use crate::domain::dip::GateStatus;
use crate::domain::entities::ticker::Ticker;
use crate::domain::error::DomainError;
use crate::domain::values::earnings_row::EarningsRow;

pub async fn run(args: &BriefArgs) -> Result<String, DomainError> {
    let calendar = NasdaqCalendar::new()?;
    let yahoo = YahooMarketSource::new()?;
    let edgar = EdgarSource::new()?;
    let deps = BriefDeps {
        earnings: &calendar,
        news: &yahoo,
        filings: &edgar,
    };
    let req = BriefRequest {
        tickers: args
            .tickers
            .iter()
            .map(|t| Ticker::parse(t))
            .collect::<Result<Vec<_>, _>>()?,
        ..BriefRequest::default()
    };
    let report = brief(&req, &deps, Utc::now()).await?;
    Ok(match args.format {
        FormatArg::Table => render_table(&report),
        FormatArg::Json => render_json(&report)?,
    })
}

fn render_json(report: &BriefReport) -> Result<String, DomainError> {
    #[derive(serde::Serialize)]
    struct Out<'a> {
        report: &'a BriefReport,
        framing: &'static str,
        disclaimer: &'static str,
    }
    serde_json::to_string_pretty(&Out {
        report,
        framing: FRAMING,
        disclaimer: DISCLAIMER,
    })
    .map_err(|e| DomainError::SourceFailure {
        name: "brief".into(),
        message: format!("render failed: {e}"),
    })
}

fn gate(status: &GateStatus) -> String {
    match status {
        GateStatus::Pass => "pass".into(),
        GateStatus::Fail(why) => format!("fail ({why})"),
        GateStatus::Unknown(why) => format!("unknown ({why})"),
    }
}

fn cap(row: &EarningsRow) -> String {
    match row.market_cap_usd {
        Some(c) if c >= 1e9 => format!("${:.1}B", c / 1e9),
        Some(c) => format!("${:.0}M", c / 1e6),
        None => "cap n/a".into(),
    }
}

fn earnings_line(out: &mut String, label: &str, rows: &[EarningsRow]) {
    if rows.is_empty() {
        return;
    }
    let _ = writeln!(out, "earnings {label} ({}):", rows.len());
    for r in rows {
        let eps = r
            .eps_forecast
            .map(|e| format!(" · est {e:.2}"))
            .unwrap_or_default();
        let _ = writeln!(out, "  {:<6} {:>8}  {}{eps}", r.symbol, cap(r), r.company);
    }
}

fn render_table(r: &BriefReport) -> String {
    let mut out = String::new();
    let c = &r.clock;
    let _ = write!(
        out,
        "=== OpenIntel Brief — {} ({}) · market {}",
        c.date_et,
        c.weekday,
        c.state.as_str()
    );
    if let Some(reason) = &c.reason {
        let _ = write!(out, " ({reason})");
    }
    let _ = writeln!(out, " ===");
    let _ = writeln!(
        out,
        "now {} · last close {} · next open {} · overnight = since {}",
        c.now_et,
        c.prior_close_date
            .map(|d| d.to_string())
            .unwrap_or_else(|| "unknown".into()),
        c.next_open_date
            .map(|d| d.to_string())
            .unwrap_or_else(|| "unknown".into()),
        r.since.to_rfc3339()
    );

    let _ = writeln!(out);
    if r.macro_releases.is_empty() {
        let _ = writeln!(out, "macro: nothing scheduled");
    } else {
        let _ = writeln!(out, "macro ({}):", r.macro_releases.len());
        for m in &r.macro_releases {
            let _ = writeln!(out, "  {} ET  {}  [{}]", m.time_et, m.release, m.agency);
        }
    }

    let _ = writeln!(out);
    match &r.earnings {
        None => {
            let _ = writeln!(out, "earnings: unavailable (see errors)");
        }
        Some(e) => {
            earnings_line(&mut out, "before open", &e.before_open);
            earnings_line(&mut out, "after close", &e.after_close);
            earnings_line(&mut out, "time unspecified", &e.unspecified);
            let _ = writeln!(
                out,
                "earnings not listed: {} below the cap floor · {} with no cap given",
                e.below_floor, e.cap_unknown
            );
        }
    }

    if !r.tickers.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "tickers:");
        for t in &r.tickers {
            let _ = writeln!(
                out,
                "  {}  filings: {} · headlines: {} · {} since close · {} undated",
                t.ticker,
                gate(&t.filings),
                gate(&t.headlines),
                t.headlines_since_close.len(),
                t.undated_headlines
            );
            for ev in &t.evidence {
                let _ = writeln!(out, "    evidence: {ev}");
            }
            for h in &t.headlines_since_close {
                let _ = writeln!(
                    out,
                    "    {}  {}: {}",
                    h.published_at.format("%m-%d %H:%MZ"),
                    h.publisher,
                    h.title
                );
            }
        }
    }

    if !r.chatter.is_empty() {
        let _ = writeln!(out);
        for s in &r.chatter {
            let mentions: Vec<String> = s
                .top_mentions
                .iter()
                .map(|m| format!("{} {}", m.ticker, m.mentions))
                .collect();
            let _ = writeln!(
                out,
                "chatter {} {} ({} posts): {}",
                s.platform,
                s.date,
                s.total_posts,
                if mentions.is_empty() {
                    "no cashtags".to_string()
                } else {
                    mentions.join(" · ")
                }
            );
        }
    }

    if !r.errors.is_empty() {
        let _ = writeln!(out);
        for e in &r.errors {
            let _ = writeln!(out, "error: {e}");
        }
    }
    if !r.notes.is_empty() {
        let _ = writeln!(out);
        for n in &r.notes {
            let _ = writeln!(out, "note: {n}");
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "{FRAMING}");
    let _ = writeln!(out, "{DISCLAIMER}");
    out
}
