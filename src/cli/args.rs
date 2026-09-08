use clap::{Parser, Subcommand, ValueEnum};

use crate::config::settings::{AppConfig, OutputFormat};

#[derive(Parser, Debug)]
#[command(
    name = "openintel",
    version,
    about = "Fuse social sentiment with market action into a speculation report"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Analyze a ticker across social + market sources
    Analyze(AnalyzeArgs),

    /// Run as an MCP server over stdio (for AI agents).
    Mcp,

    /// Guided setup + live verify for a data source (saves to the OS keychain; env vars override)
    Setup(SetupArgs),

    /// Catalyst posts from specific high-impact X accounts (paid X API — opt-in)
    Pulse(PulseArgs),

    /// Deterministic risk math for one trade idea: ATR stop, budget-capped size, R targets
    Risk(RiskArgs),

    /// Scan the day's biggest losers for gated dip setups (grades conformance, never advises)
    Dip(DipArgs),

    /// Trade journal: log entries with a frozen thesis, track positions, grade the record
    Journal(JournalArgs),

    /// Surface today's movers with evidence attached (period extremes, catalyst gates — never picks)
    Discover(DiscoverArgs),

    /// Market clock: date, weekday, and NYSE session state (pre-market, open, post-close, closed)
    Clock(ClockArgs),

    /// Today's dated evidence with no ticker: clock, macro releases, earnings, overnight filings and headlines, chatter (never picks)
    Brief(BriefArgs),

    /// Defined-risk frame for a long call or put: contracts to a budget, breakeven, the move it needs (never the odds)
    #[command(name = "option")]
    OptionFrame(OptionFrameArgs),

    /// Watch held and listed names and emit dated events as they are seen: filings, catalyst headlines, ATR moves, chatter velocity, market state (never orders)
    Watch(WatchArgs),
}

#[derive(clap::Args, Debug)]
pub struct WatchArgs {
    /// Tickers to watch in addition to open journal positions, comma-separated
    #[arg(long, value_delimiter = ',')]
    pub tickers: Vec<String>,

    /// Seconds between polls (15-3600)
    #[arg(long, default_value_t = 60)]
    pub interval: u64,

    /// Whole ATR multiples from the prior close that each earn one event
    #[arg(long = "move-atr", default_value_t = 1.0)]
    pub move_atr: f64,

    /// Minutes between read-only chatter passes over the free listening feeds (0 = off)
    #[arg(long = "chatter-every", default_value_t = 15)]
    pub chatter_every: u64,

    /// ntfy.sh topic to push each event to (or env OPENINTEL_NTFY_TOPIC); omit for no push
    #[arg(long = "ntfy-topic")]
    pub ntfy_topic: Option<String>,

    /// Poll once and exit
    #[arg(long)]
    pub once: bool,
}

#[derive(clap::Args, Debug)]
pub struct OptionFrameArgs {
    /// Underlying equity or ETF ticker, e.g. NVDA
    pub underlying: String,

    /// call or put (long only)
    #[arg(long, value_enum)]
    pub kind: OptionKindArg,

    #[arg(long)]
    pub strike: f64,

    /// Expiry, YYYY-MM-DD
    #[arg(long)]
    pub expiry: String,

    /// Quoted premium per share from the broker's chain (the contract costs premium × 100)
    #[arg(long)]
    pub premium: f64,

    /// Budget in USD: the whole premium is the max loss
    #[arg(long)]
    pub budget: f64,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}

#[derive(clap::Args, Debug)]
pub struct BriefArgs {
    /// Held and watched tickers to check for overnight filings and headlines, comma-separated
    #[arg(long, value_delimiter = ',')]
    pub tickers: Vec<String>,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}

