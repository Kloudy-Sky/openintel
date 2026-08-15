//! CLI leaf for `openintel dip` — returns rendered Strings; main prints.

use std::fmt::Write as _;

use chrono::Utc;

use crate::adapters::filings::edgar::EdgarSource;
use crate::adapters::market::yahoo::YahooMarketSource;
use crate::application::dip::{
    default_journal_path, dip_check, dip_scan, DipDeps, DipScanReport, DipScanRequest,
    TickerDipReport, FRAMING,
};
use crate::application::DISCLAIMER;
use crate::cli::args::{DipArgs, FormatArg};
use crate::config::secrets::Credentials;
use crate::domain::dip::{DipSignal, DropBand, GateStatus, QualityFloor};
use crate::domain::error::DomainError;
use crate::domain::margin::MarginInputs;

const FRAMING_LINE: &str = FRAMING;

fn request_from(args: &DipArgs) -> DipScanRequest {
    DipScanRequest {
        count: args.count,
        deep_n: args.deep,
        band: DropBand {
            min_pct: args.band_min,
            max_pct: args.band_max,
        },
        floor: QualityFloor {
            min_price: args.min_price,
            min_market_cap: args.min_cap,
            min_avg_volume: args.min_volume,
            min_listed_days: args.min_listed_days,
        },
        score_min: args.score_min,
        margin: args.equity.map(|equity_usd| MarginInputs {
            equity_usd,
            leverage: args.leverage,
            maintenance: args.maintenance,
            risk_pct: args.risk_pct,
            intraday_bp: args.intraday_bp,
        }),
        journal_path: (!args.no_journal).then(default_journal_path).flatten(),
        ..DipScanRequest::default()
    }
}

pub async fn run(args: &DipArgs, credentials: &Credentials) -> Result<String, DomainError> {
    let yahoo = YahooMarketSource::new()?;
    let edgar = EdgarSource::new()?;
    let social = crate::adapters::sources::build_social_sources(credentials);
    let deps = DipDeps {
        bars: &yahoo,
        news: &yahoo,
        filings: &edgar,
        social: &social,
        market: Some(&yahoo),
    };
    let req = request_from(args);
    let now = Utc::now();

    match &args.ticker {
        Some(ticker) => {
            let report = dip_check(ticker, &req, &deps, None, now).await?;
            Ok(match args.format {
                FormatArg::Table => render_ticker_table(&report),
                FormatArg::Json => render_json(&report)?,
            })
        }
        None => {
            let report = dip_scan(&req, &yahoo, &deps, now).await?;
            Ok(match args.format {
                FormatArg::Table => render_report_table(&report),
                FormatArg::Json => render_json(&report)?,
            })
        }
    }
}

fn render_json<T: serde::Serialize>(payload: &T) -> Result<String, DomainError> {
    #[derive(serde::Serialize)]
    struct Out<'a, T: serde::Serialize> {
        report: &'a T,
        framing: &'static str,
        disclaimer: &'static str,
    }
    serde_json::to_string_pretty(&Out {
        report: payload,
        framing: FRAMING_LINE,
        disclaimer: DISCLAIMER,
    })
    .map_err(|e| DomainError::SourceFailure {
        name: "dip".into(),
        message: format!("render failed: {e}"),
    })
}

fn gate_mark(g: &GateStatus) -> &'static str {
    match g {
        GateStatus::Pass => "✓",
        GateStatus::Fail(_) => "✗",
        GateStatus::Unknown(_) => "?",
    }
}

fn gates_line(signal: &DipSignal) -> String {
    let g = &signal.gates;
    format!(
        "floor {} · band {} · filing {} · headline {} · index {} · close {} · score {}",
        gate_mark(&g.quality_floor),
        gate_mark(&g.drop_band),
        gate_mark(&g.no_filing),
        gate_mark(&g.no_catalyst_headline),
        gate_mark(&g.idiosyncratic),
        gate_mark(&g.close_strength),
        gate_mark(&g.score_floor),
    )
}

fn opt(v: Option<f64>, fmt: &dyn Fn(f64) -> String) -> String {
    v.map_or_else(|| "n/a".to_string(), fmt)
}

