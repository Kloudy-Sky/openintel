//! dip_scan's pure core: quality floor, oversold metrics, hard gates, and a
//! v0 ranking score for "big down day, no visible catalyst" setups.
//!
//! Gates decide the verdict; the score only ranks survivors. Any gate whose
//! evidence is unavailable fails CLOSED (caps the verdict at `watch`) — an
//! unverifiable candidate is never `high_confidence`. "High confidence" means
//! high conformance to the setup template, never probability of profit.
//! Synchronous, no IO, no clock, no timezone — the application layer stamps
//! session and dates.

use chrono::NaiveDate;
use serde::Serialize;

use crate::domain::error::DomainError;
use crate::domain::risk::{atr, ATR_PERIOD};
use crate::domain::values::bar::Bar;
use crate::domain::values::filing::Filing;
use crate::domain::values::headline::Headline;
use crate::domain::values::mover::MoverRow;

pub const SMA_PERIOD: usize = 20;
pub const RSI_PERIOD: usize = 14;
/// Minimum prior (pre-drop-day) bars for baselines: SMA(20) plus one.
pub const MIN_PRIOR_BARS: usize = 21;

/// Filing-form PREFIXES that mark a real same-day catalyst: material events
/// (8-K incl. 8-K/A, foreign 6-K) and dilution/offering paper (the whole
/// 424B prospectus family, S-3 incl. /A and ASR shelf variants, FWP).
pub const CATALYST_FORM_PREFIXES: &[&str] = &["8-K", "6-K", "424B", "S-3", "FWP"];

/// Amendments and variants count: `8-K/A` is still a material event.
pub fn is_catalyst_form(form: &str) -> bool {
    CATALYST_FORM_PREFIXES.iter().any(|p| form.starts_with(p))
}

/// Whole-word, case-insensitive markers of a fundamental catalyst in
/// headline or social text. A hit is evidence, matched conservatively.
pub const CATALYST_KEYWORDS: &[&str] = &[
    "earnings",
    "miss",
    "guidance",
    "cut",
    "offering",
    "dilution",
    "downgrade",
    "halt",
    "fraud",
    "lawsuit",
    "recall",
    "fda",
    "bankruptcy",
    "delisting",
    "investigation",
    "resign",
];

fn fail(message: impl Into<String>) -> DomainError {
    DomainError::SourceFailure {
        name: "dip".into(),
        message: message.into(),
    }
}

// ---------------------------------------------------------------- quality floor

#[derive(Debug, Clone)]
pub struct QualityFloor {
    pub min_price: f64,
    pub min_market_cap: u64,
    pub min_avg_volume: u64,
    pub min_listed_days: i64,
}

