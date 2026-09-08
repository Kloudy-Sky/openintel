//! CLI leaf for `openintel option` — returns rendered Strings; main prints.

use std::fmt::Write as _;

use chrono::Utc;

use crate::adapters::market::yahoo::YahooMarketSource;
use crate::application::option::{option_frame, FRAMING};
use crate::application::DISCLAIMER;
use crate::cli::args::{FormatArg, OptionFrameArgs, OptionKindArg};
use crate::domain::error::DomainError;
use crate::domain::option::{OptionFrame, OptionSpec};
use crate::domain::trade_journal::OptionKind;

pub async fn run(args: &OptionFrameArgs) -> Result<String, DomainError> {
    let expiry = args
        .expiry
        .parse()
        .map_err(|_| DomainError::SourceFailure {
            name: "option".into(),
            message: format!("expiry {:?} is not YYYY-MM-DD", args.expiry),
        })?;
    let spec = OptionSpec {
        underlying: args.underlying.clone(),
        kind: match args.kind {
            OptionKindArg::Call => OptionKind::Call,
            OptionKindArg::Put => OptionKind::Put,
        },
        strike: args.strike,
        expiry,
        premium: args.premium,
        budget_usd: args.budget,
    };
    let yahoo = YahooMarketSource::new()?;
    let frame = option_frame(spec, &yahoo, &yahoo, Utc::now()).await?;
    Ok(match args.format {
        FormatArg::Table => render_table(&frame),
        FormatArg::Json => render_json(&frame)?,
    })
}

fn render_json(frame: &OptionFrame) -> Result<String, DomainError> {
    #[derive(serde::Serialize)]
    struct Out<'a> {
        frame: &'a OptionFrame,
        framing: &'static str,
        disclaimer: &'static str,
    }
    serde_json::to_string_pretty(&Out {
        frame,
        framing: FRAMING,
        disclaimer: DISCLAIMER,
    })
    .map_err(|e| DomainError::SourceFailure {
        name: "option".into(),
        message: format!("render failed: {e}"),
    })
}

fn opt(v: Option<f64>, f: impl Fn(f64) -> String) -> String {
    v.map(f).unwrap_or_else(|| "n/a".into())
}

fn render_table(f: &OptionFrame) -> String {
    let mut out = String::new();
    let kind = match f.kind {
        OptionKind::Call => "call",
        OptionKind::Put => "put",
    };
    let _ = writeln!(
        out,
        "=== OpenIntel Option Frame — {} {} {} exp {} ===",
        f.underlying, f.strike, kind, f.expiry
    );
    let _ = writeln!(
        out,
        "generated: {}",
        f.generated_at.format("%Y-%m-%dT%H:%M:%SZ")
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  premium:        {:>10.2}   (${:.2} per contract)",
        f.premium,
        f.premium * f.contract_multiplier
    );
    let _ = writeln!(
        out,
        "  size:           {:>10} contracts   (cost ${:.2}, budget ${:.2})",
        f.contracts, f.cost_usd, f.budget_usd
    );
    let _ = writeln!(
        out,
        "  max loss:       {:>10.2}   (the premium, paid up front)",
        f.cost_usd
    );
    let _ = writeln!(out, "  breakeven:      {:>10.2}", f.breakeven);
    let _ = writeln!(
        out,
        "  spot:           {:>10}   move to breakeven {}",
        opt(f.spot, |s| format!("{s:.2}")),
        opt(f.move_to_breakeven_pct, |m| format!("{m:+.1}%"))
    );
    let _ = writeln!(
        out,
        "  expiry:         {:>10} calendar days · {} sessions",
        f.calendar_days_to_expiry,
        f.trading_days_to_expiry
            .map(|n| n.to_string())
            .unwrap_or_else(|| "n/a".into())
    );
    let _ = writeln!(
        out,
        "  ATR(14):        {:>10}   range through expiry {} · breakeven at {} ranges",
        opt(f.atr, |a| format!("{a:.2}")),
        opt(f.atr_scaled_range, |r| format!("{r:.2}")),
        opt(f.breakeven_in_atr_ranges, |n| format!("{n:.2}"))
    );
    let _ = writeln!(
        out,
        "  realized vol:   {:>10}   IV rank {}",
        opt(f.realized_vol, |v| format!("{:.0}%", v * 100.0)),
        opt(f.iv_rank, |r| format!("{r:.2}"))
    );
    for note in &f.notes {
        let _ = writeln!(out, "\n  note: {note}");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "{FRAMING}");
    let _ = writeln!(out);
    let _ = writeln!(out, "{DISCLAIMER}");
    out
}