fn write_signal_block(out: &mut String, signal: &DipSignal) {
    let m = &signal.metrics;
    let _ = writeln!(
        out,
        "  stretch {:.1} ATR · RSI {} · vol {} · close-loc {} · vs index {} · down {}d",
        m.stretch_atr,
        opt(m.rsi14, &|v| format!("{v:.0}")),
        opt(m.volume_ratio, &|v| format!("{v:.1}x")),
        opt(m.close_location, &|v| format!("{v:.2}")),
        opt(m.excess_vs_spx_pct, &|v| format!("{v:+.1}%")),
        m.down_days,
    );
    let _ = writeln!(out, "  gates: {}", gates_line(signal));
    for e in &signal.catalyst_evidence {
        let _ = writeln!(out, "  evidence: {e}");
    }
    for n in &signal.notes {
        let _ = writeln!(out, "  note: {n}");
    }
}

fn write_sizing(
    out: &mut String,
    risk: &Option<crate::domain::risk::RiskFrame>,
    margin: &Option<crate::domain::margin::MarginFrame>,
) {
    if let Some(risk) = risk {
        let _ = writeln!(
            out,
            "  size: {} sh @ {:.2} · stop {:.2} · max loss ${:.2}",
            risk.shares, risk.entry, risk.stop, risk.max_loss_usd
        );
    }
    if let Some(m) = margin {
        let _ = writeln!(
            out,
            "  margin ({}x): {} sh · borrowed ${:.2} · m-call {} ({} away)",
            m.leverage,
            m.shares,
            m.borrowed_usd,
            m.margin_call_price
                .map_or_else(|| "n/a".into(), |p| format!("{p:.2}")),
            m.mc_distance_pct
                .map_or_else(|| "n/a".to_string(), |p| format!("{p:.0}%")),
        );
        for n in &m.notes {
            let _ = writeln!(out, "  margin note: {n}");
        }
    }
}

fn render_report_table(r: &DipScanReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== OpenIntel Dip Scan — {} ({:?}) ===",
        r.generated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        r.session
    );
    let _ = writeln!(
        out,
        "universe: {} losers · floor rejects: {} · band-excluded: {} · {} {}\n",
        r.universe_size,
        r.floor_rejects.len(),
        r.band_excluded,
        r.spx_proxy,
        opt(r.spx_change_pct, &|v| format!("{v:+.1}%")),
    );

    if r.candidates.is_empty() {
        let _ = writeln!(out, "no setups today — that's a result, not an error");
    }
    for (i, c) in r.candidates.iter().enumerate() {
        let _ = writeln!(
            out,
            "{}. {:?}  {}  {:+.1}%  (score {:.0}/{:.0})",
            i + 1,
            c.signal.verdict,
            c.ticker,
            c.change_pct,
            c.signal.score,
            c.signal.score_max,
        );
        write_signal_block(&mut out, &c.signal);
        write_sizing(&mut out, &c.risk, &c.margin);
        let _ = writeln!(out);
    }

    for e in &r.errors {
        let _ = writeln!(out, "error: {} — {}", e.ticker, e.error);
    }
    for n in &r.notes {
        let _ = writeln!(out, "note: {n}");
    }
    let _ = writeln!(out, "\n{FRAMING_LINE}");
    let _ = writeln!(out, "\n{DISCLAIMER}");
    out
}