#[derive(clap::Args, Debug)]
pub struct ClockArgs {
    /// Which market's clock: equity (NYSE), crypto (24/7), forex or future (Sun 17:00 to Fri 17:00 ET)
    #[arg(long, value_enum, default_value_t = AssetArg::Equity)]
    pub asset: AssetArg,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum AssetArg {
    Equity,
    Crypto,
    Forex,
    Future,
}

impl AssetArg {
    pub fn class(self) -> crate::domain::values::asset_class::AssetClass {
        use crate::domain::values::asset_class::AssetClass;
        match self {
            AssetArg::Equity => AssetClass::Equity,
            AssetArg::Crypto => AssetClass::Crypto,
            AssetArg::Forex => AssetClass::Forex,
            AssetArg::Future => AssetClass::Future,
        }
    }
}

#[derive(clap::Args, Debug)]
pub struct AnalyzeArgs {
    /// Ticker symbol, e.g. AAPL
    pub ticker: String,

    #[arg(long)]
    pub enable_reddit: bool,
    #[arg(long)]
    pub enable_bluesky: bool,

    /// Skip the market snapshot (social-only report)
    #[arg(long)]
    pub no_market: bool,

    /// Posts to fetch per source
    #[arg(long, default_value_t = 50)]
    pub limit: usize,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatArg {
    Table,
    Json,
}

#[derive(clap::Args, Debug)]
pub struct SetupArgs {
    /// Which source to set up
    #[arg(value_enum)]
    pub source: SetupSource,

    /// Remove this source's saved credentials from the OS keychain
    #[arg(long)]
    pub forget: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupSource {
    Reddit,
    Bluesky,
    X,
}

#[derive(clap::Args, Debug)]
pub struct PulseArgs {
    /// Ticker symbol, e.g. NVDA
    pub ticker: String,

    /// X handles to listen to, comma-separated (no @). Default: the macro list.
    #[arg(long, value_delimiter = ',')]
    pub accounts: Vec<String>,

    /// Extra search terms in the accounts' own language, comma-separated;
    /// phrases allowed (e.g. tesla,robotaxi,General Motors) — cashtags are
    /// rare in influencer posts
    #[arg(long, value_delimiter = ',')]
    pub keywords: Vec<String>,

    /// Lookback window in hours (1-167)
    #[arg(long, default_value_t = 24)]
    pub hours: u32,

    /// Max posts to read — ~$0.005 per post returned (deduped over 24h); the
    /// search floor can return up to 10 even for smaller limits (1-100)
    #[arg(long, default_value_t = 20)]
    pub limit: usize,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectionArg {
    Long,
    Short,
}

#[derive(clap::Args, Debug)]
pub struct RiskArgs {
    /// Ticker symbol, e.g. NVDA
    pub ticker: String,

    /// Per-trade risk budget in USD — the most a stop-out may lose
    #[arg(long)]
    pub budget: f64,

    #[arg(long, value_enum, default_value_t = DirectionArg::Long)]
    pub direction: DirectionArg,

    /// Stop distance in ATR multiples (0.5-5)
    #[arg(long = "stop-mult", default_value_t = 2.0)]
    pub stop_mult: f64,

    /// Entry price override (default: last close)
    #[arg(long)]
    pub entry: Option<f64>,

    /// Size in fractional units instead of whole shares (default for crypto and forex)
    #[arg(long)]
    pub fractional: bool,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}

#[derive(clap::Args, Debug)]
pub struct DipArgs {
    /// Evaluate one symbol instead of scanning the losers universe
    pub ticker: Option<String>,

    /// Grade the scan journal against forward returns instead of scanning
    #[arg(long, conflicts_with = "ticker")]
    pub review: bool,

    /// Losers to pull from the screener (1-100)
    #[arg(long, default_value_t = 100)]
    pub count: usize,

    /// Floor+band survivors to deep-analyze (1-25)
    #[arg(long, default_value_t = 10)]
    pub deep: usize,

    /// Worst day-change eligible, in percent
    #[arg(long = "band-min", default_value_t = -15.0, allow_negative_numbers = true)]
    pub band_min: f64,

    /// Mildest day-change eligible, in percent
    #[arg(long = "band-max", default_value_t = -4.0, allow_negative_numbers = true)]
    pub band_max: f64,