impl Default for QualityFloor {
    fn default() -> Self {
        Self {
            min_price: 5.0,
            min_market_cap: 500_000_000,
            min_avg_volume: 1_000_000,
            min_listed_days: 180,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FloorReject {
    pub symbol: String,
    pub reason: String,
}

/// Split screener rows into floor survivors and rejects (each reject carries
/// its reason). A missing screener field is a reject, never a pass.
pub fn apply_floor(
    rows: &[MoverRow],
    floor: &QualityFloor,
    now_ms: i64,
) -> (Vec<MoverRow>, Vec<FloorReject>) {
    let mut pass = Vec::new();
    let mut rejects = Vec::new();
    for row in rows {
        let reason = floor_reason(row, floor, now_ms);
        match reason {
            None => pass.push(row.clone()),
            Some(reason) => rejects.push(FloorReject {
                symbol: row.symbol.clone(),
                reason,
            }),
        }
    }
    (pass, rejects)
}

fn floor_reason(row: &MoverRow, floor: &QualityFloor, now_ms: i64) -> Option<String> {
    let ex = row.exchange.to_ascii_lowercase();
    if ex.contains("otc") || ex.contains("pink") {
        return Some(format!("off-exchange venue ({})", row.exchange));
    }
    if row.price < floor.min_price {
        return Some(format!("price {:.2} < {:.2}", row.price, floor.min_price));
    }
    match row.market_cap {
        Some(cap) if cap >= floor.min_market_cap => {}
        Some(cap) => return Some(format!("market cap {cap} < {}", floor.min_market_cap)),
        None => return Some("market cap unavailable".into()),
    }
    match row.avg_volume_3mo {
        Some(v) if v >= floor.min_avg_volume => {}
        Some(v) => return Some(format!("avg volume {v} < {}", floor.min_avg_volume)),
        None => return Some("avg volume unavailable".into()),
    }
    match row.first_trade_ms {
        Some(first) if now_ms.saturating_sub(first) >= floor.min_listed_days * 86_400_000 => {}
        Some(_) => return Some(format!("listed < {} days", floor.min_listed_days)),
        None => return Some("listing date unavailable".into()),
    }
    None
}

// ---------------------------------------------------------------- metrics

pub fn sma(closes: &[f64], period: usize) -> Option<f64> {
    if period == 0 || closes.len() < period {
        return None;
    }
    Some(closes[closes.len() - period..].iter().sum::<f64>() / period as f64)
}

/// Cutler's RSI: simple (not Wilder-smoothed) averages of gains and losses
/// over the last `period` deltas — deterministic, no seed dependence.
/// Needs `period + 1` closes. A flat window returns a neutral 50.
pub fn rsi_cutler(closes: &[f64], period: usize) -> Option<f64> {
    if period == 0 || closes.len() < period + 1 {
        return None;
    }
    let window = &closes[closes.len() - (period + 1)..];
    let (mut gains, mut losses) = (0.0_f64, 0.0_f64);
    for w in window.windows(2) {
        let d = w[1] - w[0];
        if d >= 0.0 {
            gains += d;
        } else {
            losses -= d;
        }
    }
    if gains == 0.0 && losses == 0.0 {
        return Some(50.0);
    }
    Some(100.0 * gains / (gains + losses))
}

/// Where the close sits in the day's range: 0 = at the low, 1 = at the high.
/// None on a rangeless bar.
pub fn close_location(bar: &Bar) -> Option<f64> {
    let range = bar.high - bar.low;
    (range > 0.0).then(|| ((bar.close - bar.low) / range).clamp(0.0, 1.0))
}

/// Consecutive down closes counted backward from the last close.
pub fn consecutive_down_closes(closes: &[f64]) -> usize {
    closes.windows(2).rev().take_while(|w| w[1] < w[0]).count()
}

/// Case-insensitive whole-word keyword hits across the given texts, deduped.
pub fn catalyst_hits(texts: &[&str]) -> Vec<String> {
    let mut hits: Vec<String> = Vec::new();
    for text in texts {
        let lower = text.to_ascii_lowercase();
        for token in lower.split(|c: char| !c.is_ascii_alphanumeric()) {
            if CATALYST_KEYWORDS.contains(&token) && !hits.iter().any(|h| h == token) {
                hits.push(token.to_string());
            }
        }
    }
    hits
}

// ---------------------------------------------------------------- score

/// v0 weights — hand-picked and UNVALIDATED. The scan journal exists so these
/// can be graded against forward returns before anyone trusts them.
#[derive(Debug, Clone, Serialize)]
pub struct ScoreWeights {
    pub stretch: f64,
    pub close_location: f64,
    pub idiosyncrasy: f64,
    pub quiet_volume: f64,
    pub rsi: f64,
    pub multi_day: f64,
    pub divergence: f64,
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            stretch: 20.0,
            close_location: 20.0,
            idiosyncrasy: 15.0,
            quiet_volume: 15.0,
            rsi: 10.0,
            multi_day: 10.0,
            divergence: 10.0,
        }
    }
}

/// Weighted points actually awarded per component (already × weight).
#[derive(Debug, Clone, Serialize)]
pub struct ScoreComponents {
    pub stretch: f64,
    /// None intraday — the day bar is not final until the close.
    pub close_location: Option<f64>,
    pub idiosyncrasy: f64,
    pub quiet_volume: f64,
    pub rsi: f64,
    pub multi_day: f64,
    pub divergence: f64,
}

impl ScoreComponents {
    pub fn total(&self) -> f64 {
        self.stretch
            + self.close_location.unwrap_or(0.0)
            + self.idiosyncrasy
            + self.quiet_volume
            + self.rsi
            + self.multi_day
            + self.divergence
    }
}

fn frac_stretch(stretch_atr: f64) -> f64 {
    (stretch_atr / 3.5).clamp(0.0, 1.0)
}

/// Low-volume declines revert; volume-confirmed ones are more often informed
/// moves that continue — so QUIET volume scores, a climax does not.
fn frac_quiet_volume(ratio: f64) -> f64 {
    ((3.0 - ratio) / 2.0).clamp(0.0, 1.0)
}

fn frac_rsi(rsi: f64) -> f64 {
    ((50.0 - rsi) / 30.0).clamp(0.0, 1.0)
}

fn frac_idiosyncrasy(excess_pct: f64) -> f64 {
    ((-excess_pct - 3.0) / 7.0).clamp(0.0, 1.0)
}

fn frac_multi_day(down_days: usize) -> f64 {
    ((down_days as f64 - 1.0) / 3.0).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------- gates

/// Evidence for a gate: available data, or a reason it could not be fetched.
/// Unavailable evidence fails closed (verdict capped at `watch`).
#[derive(Debug, Clone)]
pub enum GateEvidence<T> {
    Available(T),
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", content = "reason", rename_all = "lowercase")]
pub enum GateStatus {
    Pass,
    Fail(String),
    Unknown(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct GateResults {
    pub quality_floor: GateStatus,
    pub drop_band: GateStatus,
    pub no_filing: GateStatus,
    pub no_catalyst_headline: GateStatus,
    pub idiosyncratic: GateStatus,
    pub close_strength: GateStatus,
    pub score_floor: GateStatus,
}

impl GateResults {
    fn all_pass(&self) -> bool {
        [
            &self.quality_floor,
            &self.drop_band,
            &self.no_filing,
            &self.no_catalyst_headline,
            &self.idiosyncratic,
            &self.close_strength,
            &self.score_floor,
        ]
        .iter()
        .all(|g| matches!(g, GateStatus::Pass))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    NoSetup,
    Watch,
    HighConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Session {
    Intraday,
    PostClose,
}

/// The drop-day change band eligible for evaluation, in percent. The
/// catastrophic tail is excluded by design — it is where informed moves live.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DropBand {
    pub min_pct: f64,
    pub max_pct: f64,
}

impl Default for DropBand {
    fn default() -> Self {
        Self {
            min_pct: -15.0,
            max_pct: -4.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SentimentSummary {
    pub net_sentiment: f64,
    pub mentions: usize,
}

// ---------------------------------------------------------------- signal

pub struct DipInputs<'a> {
    pub ticker: &'a str,
    /// Baseline history — MUST exclude the drop-day bar.
    pub prior_bars: &'a [Bar],
    /// The drop day's own bar (final post-close, partial intraday).
    pub drop_day_bar: Option<Bar>,
    pub drop_date: NaiveDate,
    pub last_price: f64,
    /// Drop-day change in percent (negative for a decline).
    pub change_pct: f64,
    pub day_volume: Option<u64>,
    pub avg_volume_3mo: Option<u64>,
    /// Same-day index change in percent (None = gate unknown).
    pub spx_change_pct: Option<f64>,
    /// Recent filings covering drop day minus one calendar day onward.
    pub filings: GateEvidence<Vec<Filing>>,
    /// Headlines already filtered to the drop day by the caller.
    pub headlines: GateEvidence<Vec<Headline>>,
    pub sentiment: Option<SentimentSummary>,
    pub session: Session,
    /// Floor result from the screener row (Unknown in single-ticker mode,
    /// where no screener row exists to evaluate).
    pub floor: GateStatus,
    pub band: DropBand,
    pub weights: &'a ScoreWeights,
    pub score_min: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DipMetrics {
    pub sma20: f64,
    pub atr: f64,
    pub rsi14: Option<f64>,
    /// How far below the 20-day mean the last price sits, in ATR units.
    pub stretch_atr: f64,
    pub volume_ratio: Option<f64>,
    pub close_location: Option<f64>,
    pub down_days: usize,
    pub excess_vs_spx_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DipSignal {
    pub ticker: String,
    pub verdict: Verdict,
    pub session: Session,
    pub score: f64,
    /// 100 post-close; 80 intraday (close-location is unscoreable).
    pub score_max: f64,
    pub components: ScoreComponents,
    pub metrics: DipMetrics,
    pub gates: GateResults,
    pub catalyst_evidence: Vec<String>,
    pub notes: Vec<String>,
}

pub fn dip_signal(inputs: &DipInputs) -> Result<DipSignal, DomainError> {
    if !(inputs.last_price.is_finite() && inputs.last_price > 0.0) {
        return Err(fail("last price must be a positive number"));
    }
    if !inputs.change_pct.is_finite() {
        return Err(fail("change percent must be a number"));
    }
    if inputs.prior_bars.len() < MIN_PRIOR_BARS {
        return Err(fail(format!(
            "not enough history — need {MIN_PRIOR_BARS} prior bars, got {}",
            inputs.prior_bars.len()
        )));
    }
    let closes: Vec<f64> = inputs.prior_bars.iter().map(|b| b.close).collect();
    let sma20 = sma(&closes, SMA_PERIOD).ok_or_else(|| fail("not enough closes for SMA(20)"))?;
    let atr = atr(inputs.prior_bars, ATR_PERIOD)
        .ok_or_else(|| fail(format!("not enough history for ATR({ATR_PERIOD})")))?;
    if !(atr.is_finite() && atr > 0.0) {
        return Err(fail("flat price history — ATR is zero"));
    }

    let mut notes: Vec<String> = Vec::new();
    let mut catalyst_evidence: Vec<String> = Vec::new();

    let stretch_atr = (sma20 - inputs.last_price) / atr;
    let rsi14 = rsi_cutler(&closes, RSI_PERIOD);
    let volume_ratio = match (inputs.day_volume, inputs.avg_volume_3mo) {
        (Some(day), Some(avg)) if avg > 0 => Some(day as f64 / avg as f64),
        _ => None,
    };
    let close_loc = match inputs.session {
        Session::Intraday => None,
        Session::PostClose => inputs.drop_day_bar.as_ref().and_then(close_location),
    };
    let mut all_closes = closes.clone();
    all_closes.push(inputs.last_price);
    let down_days = consecutive_down_closes(&all_closes);
    let excess = inputs.spx_change_pct.map(|s| inputs.change_pct - s);

    // -------- components (each fraction × its weight)
    let w = inputs.weights;
    if volume_ratio.is_none() {
        notes.push("volume unavailable — quiet-volume component scored 0".into());
    }
    let divergence_frac = match inputs.sentiment {
        Some(s) if inputs.change_pct < 0.0 && s.net_sentiment >= 0.0 => {
            (s.mentions as f64 / 20.0).min(1.0)
        }
        Some(_) => 0.0,
        None => {
            notes.push("no social data — divergence component scored 0".into());
            0.0
        }
    };
    let components = ScoreComponents {
        stretch: frac_stretch(stretch_atr) * w.stretch,
        close_location: close_loc.map(|c| c * w.close_location),
        idiosyncrasy: excess.map_or(0.0, frac_idiosyncrasy) * w.idiosyncrasy,
        quiet_volume: volume_ratio.map_or(0.0, frac_quiet_volume) * w.quiet_volume,
        rsi: rsi14.map_or(0.0, frac_rsi) * w.rsi,
        multi_day: frac_multi_day(down_days) * w.multi_day,
        divergence: divergence_frac * w.divergence,
    };
    let score = components.total();
    let score_max = match inputs.session {
        Session::PostClose => {
            w.stretch
                + w.close_location
                + w.idiosyncrasy
                + w.quiet_volume
                + w.rsi
                + w.multi_day
                + w.divergence
        }
        Session::Intraday => {
            notes.push("intraday run — close-location unscored, verdict capped at watch".into());
            w.stretch + w.idiosyncrasy + w.quiet_volume + w.rsi + w.multi_day + w.divergence
        }
    };

    // -------- gates
    let drop_band =
        if inputs.change_pct >= inputs.band.min_pct && inputs.change_pct <= inputs.band.max_pct {
            GateStatus::Pass
        } else {
            GateStatus::Fail(format!(
                "day change {:.1}% outside [{:.0}%, {:.0}%]",
                inputs.change_pct, inputs.band.min_pct, inputs.band.max_pct
            ))
        };

    let since = inputs.drop_date.pred_opt().unwrap_or(inputs.drop_date);
    let no_filing = match &inputs.filings {
        GateEvidence::Unavailable(reason) => GateStatus::Unknown(reason.clone()),
        GateEvidence::Available(filings) => {
            let hits: Vec<&Filing> = filings
                .iter()
                .filter(|f| f.filed_on >= since && is_catalyst_form(&f.form))
                .collect();
            if hits.is_empty() {
                GateStatus::Pass
            } else {
                for f in &hits {
                    catalyst_evidence.push(format!("SEC {} filed {}", f.form, f.filed_on));
                }
                GateStatus::Fail(format!(
                    "{} catalyst filing(s) on/around drop day",
                    hits.len()
                ))
            }
        }
    };

    let no_catalyst_headline = match &inputs.headlines {
        GateEvidence::Unavailable(reason) => GateStatus::Unknown(reason.clone()),
        GateEvidence::Available(headlines) => {
            let titles: Vec<&str> = headlines.iter().map(|h| h.title.as_str()).collect();
            let hits = catalyst_hits(&titles);
            if hits.is_empty() {
                GateStatus::Pass
            } else {
                for h in headlines {
                    let title_hits = catalyst_hits(&[h.title.as_str()]);
                    if !title_hits.is_empty() {
                        catalyst_evidence.push(format!(
                            "headline [{}]: \"{}\" (terms: {})",
                            h.publisher,
                            h.title,
                            title_hits.join(", ")
                        ));
                    }
                }
                GateStatus::Fail(format!(
                    "catalyst term(s) in headlines: {}",
                    hits.join(", ")
                ))
            }
        }
    };

    let idiosyncratic = match excess {
        None => GateStatus::Unknown("index change unavailable".into()),
        Some(e) if e <= -3.0 => GateStatus::Pass,
        Some(e) => GateStatus::Fail(format!(
            "excess vs index {e:.1}% > -3% — market-driven move"
        )),
    };

    let close_strength = match inputs.session {
        Session::Intraday => GateStatus::Unknown("intraday — day bar not final".into()),
        Session::PostClose => match close_loc {
            None => GateStatus::Unknown("day bar range unavailable".into()),
            Some(c) if c >= 0.5 => GateStatus::Pass,
            Some(c) => GateStatus::Fail(format!(
                "closed at {:.0}% of day range — no buyers at the close",
                c * 100.0
            )),
        },
    };

    let score_floor = if score >= inputs.score_min {
        GateStatus::Pass
    } else {
        GateStatus::Fail(format!("score {score:.0} < {:.0}", inputs.score_min))
    };

    let gates = GateResults {
        quality_floor: inputs.floor.clone(),
        drop_band,
        no_filing,
        no_catalyst_headline,
        idiosyncratic,
        close_strength,
        score_floor,
    };

    // -------- verdict: catalysts and eligibility kill; anything short of
    // all-gates-pass post-close is at most `watch`.
    let verdict = if matches!(gates.no_filing, GateStatus::Fail(_))
        || matches!(gates.no_catalyst_headline, GateStatus::Fail(_))
        || matches!(gates.quality_floor, GateStatus::Fail(_))
        || matches!(gates.drop_band, GateStatus::Fail(_))
    {
        Verdict::NoSetup
    } else if gates.all_pass() && inputs.session == Session::PostClose {
        Verdict::HighConfidence
    } else {
        Verdict::Watch
    };

    Ok(DipSignal {
        ticker: inputs.ticker.to_string(),
        verdict,
        session: inputs.session,
        score,
        score_max,
        components,
        metrics: DipMetrics {
            sma20,
            atr,
            rsi14,
            stretch_atr,
            volume_ratio,
            close_location: close_loc,
            down_days,
            excess_vs_spx_pct: excess,
        },
        gates,
        catalyst_evidence,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, day).unwrap()
    }

    fn bar_on(day: u32, close: f64) -> Bar {
        Bar {
            date: d(day),
            open: close,
            high: close + 1.0,
            low: close - 1.0,
            close,
        }
    }

    fn row(symbol: &str) -> MoverRow {
        MoverRow {
            symbol: symbol.into(),
            change_pct: -8.0,
            price: 50.0,
            market_cap: Some(2_000_000_000),
            avg_volume_3mo: Some(5_000_000),
            day_volume: Some(4_000_000),
            exchange: "NYSE".into(),
            first_trade_ms: Some(0),
        }
    }

    const NOW_MS: i64 = 1_786_000_000_000; // well past any test listing date

    #[test]
    fn floor_passes_quality_and_rejects_with_reasons() {
        let rows = vec![
            row("GOOD"),
            MoverRow {
                price: 3.0,
                ..row("CHEAP")
            },
            MoverRow {
                market_cap: None,
                ..row("NOCAP")
            },
            MoverRow {
                avg_volume_3mo: Some(100),
                ..row("THIN")
            },
            MoverRow {
                exchange: "Other OTC".into(),
                ..row("PINK")
            },
            MoverRow {
                first_trade_ms: Some(NOW_MS - 86_400_000), // listed yesterday
                ..row("IPO")
            },
        ];
        let (pass, rejects) = apply_floor(&rows, &QualityFloor::default(), NOW_MS);
        assert_eq!(pass.len(), 1);
        assert_eq!(pass[0].symbol, "GOOD");
        assert_eq!(rejects.len(), 5);
        let reason = |s: &str| {
            rejects
                .iter()
                .find(|r| r.symbol == s)
                .unwrap()
                .reason
                .clone()
        };
        assert!(reason("CHEAP").contains("price"));
        assert!(reason("NOCAP").contains("unavailable"));
        assert!(reason("THIN").contains("avg volume"));
        assert!(reason("PINK").contains("venue"));
        assert!(reason("IPO").contains("listed <"));
    }

    #[test]
    fn sma_and_rsi_math() {
        assert_eq!(sma(&[1.0, 2.0, 3.0, 4.0], 2), Some(3.5));
        assert_eq!(sma(&[1.0], 2), None);
        // deltas over period 3: +1, -1, +2 -> gains 3, losses 1 -> RSI 75
        let rsi = rsi_cutler(&[10.0, 11.0, 10.0, 12.0], 3).unwrap();
        assert!((rsi - 75.0).abs() < 1e-12);
        assert_eq!(rsi_cutler(&[10.0, 11.0], 3), None);
        assert_eq!(rsi_cutler(&[5.0; 20], 14), Some(50.0)); // flat -> neutral
        assert_eq!(rsi_cutler(&[1.0, 2.0, 3.0, 4.0], 3), Some(100.0)); // all gains
    }

    #[test]
    fn close_location_and_down_days() {
        let b = Bar {
            date: d(1),
            open: 108.0,
            high: 110.0,
            low: 100.0,
            close: 105.0,
        };
        assert_eq!(close_location(&b), Some(0.5));
        let flat = Bar {
            date: d(1),
            open: 5.0,
            high: 5.0,
            low: 5.0,
            close: 5.0,
        };
        assert_eq!(close_location(&flat), None);
        assert_eq!(consecutive_down_closes(&[5.0, 4.0, 3.0]), 2);
        assert_eq!(consecutive_down_closes(&[3.0, 4.0, 3.5]), 1);
        assert_eq!(consecutive_down_closes(&[1.0, 2.0]), 0);
        assert_eq!(consecutive_down_closes(&[1.0]), 0);
    }

    #[test]
    fn catalyst_hits_are_whole_word_and_deduped() {
        let hits = catalyst_hits(&["Earnings miss shocks", "dismissal of claims", "MISS again"]);
        assert_eq!(hits, vec!["earnings".to_string(), "miss".to_string()]);
        assert!(catalyst_hits(&["a quiet day"]).is_empty());
    }

    #[test]
    fn score_fractions_hit_bounds() {
        assert_eq!(frac_stretch(0.0), 0.0);
        assert_eq!(frac_stretch(3.5), 1.0);
        assert_eq!(frac_stretch(9.0), 1.0);
        assert_eq!(frac_quiet_volume(1.0), 1.0);
        assert_eq!(frac_quiet_volume(3.0), 0.0);
        assert_eq!(frac_quiet_volume(0.2), 1.0);
        assert_eq!(frac_rsi(50.0), 0.0);
        assert_eq!(frac_rsi(20.0), 1.0);
        assert_eq!(frac_idiosyncrasy(-3.0), 0.0);
        assert_eq!(frac_idiosyncrasy(-10.0), 1.0);
        assert_eq!(frac_multi_day(1), 0.0);
        assert_eq!(frac_multi_day(4), 1.0);
    }

    /// 30 prior bars trending gently down so ATR/SMA are sane, then a -8% day.
    fn prior() -> Vec<Bar> {
        (1..=30)
            .map(|i| bar_on(i, 110.0 - i as f64 * 0.5))
            .collect()
    }

    fn inputs<'a>(prior_bars: &'a [Bar], weights: &'a ScoreWeights) -> DipInputs<'a> {
        DipInputs {
            ticker: "TEST",
            prior_bars,
            // Gapped down to 88, sold to 80, closed 87.4 — near the high of
            // the day's range (location ≈ 0.87): buyers stepped in.
            drop_day_bar: Some(Bar {
                date: d(31),
                open: 88.0,
                high: 88.5,
                low: 80.0,
                close: 87.4,
            }),
            drop_date: d(31),
            last_price: 87.4,
            change_pct: -8.0,
            day_volume: Some(4_000_000),
            avg_volume_3mo: Some(5_000_000),
            spx_change_pct: Some(-0.5),
            filings: GateEvidence::Available(vec![]),
            headlines: GateEvidence::Available(vec![]),
            sentiment: Some(SentimentSummary {
                net_sentiment: 0.2,
                mentions: 25,
            }),
            session: Session::PostClose,
            floor: GateStatus::Pass,
            band: DropBand::default(),
            weights,
            score_min: 0.0,
        }
    }

    #[test]
    fn clean_post_close_setup_is_high_confidence() {
        let prior = prior();
        let w = ScoreWeights::default();
        let sig = dip_signal(&inputs(&prior, &w)).unwrap();
        assert_eq!(
            sig.verdict,
            Verdict::HighConfidence,
            "gates: {:?}",
            sig.gates
        );
        assert_eq!(sig.score_max, 100.0);
        assert!(sig.score > 0.0 && sig.score <= 100.0);
        assert!(sig.catalyst_evidence.is_empty());
        assert_eq!(sig.gates.close_strength, GateStatus::Pass);
    }

    #[test]
    fn filing_on_drop_day_is_no_setup_with_evidence() {
        let prior = prior();
        let w = ScoreWeights::default();
        let mut i = inputs(&prior, &w);
        i.filings = GateEvidence::Available(vec![
            Filing {
                form: "8-K".into(),
                filed_on: d(31),
            },
            Filing {
                form: "4".into(), // insider form — must NOT trigger
                filed_on: d(31),
            },
        ]);
        let sig = dip_signal(&i).unwrap();
        assert_eq!(sig.verdict, Verdict::NoSetup);
        assert_eq!(sig.catalyst_evidence.len(), 1);
        assert!(sig.catalyst_evidence[0].contains("8-K"));
    }

    #[test]
    fn old_filing_does_not_trigger() {
        let prior = prior();
        let w = ScoreWeights::default();
        let mut i = inputs(&prior, &w);
        i.filings = GateEvidence::Available(vec![Filing {
            form: "8-K".into(),
            filed_on: d(20), // well before the drop
        }]);
        let sig = dip_signal(&i).unwrap();
        assert_eq!(sig.gates.no_filing, GateStatus::Pass);
    }

    #[test]
    fn amended_and_variant_forms_trigger_plain_forms_do_not() {
        for form in [
            "8-K", "8-K/A", "6-K/A", "424B3", "424B5", "S-3/A", "S-3ASR", "FWP",
        ] {
            assert!(is_catalyst_form(form), "{form} should be a catalyst");
        }
        for form in ["4", "10-Q", "10-K", "S-1", "13F-HR", "SC 13G"] {
            assert!(!is_catalyst_form(form), "{form} should not be a catalyst");
        }
        // end-to-end: an amended 8-K on the drop day still kills the setup
        let prior = prior();
        let w = ScoreWeights::default();
        let mut i = inputs(&prior, &w);
        i.filings = GateEvidence::Available(vec![Filing {
            form: "8-K/A".into(),
            filed_on: d(31),
        }]);
        assert_eq!(dip_signal(&i).unwrap().verdict, Verdict::NoSetup);
    }

    #[test]
    fn catalyst_headline_is_no_setup_with_evidence() {
        let prior = prior();
        let w = ScoreWeights::default();
        let mut i = inputs(&prior, &w);
        i.headlines = GateEvidence::Available(vec![Headline {
            title: "Company cuts guidance after earnings miss".into(),
            publisher: "Wire".into(),
            published_at: Some(chrono::Utc::now()),
        }]);
        let sig = dip_signal(&i).unwrap();
        assert_eq!(sig.verdict, Verdict::NoSetup);
        assert!(sig.catalyst_evidence[0].contains("guidance"));
    }

    #[test]
    fn unavailable_evidence_fails_closed_to_watch() {
        let prior = prior();
        let w = ScoreWeights::default();
        let mut i = inputs(&prior, &w);
        i.filings = GateEvidence::Unavailable("EDGAR unreachable".into());
        let sig = dip_signal(&i).unwrap();
        assert_eq!(sig.verdict, Verdict::Watch);
        assert!(matches!(sig.gates.no_filing, GateStatus::Unknown(_)));
    }

    #[test]
    fn intraday_caps_at_watch_and_shrinks_score_max() {
        let prior = prior();
        let w = ScoreWeights::default();
        let mut i = inputs(&prior, &w);
        i.session = Session::Intraday;
        let sig = dip_signal(&i).unwrap();
        assert_eq!(sig.verdict, Verdict::Watch);
        assert_eq!(sig.score_max, 80.0);
        assert!(sig.components.close_location.is_none());
        assert!(sig.notes.iter().any(|n| n.contains("intraday")));
    }

    #[test]
    fn band_edges_and_catastrophic_tail() {
        let prior = prior();
        let w = ScoreWeights::default();
        for (chg, expect_setup) in [(-4.0, true), (-15.0, true), (-3.9, false), (-30.0, false)] {
            let mut i = inputs(&prior, &w);
            i.change_pct = chg;
            let sig = dip_signal(&i).unwrap();
            if expect_setup {
                assert_ne!(sig.verdict, Verdict::NoSetup, "change {chg}");
            } else {
                assert_eq!(sig.verdict, Verdict::NoSetup, "change {chg}");
            }
        }
    }

    #[test]
    fn market_driven_move_fails_idiosyncrasy_to_watch() {
        let prior = prior();
        let w = ScoreWeights::default();
        let mut i = inputs(&prior, &w);
        i.spx_change_pct = Some(-6.0); // tape-wide crash; excess = -2%
        let sig = dip_signal(&i).unwrap();
        assert_eq!(sig.verdict, Verdict::Watch);
        assert!(matches!(sig.gates.idiosyncratic, GateStatus::Fail(_)));
    }

    #[test]
    fn weak_close_fails_gate_to_watch() {
        let prior = prior();
        let w = ScoreWeights::default();
        let mut i = inputs(&prior, &w);
        i.drop_day_bar = Some(Bar {
            date: d(31),
            open: 94.0,
            high: 95.0,
            low: 87.0,
            close: 87.4, // near the low
        });
        let sig = dip_signal(&i).unwrap();
        assert_eq!(sig.verdict, Verdict::Watch);
        assert!(matches!(sig.gates.close_strength, GateStatus::Fail(_)));
    }

    #[test]
    fn score_floor_gate_and_divergence_scaling() {
        let prior = prior();
        let w = ScoreWeights::default();
        let mut i = inputs(&prior, &w);
        i.score_min = 1000.0; // unreachable
        let sig = dip_signal(&i).unwrap();
        assert_eq!(sig.verdict, Verdict::Watch);
        assert!(matches!(sig.gates.score_floor, GateStatus::Fail(_)));

        // 5 mentions scale divergence to a quarter of its weight
        let mut i = inputs(&prior, &w);
        i.sentiment = Some(SentimentSummary {
            net_sentiment: 0.2,
            mentions: 5,
        });
        let sig = dip_signal(&i).unwrap();
        assert!((sig.components.divergence - 2.5).abs() < 1e-12);

        // bearish sentiment (crowd capitulating) scores zero
        let mut i = inputs(&prior, &w);
        i.sentiment = Some(SentimentSummary {
            net_sentiment: -0.5,
            mentions: 50,
        });
        let sig = dip_signal(&i).unwrap();
        assert_eq!(sig.components.divergence, 0.0);
    }

    #[test]
    fn thin_history_and_bad_inputs_error() {
        let w = ScoreWeights::default();
        let short: Vec<Bar> = (1..=10).map(|i| bar_on(i, 100.0)).collect();
        assert!(dip_signal(&inputs(&short, &w)).is_err());
        let prior = prior();
        let mut i = inputs(&prior, &w);
        i.last_price = f64::NAN;
        assert!(dip_signal(&i).is_err());
    }
}