fn render_ticker_table(report: &TickerDipReport) -> String {
    let signal = &report.signal;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== OpenIntel Dip Signal — {} ({:?}) ===",
        signal.ticker, signal.session
    );
    let _ = writeln!(
        out,
        "verdict: {:?} · score {:.0}/{:.0}\n",
        signal.verdict, signal.score, signal.score_max
    );
    write_signal_block(&mut out, signal);
    write_sizing(&mut out, &report.risk, &report.margin);
    for n in &report.notes {
        let _ = writeln!(out, "  note: {n}");
    }
    let _ = writeln!(out, "\n{FRAMING_LINE}");
    let _ = writeln!(out, "\n{DISCLAIMER}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::dip::DipCandidate;
    use crate::domain::dip::{DipMetrics, GateResults, ScoreComponents, Session, Verdict};
    use chrono::TimeZone;

    fn signal(verdict: Verdict) -> DipSignal {
        DipSignal {
            ticker: "GOOD".into(),
            verdict,
            session: Session::PostClose,
            score: 78.0,
            score_max: 100.0,
            components: ScoreComponents {
                stretch: 20.0,
                close_location: Some(18.0),
                idiosyncrasy: 10.0,
                quiet_volume: 15.0,
                rsi: 5.0,
                multi_day: 5.0,
                divergence: 5.0,
            },
            metrics: DipMetrics {
                sma20: 99.75,
                atr: 2.0,
                rsi14: Some(28.0),
                stretch_atr: 2.1,
                volume_ratio: Some(0.8),
                close_location: Some(0.88),
                down_days: 3,
                excess_vs_spx_pct: Some(-7.6),
            },
            gates: GateResults {
                quality_floor: GateStatus::Pass,
                drop_band: GateStatus::Pass,
                no_filing: GateStatus::Pass,
                no_catalyst_headline: GateStatus::Pass,
                idiosyncratic: GateStatus::Pass,
                close_strength: GateStatus::Pass,
                score_floor: GateStatus::Pass,
            },
            catalyst_evidence: vec![],
            notes: vec![],
        }
    }

    fn report(candidates: Vec<DipCandidate>) -> DipScanReport {
        DipScanReport {
            generated_at: Utc.with_ymd_and_hms(2026, 8, 14, 21, 30, 0).unwrap(),
            session: Session::PostClose,
            spx_proxy: "SPY",
            spx_change_pct: Some(-0.4),
            universe_size: 100,
            band_excluded: 12,
            floor_rejects: vec![],
            candidates,
            errors: vec![],
            notes: vec![],
        }
    }

    #[test]
    fn empty_scan_renders_no_setups_line() {
        let t = render_report_table(&report(vec![]));
        assert!(t.contains("no setups today — that's a result, not an error"));
        assert!(t.contains("grades setup conformance"));
        assert!(t.contains("Not financial advice"));
    }

    #[test]
    fn candidate_block_shows_gates_and_metrics() {
        let c = DipCandidate {
            ticker: "GOOD".into(),
            change_pct: -8.0,
            price: 87.4,
            signal: signal(Verdict::HighConfidence),
            risk: None,
            margin: None,
        };
        let t = render_report_table(&report(vec![c]));
        assert!(t.contains("HighConfidence"));
        assert!(t.contains("score 78/100"));
        assert!(t.contains("stretch 2.1 ATR"));
        assert!(t.contains("floor ✓"));
        assert!(!t.contains("size:")); // no equity given
    }

    #[test]
    fn single_signal_table_renders_with_optional_sizing() {
        let report = TickerDipReport {
            signal: signal(Verdict::Watch),
            risk: None,
            margin: None,
            notes: vec![],
        };
        let t = render_ticker_table(&report);
        assert!(t.contains("Dip Signal — GOOD"));
        assert!(t.contains("verdict: Watch"));
        assert!(!t.contains("size:"));

        let report = TickerDipReport {
            signal: signal(Verdict::Watch),
            risk: Some(crate::domain::risk::RiskFrame {
                ticker: "GOOD".into(),
                direction: crate::domain::risk::Direction::Long,
                entry: 87.4,
                atr: 2.0,
                stop_multiple: 2.0,
                stop: 83.4,
                risk_per_share: 4.0,
                shares: 62,
                max_loss_usd: 248.0,
                budget_usd: 250.0,
                targets: [91.4, 95.4, 99.4],
                notional_usd: 5418.8,
                bars_used: 31,
                note: None,
                generated_at: Utc.with_ymd_and_hms(2026, 8, 14, 21, 30, 0).unwrap(),
            }),
            margin: None,
            notes: vec![],
        };
        let t = render_ticker_table(&report);
        assert!(t.contains("size: 62 sh @ 87.40"));
    }

    #[test]
    fn request_maps_args_and_journal_opt_out() {
        use clap::Parser;
        let cli = crate::cli::args::Cli::try_parse_from([
            "openintel",
            "dip",
            "--equity",
            "10000",
            "--band-min",
            "-12",
            "--no-journal",
        ])
        .unwrap();
        let crate::cli::args::Command::Dip(args) = cli.command else {
            panic!("expected dip command");
        };
        let req = request_from(&args);
        assert_eq!(req.band.min_pct, -12.0);
        assert!(req.journal_path.is_none());
        let m = req.margin.unwrap();
        assert_eq!(m.equity_usd, 10_000.0);
        assert!(!m.intraday_bp);
    }
}