    /// Account equity in USD — enables the risk + margin sizing section
    #[arg(long)]
    pub equity: Option<f64>,

    /// Buying-power multiple (overnight Reg-T caps at 2)
    #[arg(long, default_value_t = 2.0)]
    pub leverage: f64,

    /// Unlock 4x intraday buying power (not holdable overnight)
    #[arg(long)]
    pub intraday_bp: bool,

    /// Maintenance requirement fraction
    #[arg(long, default_value_t = 0.25)]
    pub maintenance: f64,

    /// Fraction of equity risked to the stop per position
    #[arg(long = "risk-pct", default_value_t = 0.01)]
    pub risk_pct: f64,

    /// Minimum composite score for the score gate
    #[arg(long = "score-min", default_value_t = 65.0)]
    pub score_min: f64,

    /// Quality floor: minimum share price
    #[arg(long = "min-price", default_value_t = 5.0)]
    pub min_price: f64,

    /// Quality floor: minimum market cap in USD
    #[arg(long = "min-cap", default_value_t = 500_000_000)]
    pub min_cap: u64,

    /// Quality floor: minimum 3-month average daily volume in shares
    #[arg(long = "min-volume", default_value_t = 1_000_000)]
    pub min_volume: u64,

    /// Quality floor: minimum days since listing
    #[arg(long = "min-listed-days", default_value_t = 180)]
    pub min_listed_days: i64,

    /// Skip the scan journal (~/.openintel/dip_journal.jsonl)
    #[arg(long)]
    pub no_journal: bool,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenArg {
    Gainers,
    Losers,
    Actives,
    Crypto,
}

#[derive(clap::Args, Debug)]
pub struct DiscoverArgs {
    /// Chatter mode: cashtag mention velocity from the listening set
    /// (~/.openintel/listening.json) instead of the movers screens
    #[arg(long)]
    pub chatter: bool,

    /// Chatter: include the paid X leg — ~$0.005 per post returned, capped by --x-limit
    #[arg(long, requires = "chatter")]
    pub x: bool,

    /// Chatter: lookback window in hours (1-167)
    #[arg(long, default_value_t = 24, requires = "chatter")]
    pub hours: u32,

    /// Chatter: max X posts returned (each bills ~$0.005; the search floor can return up to 10)
    #[arg(long = "x-limit", default_value_t = 20, requires = "x")]
    pub x_limit: usize,

    /// Movers: screens to pull, comma-separated (default: all three)
    #[arg(long, value_enum, value_delimiter = ',', conflicts_with = "chatter")]
    pub screens: Vec<ScreenArg>,

    /// Movers: rows pulled per screen (1-100)
    #[arg(long, default_value_t = 25)]
    pub count: usize,

