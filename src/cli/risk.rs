//! CLI leaf for `openintel risk` — returns rendered Strings; main prints.

use chrono::Utc;

use crate::adapters::market::yahoo::YahooMarketSource;
use crate::application::DISCLAIMER;
use crate::cli::args::{DirectionArg, FormatArg, RiskArgs};
use crate::domain::error::DomainError;
use crate::domain::risk::{Direction, RiskFrame};

const CALCULATOR_LINE: &str =
    "risk_frame is a calculator, not advice — it never recommends taking a trade.";

pub async fn run(args: &RiskArgs) -> Result<String, DomainError> {
    let direction = match args.direction {
        DirectionArg::Long => Direction::Long,
        DirectionArg::Short => Direction::Short,
    };
    let bars = YahooMarketSource::new()?;
    let frame = crate::application::risk::risk_frame(
        &args.ticker,
        direction,
        args.budget,
        Some(args.stop_mult),
        args.entry,
        &bars,
        Utc::now(),
    )
    .await?;
    Ok(match args.format {
        FormatArg::Table => render_table(&frame),
        FormatArg::Json => render_json(&frame)?,
    })
}

fn render_json(frame: &RiskFrame) -> Result<String, DomainError> {
    #[derive(serde::Serialize)]
    struct Out<'a> {
        frame: &'a RiskFrame,
        framing: &'static str,
        disclaimer: &'static str,
    }
    serde_json::to_string_pretty(&Out {
        frame,
        framing: CALCULATOR_LINE,
        disclaimer: DISCLAIMER,
    })
    .map_err(|e| DomainError::SourceFailure {
        name: "risk".into(),
        message: format!("render failed: {e}"),
    })
}

fn render_table(f: &RiskFrame) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== OpenIntel Risk Frame — {} ({:?}) ===",
        f.ticker, f.direction
    );
    let _ = writeln!(
        out,
        "generated: {} · bars: {} · ATR(14): {:.2}\n",
        f.generated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        f.bars_used,
        f.atr
    );
    let _ = writeln!(out, "  entry:          {:>10.2}", f.entry);
    let _ = writeln!(
        out,
        "  stop:           {:>10.2}   ({}×ATR = {:.2}/share)",
        f.stop, f.stop_multiple, f.risk_per_share
    );
    let _ = writeln!(
        out,
        "  size:           {:>10} shares   (notional ${:.2})",
        f.shares, f.notional_usd
    );
    let _ = writeln!(
        out,
        "  max loss:       {:>10.2}   (budget ${:.2})",
        f.max_loss_usd, f.budget_usd
    );
    let _ = writeln!(
        out,
        "  1R / 2R / 3R:   {:.2} / {:.2} / {:.2}",
        f.targets[0], f.targets[1], f.targets[2]
    );
    if let Some(note) = &f.note {
        let _ = writeln!(out, "\n  note: {note}");
    }
    let _ = writeln!(out, "\n{CALCULATOR_LINE}");
    let _ = writeln!(out, "\n{DISCLAIMER}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn frame() -> RiskFrame {
        RiskFrame {
            ticker: "NVDA".into(),
            direction: Direction::Long,
            entry: 106.0,
            atr: 4.0,
            stop_multiple: 2.0,
            stop: 98.0,
            risk_per_share: 8.0,
            shares: 25,
            max_loss_usd: 200.0,
            budget_usd: 200.0,
            targets: [114.0, 122.0, 130.0],
            notional_usd: 2650.0,
            bars_used: 16,
            note: None,
            generated_at: chrono::Utc.with_ymd_and_hms(2026, 7, 16, 0, 0, 0).unwrap(),
        }
    }

    #[test]
    fn table_shows_all_numbers_and_framing() {
        let t = render_table(&frame());
        assert!(t.contains("=== OpenIntel Risk Frame — NVDA (Long) ==="));
        assert!(t.contains("98.00"));
        assert!(t.contains("25 shares"));
        assert!(t.contains("200.00"));
        assert!(t.contains("114.00 / 122.00 / 130.00"));
        assert!(t.contains("calculator, not advice"));
        assert!(t.contains("Not financial advice"));
        assert!(!t.contains("note:"));
    }

    #[test]
    fn table_shows_zero_share_note() {
        let mut f = frame();
        f.shares = 0;
        f.max_loss_usd = 0.0;
        f.note = Some("budget too small for one share at this stop distance".into());
        assert!(render_table(&f).contains("note: budget too small"));
    }

    #[test]
    fn json_has_frame_framing_disclaimer() {
        let j = render_json(&frame()).unwrap();
        assert!(j.contains("\"shares\": 25"));
        assert!(j.contains("calculator, not advice"));
        assert!(j.contains("Not financial advice"));
    }
}
