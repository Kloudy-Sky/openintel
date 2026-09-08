//! CLI leaf for `openintel clock` — returns rendered Strings; main prints.

use chrono::Utc;

use crate::cli::args::{ClockArgs, FormatArg};
use crate::domain::clock::{market_clock_for, MarketClock};
use crate::domain::error::DomainError;

pub fn run(args: &ClockArgs) -> Result<String, DomainError> {
    let clock = market_clock_for(Utc::now(), args.asset.class());
    Ok(match args.format {
        FormatArg::Table => render_table(&clock),
        FormatArg::Json => {
            serde_json::to_string_pretty(&clock).map_err(|e| DomainError::SourceFailure {
                name: "clock".into(),
                message: format!("render failed: {e}"),
            })?
        }
    })
}

fn render_table(c: &MarketClock) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== OpenIntel Market Clock ({}) ===",
        c.asset_class.as_str()
    );
    let _ = writeln!(out, "now (ET):    {}  ({})", c.now_et, c.weekday);
    let _ = writeln!(out, "now (UTC):   {}", c.now_utc.to_rfc3339());
    let _ = write!(out, "market:      {}", c.state.as_str());
    if let Some(reason) = &c.reason {
        let _ = write!(out, " ({reason})");
    }
    let _ = writeln!(out);
    if let Some(d) = c.prior_close_date {
        let _ = writeln!(out, "last close:  {d}  (newest daily bar)");
    }
    if let Some(d) = c.next_open_date {
        let _ = writeln!(out, "next open:   {d}");
    }
    let _ = writeln!(out, "calendar:    {}", c.calendar_coverage);
    if let Some(note) = &c.note {
        let _ = writeln!(out, "note:        {note}");
    }
    out
}