    /// Movers: total candidates deep-annotated across screens (1-25)
    #[arg(long, default_value_t = 9)]
    pub deep: usize,

    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    pub format: FormatArg,
}

#[derive(clap::Args, Debug)]
pub struct JournalArgs {
    #[command(subcommand)]
    pub command: JournalCommand,
}

#[derive(Subcommand, Debug)]
pub enum JournalCommand {
    /// Log an opened trade — thesis and plan are frozen at this moment
    Log(JournalLogArgs),
    /// Append a note to an open trade, optionally moving the stop or target
    Amend(JournalAmendArgs),
    /// Close an open trade with an exit price and reason
    Close(JournalCloseArgs),
    /// Every open trade with its frozen thesis and plan
    Positions {
        #[arg(long, value_enum, default_value_t = FormatArg::Table)]
        format: FormatArg,
    },
    /// Grade the whole journal: R vs original stops, discipline, forward returns
    Review {
        #[arg(long, value_enum, default_value_t = FormatArg::Table)]
        format: FormatArg,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionKindArg {
    Call,
    Put,
}

#[derive(clap::Args, Debug)]
pub struct JournalLogArgs {
    /// Ticker (equity), underlying (with --option), or symbol (with --crypto)
    pub symbol: String,

    /// Shares / contracts / units
    #[arg(long)]
    pub qty: f64,

    /// Fill price of the traded instrument (premium per contract for options)
    #[arg(long)]
    pub entry: f64,

    /// Exit-if price of the traded instrument, strictly below entry
    #[arg(long)]
    pub stop: f64,

    /// Why this trade, in your own words — frozen forever
    #[arg(long)]
    pub thesis: String,

    /// Setup label, e.g. sr-support-bounce
    #[arg(long)]
    pub tag: String,

    /// Optional target price, above entry
    #[arg(long)]
    pub target: Option<f64>,

    /// USD at risk on this trade (pair with --wallet)
    #[arg(long = "risk-usd", requires = "wallet")]
    pub risk_usd: Option<f64>,

    /// Wallet size right now, for the %-of-wallet snapshot (pair with --risk-usd)
    #[arg(long, requires = "risk_usd")]
    pub wallet: Option<f64>,

    /// The instrument is a long option on SYMBOL
    #[arg(long, value_enum)]
    pub option: Option<OptionKindArg>,

    /// Option strike price
    #[arg(long, requires = "option")]
    pub strike: Option<f64>,

    /// Option expiry, YYYY-MM-DD
    #[arg(long, requires = "option")]
    pub expiry: Option<String>,

    /// The instrument is crypto (BTC or BTC-USD)
    #[arg(long, conflicts_with_all = ["option", "strike", "expiry", "forex"])]
    pub crypto: bool,

    /// The instrument is a forex pair (EURUSD or EURUSD=X); journaled only, no execution rail here
    #[arg(long, conflicts_with_all = ["option", "strike", "expiry", "crypto"])]
    pub forex: bool,

    /// Skip the same-day duplicate guard
    #[arg(long = "allow-duplicate")]
    pub allow_duplicate: bool,
}

#[derive(clap::Args, Debug)]
pub struct JournalAmendArgs {
    /// Trade id, e.g. NVDA-20260821-1
    pub trade_id: String,

    /// Why — required for every amendment
    #[arg(long)]
    pub note: String,

    /// New stop price (widened stops are reported by review)
    #[arg(long)]
    pub stop: Option<f64>,

    /// New target price
    #[arg(long)]
    pub target: Option<f64>,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalCloseReasonArg {
    Stop,
    Target,
    Discretion,
    Expiry,
}

#[derive(clap::Args, Debug)]
pub struct JournalCloseArgs {
    /// Trade id, e.g. NVDA-20260821-1
    pub trade_id: String,

    /// Exit fill price
    #[arg(long)]
    pub exit: f64,

    /// What ended it
    #[arg(long, value_enum)]
    pub reason: JournalCloseReasonArg,

    /// Optional context
    #[arg(long)]
    pub note: Option<String>,
}

pub fn to_app_config(args: &AnalyzeArgs) -> AppConfig {
    let format = match args.format {
        FormatArg::Table => OutputFormat::Table,
        FormatArg::Json => OutputFormat::Json,
    };
    AppConfig::new(
        args.ticker.clone(),
        args.enable_reddit,
        args.enable_bluesky,
        args.no_market,
        args.limit,
        format,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_analyze_with_json_format() {
        let cli =
            Cli::try_parse_from(["openintel", "analyze", "AAPL", "--format", "json"]).unwrap();
        let Command::Analyze(args) = cli.command else {
            unreachable!()
        };
        assert_eq!(args.ticker, "AAPL");
        assert_eq!(args.format, FormatArg::Json);
        assert_eq!(args.limit, 50);
    }

    #[test]
    fn maps_no_flags_to_all_sources() {
        let cli = Cli::try_parse_from(["openintel", "analyze", "MSFT"]).unwrap();
        let Command::Analyze(args) = cli.command else {
            unreachable!()
        };
        let cfg = to_app_config(&args);
        assert_eq!(cfg.enabled_sources.len(), 2);
        assert!(cfg.market_enabled);
        assert_eq!(cfg.format, crate::config::settings::OutputFormat::Table);
    }

    #[test]
    fn enable_x_flag_no_longer_exists() {
        assert!(Cli::try_parse_from(["openintel", "analyze", "AAPL", "--enable-x"]).is_err());
    }

    #[test]
    fn parses_setup_reddit() {
        let cli = Cli::try_parse_from(["openintel", "setup", "reddit"]).unwrap();
        let Command::Setup(args) = cli.command else {
            panic!("expected setup command");
        };
        assert_eq!(args.source, SetupSource::Reddit);
    }

    #[test]
    fn parses_setup_bluesky() {
        let cli = Cli::try_parse_from(["openintel", "setup", "bluesky"]).unwrap();
        let Command::Setup(args) = cli.command else {
            panic!("expected setup command");
        };
        assert_eq!(args.source, SetupSource::Bluesky);
    }

    #[test]
    fn parses_setup_x() {
        let cli = Cli::try_parse_from(["openintel", "setup", "x"]).unwrap();
        let Command::Setup(args) = cli.command else {
            panic!("expected setup command");
        };
        assert_eq!(args.source, SetupSource::X);
    }

    #[test]
    fn rejects_unknown_setup_source() {
        assert!(Cli::try_parse_from(["openintel", "setup", "bogus"]).is_err());
    }

    #[test]
    fn parses_setup_forget_flag() {
        let cli = Cli::try_parse_from(["openintel", "setup", "reddit", "--forget"]).unwrap();
        let Command::Setup(args) = cli.command else {
            panic!("expected setup command");
        };
        assert!(args.forget);
    }

    #[test]
    fn parses_pulse_with_accounts() {
        let cli = Cli::try_parse_from([
            "openintel",
            "pulse",
            "NVDA",
            "--accounts",
            "jensenhuang,elonmusk",
            "--hours",
            "48",
        ])
        .unwrap();
        let Command::Pulse(args) = cli.command else {
            panic!("expected pulse command");
        };
        assert_eq!(args.ticker, "NVDA");
        assert_eq!(args.accounts, vec!["jensenhuang", "elonmusk"]);
        assert_eq!(args.hours, 48);
        assert_eq!(args.limit, 20);
    }

    #[test]
    fn pulse_defaults_have_empty_accounts() {
        let cli = Cli::try_parse_from(["openintel", "pulse", "GME"]).unwrap();
        let Command::Pulse(args) = cli.command else {
            panic!("expected pulse command");
        };
        assert!(args.accounts.is_empty());
        assert!(args.keywords.is_empty());
        assert_eq!(args.hours, 24);
    }

    #[test]
    fn parses_pulse_with_keywords() {
        let cli = Cli::try_parse_from([
            "openintel",
            "pulse",
            "TSLA",
            "--accounts",
            "elonmusk",
            "--keywords",
            "tesla,robotaxi",
        ])
        .unwrap();
        let Command::Pulse(args) = cli.command else {
            panic!("expected pulse command");
        };
        assert_eq!(args.keywords, vec!["tesla", "robotaxi"]);
    }

    #[test]
    fn parses_risk_args() {
        let cli = Cli::try_parse_from([
            "openintel",
            "risk",
            "NVDA",
            "--budget",
            "200",
            "--direction",
            "short",
            "--stop-mult",
            "1.5",
        ])
        .unwrap();
        let Command::Risk(args) = cli.command else {
            panic!("expected risk command");
        };
        assert_eq!(args.ticker, "NVDA");
        assert_eq!(args.budget, 200.0);
        assert_eq!(args.direction, DirectionArg::Short);
        assert_eq!(args.stop_mult, 1.5);
        assert!(args.entry.is_none());
    }

    #[test]
    fn risk_requires_budget() {
        assert!(Cli::try_parse_from(["openintel", "risk", "NVDA"]).is_err());
    }
}
