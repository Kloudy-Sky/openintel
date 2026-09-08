//! CLI leaf for `openintel journal` — the manual fallback for the trade
//! journal (the MCP tools are the primary surface). Returns rendered Strings;
//! main prints.

use chrono::Utc;

use crate::application::trade_journal::{
    default_path_or_err, log_trade, open_positions, review_trades, update_trade, LogTradeRequest,
    PositionsReport, TradeReviewReport, TradeUpdate,
};
use crate::application::DISCLAIMER;
use crate::cli::args::{FormatArg, JournalArgs, JournalCloseReasonArg, JournalCommand};
use crate::domain::error::DomainError;
use crate::domain::trade_journal::{CloseReason, Instrument, OptionKind};
use crate::domain::trade_review::Bucket;

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "trade_journal".into(),
        message: message.into(),
    }
}

pub async fn run(args: &JournalArgs) -> Result<String, DomainError> {
    let path = default_path_or_err()?;
    match &args.command {
        JournalCommand::Log(log) => {
            let instrument = match (&log.option, log.crypto, log.forex) {
                (Some(_), true, _) | (Some(_), _, true) | (None, true, true) => {
                    return Err(fail("--option, --crypto, and --forex are exclusive"))
                }
                (None, true, false) => Instrument::Crypto {
                    symbol: log.symbol.to_ascii_uppercase(),
                },
                (None, false, true) => Instrument::Forex {
                    pair: log.symbol.to_ascii_uppercase(),
                },
                (None, false, false) => Instrument::Equity {
                    ticker: log.symbol.to_ascii_uppercase(),
                },
                (Some(kind), false, false) => Instrument::Option {
                    underlying: log.symbol.to_ascii_uppercase(),
                    strike: log.strike.ok_or_else(|| fail("--option needs --strike"))?,
                    expiry: log
                        .expiry
                        .as_deref()
                        .and_then(|e| e.parse().ok())
                        .ok_or_else(|| fail("--option needs --expiry YYYY-MM-DD"))?,
                    kind: match kind {
                        crate::cli::args::OptionKindArg::Call => OptionKind::Call,
                        crate::cli::args::OptionKindArg::Put => OptionKind::Put,
                    },
                },
            };
            let outcome = log_trade(
                &path,
                LogTradeRequest {
                    source: "cli".into(),
                    instrument,
                    qty: log.qty,
                    entry: log.entry,
                    thesis: log.thesis.clone(),
                    setup_tag: log.tag.clone(),
                    planned_stop: log.stop,
                    planned_target: log.target,
                    risk_usd: log.risk_usd,
                    wallet_usd: log.wallet,
                    allow_duplicate: log.allow_duplicate,
                },
                Utc::now(),
            )?;
            Ok(if outcome.logged {
                format!(
                    "logged {} — thesis and plan are now frozen",
                    outcome.trade_id
                )
            } else {
                format!(
                    "not logged: same-day duplicate of {} (use --allow-duplicate if this is \
                     really a second trade)",
                    outcome.trade_id
                )
            })
        }
        JournalCommand::Amend(amend) => {
            update_trade(
                &path,
                &amend.trade_id,
                TradeUpdate::Amend {
                    note: amend.note.clone(),
                    new_stop: amend.stop,
                    new_target: amend.target,
                },
                Utc::now(),
            )?;
            Ok(format!("amended {}", amend.trade_id))
        }
        JournalCommand::Close(close) => {
            update_trade(
                &path,
                &close.trade_id,
                TradeUpdate::Close {
                    exit: close.exit,
                    reason: match close.reason {
                        JournalCloseReasonArg::Stop => CloseReason::Stop,
                        JournalCloseReasonArg::Target => CloseReason::Target,
                        JournalCloseReasonArg::Discretion => CloseReason::Discretion,
                        JournalCloseReasonArg::Expiry => CloseReason::Expiry,
                    },
                    note: close.note.clone(),
                },
                Utc::now(),
            )?;
            Ok(format!("closed {}", close.trade_id))
        }
        JournalCommand::Positions { format } => {
            let report = open_positions(&path);
            Ok(match format {
                FormatArg::Json => serde_json::to_string_pretty(&report)
                    .map_err(|e| fail(format!("render failed: {e}")))?,
                FormatArg::Table => render_positions(&report),
            })
        }
        JournalCommand::Review { format } => {
            let bars = crate::adapters::market::yahoo::YahooMarketSource::new()?;
            let report = review_trades(&path, &bars, Utc::now()).await?;
            Ok(match format {
                FormatArg::Json => serde_json::to_string_pretty(&report)
                    .map_err(|e| fail(format!("render failed: {e}")))?,
                FormatArg::Table => render_review(&report),
            })
        }
    }
}

fn render_positions(report: &PositionsReport) -> String {
    use std::fmt::Write as _;
    let open = &report.open;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== OpenIntel Trade Journal — {} open, {} closed ===\n",
        open.len(),
        report.closed_count
    );
    if open.is_empty() {
        let _ = writeln!(out, "  no open trades");
    }
    for t in open {
        let _ = writeln!(
            out,
            "  {}  {}  qty {} @ {:.2}  stop {:.2}{}  [{}]",
            t.id,
            t.instrument.describe(),
            t.qty,
            t.entry,
            t.current_stop,
            t.current_target
                .map(|x| format!("  target {x:.2}"))
                .unwrap_or_default(),
            t.setup_tag,
        );
        let _ = writeln!(out, "    thesis: {}", t.thesis);
        if t.stop_widened {
            let _ = writeln!(out, "    ⚠ stop widened from {:.2}", t.original_stop);
        }
        for a in &t.amendments {
            let _ = writeln!(out, "    amended {}: {}", a.at.date_naive(), a.note);
        }
    }
    if report.skipped_lines > 0 {
        let _ = writeln!(
            out,
            "\n⚠ {} unparseable journal line(s) skipped — the counts above may be incomplete",
            report.skipped_lines
        );
    }
    for e in &report.errors {
        let _ = writeln!(out, "⚠ {e}");
    }
    let _ = writeln!(out, "\n{DISCLAIMER}");
    out
}

fn render_bucket_rows(out: &mut String, title: &str, buckets: &[Bucket]) {
    use std::fmt::Write as _;
    let _ = writeln!(out, "{title}");
    for b in buckets {
        match &b.note {
            Some(note) => {
                let _ = writeln!(out, "  {:<24} n={:<3} {}", b.key, b.trades, note);
            }
            None => {
                let _ = writeln!(
                    out,
                    "  {:<24} n={:<3} closed={:<3} win {}% · avg {}R · stops widened {}",
                    b.key,
                    b.trades,
                    b.closed,
                    b.win_rate_pct
                        .map(|w| format!("{w:.0}"))
                        .unwrap_or_default(),
                    b.avg_realized_r
                        .map(|r| format!("{r:+.2}"))
                        .unwrap_or_default(),
                    b.stops_widened,
                );
            }
        }
    }
}

fn render_review(r: &TradeReviewReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== OpenIntel Trade Review — {} trades ({} closed, {} open) ===\n",
        r.trades_total, r.closed, r.open
    );
    render_bucket_rows(&mut out, "by setup:", &r.by_setup);
    let _ = writeln!(out);
    render_bucket_rows(&mut out, "by instrument:", &r.by_instrument);
    let _ = writeln!(out, "\ntrades:");
    for g in &r.trades {
        let status = if g.open {
            "open".to_string()
        } else {
            format!(
                "{}R{}",
                g.realized_r.map(|x| format!("{x:+.1}")).unwrap_or_default(),
                g.close_reason
                    .map(|c| format!(" ({c:?})").to_lowercase())
                    .unwrap_or_default()
            )
        };
        let _ = writeln!(out, "  {}  {}  {}", g.id, g.instrument, status);
        for d in &g.discipline {
            let _ = writeln!(out, "    ⚠ {d}");
        }
    }
    for n in &r.notes {
        let _ = writeln!(out, "\nnote: {n}");
    }
    for e in &r.errors {
        let _ = writeln!(out, "error: {e}");
    }
    let _ = writeln!(out, "\n{DISCLAIMER}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::trade_journal::{fold, open_event, RiskSnapshot, Trade, TradeEvent};
    use crate::domain::trade_review::{aggregate, grade};
    use chrono::TimeZone;

    fn trades() -> Vec<Trade> {
        let events = vec![
            open_event(
                "NVDA-20260821-1".into(),
                Utc.with_ymd_and_hms(2026, 8, 21, 14, 0, 0).unwrap(),
                "cli".into(),
                Instrument::Equity {
                    ticker: "NVDA".into(),
                },
                10.0,
                100.0,
                "weekly support at 98".into(),
                "sr-support-bounce".into(),
                95.0,
                Some(110.0),
                Some(RiskSnapshot::new(50.0, 5000.0).unwrap()),
            )
            .unwrap(),
            TradeEvent::Amended {
                trade_id: "NVDA-20260821-1".into(),
                at: Utc.with_ymd_and_hms(2026, 8, 21, 16, 0, 0).unwrap(),
                note: "giving it room".into(),
                new_stop: Some(93.0),
                new_target: None,
            },
        ];
        fold(&events).trades
    }

    #[test]
    fn positions_table_shows_thesis_and_widened_stop() {
        let t = render_positions(&PositionsReport {
            journal_path: "/tmp/x".into(),
            open: trades(),
            closed_count: 3,
            skipped_lines: 2,
            errors: vec!["close for unknown trade GHOST-1".into()],
        });
        assert!(t.contains("2 unparseable journal line(s) skipped"));
        assert!(t.contains("close for unknown trade GHOST-1"));
        assert!(t.contains("1 open, 3 closed"));
        assert!(t.contains("thesis: weekly support at 98"));
        assert!(t.contains("stop widened from 95.00"));
        assert!(t.contains("Not financial advice"));
    }

    #[test]
    fn review_table_shows_buckets_and_discipline() {
        let all = trades();
        let graded: Vec<_> = all.iter().map(|t| grade(t, None, None)).collect();
        let (by_setup, by_instrument) = aggregate(&graded);
        let report = TradeReviewReport {
            generated_at: Utc.with_ymd_and_hms(2026, 8, 21, 21, 0, 0).unwrap(),
            journal_path: "/tmp/x".into(),
            trades_total: graded.len(),
            closed: 0,
            open: graded.len(),
            by_setup,
            by_instrument,
            trades: graded,
            skipped_lines: 0,
            errors: vec![],
            notes: vec!["only 0 closed trades — anecdote".into()],
        };
        let t = render_review(&report);
        assert!(t.contains("sr-support-bounce"));
        assert!(t.contains("insufficient sample"));
        assert!(t.contains("stop was widened after entry"));
        assert!(t.contains("anecdote"));
    }
}
