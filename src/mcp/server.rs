use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::io::stdio;
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt};

use crate::adapters::market::yahoo::YahooMarketSource;
use crate::config::secrets::Credentials;
use crate::domain::ports::social_data_source::SocialDataSource;
use crate::mcp::tools;

#[derive(Clone)]
pub struct OpenIntelServer {
    tool_router: ToolRouter<OpenIntelServer>,
    social: Arc<Vec<Box<dyn SocialDataSource>>>,
    listening: Arc<Vec<Box<dyn crate::domain::ports::listening_feed::ListeningFeed>>>,
    market: YahooMarketSource,
    filings: Arc<crate::adapters::filings::edgar::EdgarSource>,
    pulse_feed: Option<Arc<crate::adapters::sources::x::XPulseSource>>,
}

impl OpenIntelServer {
    pub fn new(
        social: Vec<Box<dyn SocialDataSource>>,
        listening: Vec<Box<dyn crate::domain::ports::listening_feed::ListeningFeed>>,
        market: YahooMarketSource,
        filings: crate::adapters::filings::edgar::EdgarSource,
        pulse_feed: Option<crate::adapters::sources::x::XPulseSource>,
    ) -> Self {
        Self {
            tool_router: Self::tool_router(),
            social: Arc::new(social),
            listening: Arc::new(listening),
            market,
            filings: Arc::new(filings),
            pulse_feed: pulse_feed.map(Arc::new),
        }
    }
}

#[tool_router]
impl OpenIntelServer {
    #[tool(
        description = "List the social and market data sources OpenIntel can analyze. Read-only metadata."
    )]
    async fn list_sources(&self) -> Result<CallToolResult, ErrorData> {
        let json =
            serde_json::to_string_pretty(&tools::run_list_sources(&self.social, &self.market))
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Analyze one ticker: fuse social sentiment with market action into a \
                       speculation report (net sentiment, speculation index, crowding, \
                       alignment = confirming/diverging/quiet). On a ≤ -4% down day the output \
                       also carries dip_signal — the same gated setup verdict dip_scan computes \
                       (capped at watch in this mode). Read-only — does not trade."
    )]
    async fn analyze_ticker(
        &self,
        Parameters(args): Parameters<tools::AnalyzeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let dip_deps = crate::application::dip::DipDeps {
            bars: &self.market,
            news: &self.market,
            filings: self.filings.as_ref(),
            social: &self.social,
            market: Some(&self.market),
        };
        let out = tools::run_analyze(args, &self.social, &self.market, Some(&dip_deps))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Analyze a watchlist of tickers concurrently. Returns one entry per \
                       ticker (report or error); one bad ticker does not fail the batch. \
                       Read-only — does not trade."
    )]
    async fn scan_watchlist(
        &self,
        Parameters(args): Parameters<tools::ScanArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = tools::run_scan(args, &self.social, &self.market).await;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Compare tickers and rank them by a chosen signal: rank_by ∈ \
                       {crowding (default), speculation_index, net_sentiment, divergence}. \
                       Read-only — does not trade."
    )]
    async fn compare_tickers(
        &self,
        Parameters(args): Parameters<tools::CompareArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = tools::run_compare(args, &self.social, &self.market).await;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Fetch recent posts about a ticker from specific high-impact X accounts \
                       (paid API: ~$0.005 per post read). Before calling: research which accounts \
                       actually matter for this ticker — CEO/founder, major institutional holders \
                       or activist funds, respected sector journalists, and market-moving macro \
                       figures — then propose the account list and estimated max cost \
                       (up to max(limit, 10) × $0.005 — billing is per post returned, deduped \
                       over 24h; the search floor can return up to 10 posts even for smaller \
                       limits) to the user and get their confirmation. Also propose \
                       company-language keywords (e.g. \"Tesla\" for TSLA) — these accounts \
                       rarely write cashtags, so symbol-only matching misses their posts. \
                       Omit `accounts` only if the user asks for the default macro list. \
                       Returned posts are \
                       catalyst events — reason about them directly; do not treat them as a \
                       sentiment sample. Read-only — does not trade."
    )]
    async fn x_pulse(
        &self,
        Parameters(args): Parameters<tools::PulseToolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(feed) = self.pulse_feed.as_deref() else {
            return Err(ErrorData::invalid_request(
                "x is not configured — set OPENINTEL_X_BEARER or run `openintel setup x`"
                    .to_string(),
                None,
            ));
        };
        let out = tools::run_pulse(args, feed)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Deterministic risk calculator: given a ticker, a per-trade risk budget in \
                       USD, and a direction, returns an ATR(14)-based stop level, the whole-share \
                       size that caps a stop-out at the budget, max loss, and 1R/2R/3R reference \
                       levels. It does NOT recommend trades — combine it with analyze_ticker / \
                       x_pulse, present the numbers to the user, and get their explicit approval \
                       before any execution step. Read-only — does not trade."
    )]
    async fn risk_frame(
        &self,
        Parameters(args): Parameters<tools::RiskToolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = tools::run_risk_frame(args, &self.market)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Scan the day's biggest losers for dip-buy SETUPS, or evaluate one ticker \
                       (pass `ticker`). Hard gates decide a tiered verdict — no_setup / watch / \
                       high_confidence: quality floor, -15%..-4% drop band, no same-day SEC \
                       filing (8-K/6-K/424B5/S-3/FWP via EDGAR), no catalyst headline, \
                       idiosyncratic vs the index, buyers-at-the-close, and a score floor. \
                       Unverifiable evidence FAILS CLOSED (caps at watch); intraday runs always \
                       cap at watch. 'high_confidence' means conformance to the setup template, \
                       NEVER probability of profit; zero candidates is a normal result. Passing \
                       equity_usd adds ATR-stop sizing plus margin mechanics (overnight Reg-T 2x \
                       cap, margin-call price; single-position model, interest not modeled). \
                       Appends a scan journal line to ~/.openintel/dip_journal.jsonl unless \
                       no_journal. The composite score's weights are v0 and unvalidated. \
                       Read-only — does not trade."
    )]
    async fn dip_scan(
        &self,
        Parameters(args): Parameters<tools::DipScanArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let deps = crate::application::dip::DipDeps {
            bars: &self.market,
            news: &self.market,
            filings: self.filings.as_ref(),
            social: &self.social,
            market: Some(&self.market),
        };
        let out = tools::run_dip_scan(args, &self.market, &deps)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Grade the dip_scan journal (~/.openintel/dip_journal.jsonl) against \
                       forward returns: 1/5/10-day raw and SPY-adjusted stats per verdict, \
                       plus score↔return correlation. This is the evidence for whether \
                       dip_scan's v0 score weights have any edge — until the graded sample is \
                       meaningful (n≥30), treat verdicts as setup conformance only. Entries \
                       older than ~3 months are ungradable (bar history limit). Read-only — \
                       does not trade."
    )]
    async fn dip_review(&self) -> Result<CallToolResult, ErrorData> {
        let out = tools::run_dip_review(&self.market)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Answer \"where are trades today?\" without a ticker. Mode \"movers\" \
                       (default): Yahoo's predefined screens (gainers / losers / most actives), \
                       quality floor, then evidence per candidate — day change, RVOL, \
                       ATR-stretch, distance to 3-month and ~1-year period extremes (a proxy, \
                       NOT support/resistance), same-day SEC-filing and catalyst-headline gates \
                       (unverifiable evidence reads Unknown), and social attention where \
                       configured. Mode \"chatter\": cashtag mention velocity from the user's \
                       listening set (~/.openintel/listening.json — propose curation additions \
                       for the user to approve), graded against a baseline journal with honesty \
                       gates (no velocity claim below 5 mentions / 3 baseline days), plus the \
                       raw recent posts (influencers write company names, not cashtags — read \
                       them). include_x adds the PAID X leg: ~$0.005 per post returned, deduped \
                       24h, default cap 20 ≈ $0.10 max — state the cost and get the user's \
                       confirmation BEFORE calling with include_x. NO ranking, NO verdicts, NO \
                       picks. For gated dip verdicts on losers run dip_scan; before any trade \
                       run risk_frame and get explicit user approval. Read-only — never trades."
    )]
    async fn discover(
        &self,
        Parameters(args): Parameters<tools::DiscoverToolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if matches!(args.mode, Some(tools::DiscoverModeArg::Chatter)) {
            let mut feeds: Vec<&dyn crate::domain::ports::listening_feed::ListeningFeed> =
                self.listening.iter().map(|f| f.as_ref() as _).collect();
            if args.include_x.unwrap_or(false) {
                match self.pulse_feed.as_deref() {
                    Some(x) => feeds.push(x),
                    None => {
                        return Err(ErrorData::invalid_request(
                            "include_x set but X is not configured — run `openintel setup x`"
                                .to_string(),
                            None,
                        ))
                    }
                }
            }
            let out = tools::run_chatter(&args, &feeds, Some(&self.market))
                .await
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            let json = serde_json::to_string_pretty(&out)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            return Ok(CallToolResult::success(vec![ContentBlock::text(json)]));
        }
        let deps = crate::application::discover::DiscoverDeps {
            movers: &self.market,
            bars: &self.market,
            news: &self.market,
            filings: self.filings.as_ref(),
            social: &self.social,
        };
        let out = tools::run_discover(args, &deps)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Journal a trade the user just APPROVED — call this in the same moment \
                       the order is placed, before the conversation moves on. The thesis must \
                       be the user's why in their own words; planned_stop is required (no entry \
                       without a stop) and refers to the traded instrument's price (premium for \
                       options). Pass wallet_usd fresh from the broker with risk_usd so the \
                       %-of-wallet snapshot never goes stale. A same-day duplicate (instrument \
                       + qty + entry) is rejected with the existing id — a retry, not an error. \
                       Writes only the local journal; never trades."
    )]
    async fn log_trade(
        &self,
        Parameters(args): Parameters<tools::LogTradeToolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = tools::run_log_trade(args)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Amend an open journaled trade (append a note, move the stop or target — \
                       the original plan stays frozen and widened stops are reported by \
                       review_trades) or close it with an exit price and reason \
                       (stop/target/discretion/expiry). Log the close in the same conversation \
                       the exit happens. Writes only the local journal; never trades."
    )]
    async fn update_trade(
        &self,
        Parameters(args): Parameters<tools::UpdateTradeToolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = tools::run_update_trade(args)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Every open journaled trade with its frozen thesis, plan, amendments, and \
                       age — the cross-session memory. Call this FIRST in a new conversation \
                       about trading, before proposing anything, so existing positions and \
                       their reasoning are in context. Read-only."
    )]
    async fn open_positions(&self) -> Result<CallToolResult, ErrorData> {
        let out = tools::run_open_positions()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Grade the whole trade journal: realized R vs the ORIGINAL stop, win \
                       rates, discipline flags (widened stops, exits beyond the planned stop), \
                       forward returns for equities (raw and SPY-adjusted), buckets by setup \
                       tag and instrument. Options grade on realized P&L plus underlying \
                       direction; crypto on realized P&L only. Honesty gates: numbers appear \
                       only at n≥10 per bucket and conclusions need n≥30 closed trades — below \
                       that, everything is anecdote and the report says so. Read-only."
    )]
    async fn review_trades(&self) -> Result<CallToolResult, ErrorData> {
        let out = tools::run_review_trades(&self.market)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for OpenIntelServer {
    fn get_info(&self) -> ServerInfo {
        // NOTE: `Implementation::from_build_env()` expands `env!` *inside* the rmcp
        // crate, so it would report rmcp's own name/version ("rmcp" / "2.0.0") rather
        // than ours. Build it from this crate's env vars so the server identifies as
        // openintel.
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "OpenIntel — fuses social sentiment with market action into a speculation \
                 report (crowding, divergence, sentiment), plus deterministic risk/margin \
                 calculators and a gated dip-setup scanner with a forward-return review. \
                 READ-ONLY: it never places trades.",
            )
    }
}

/// Run the MCP server over stdio (blocks until the client disconnects).
pub async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let store = crate::config::store::KeychainStore::new();
    let credentials = Credentials::load(&store);
    let social = crate::adapters::sources::build_social_sources(&credentials);

    let market = YahooMarketSource::new()?;
    let filings = crate::adapters::filings::edgar::EdgarSource::new()?;
    let pulse_feed = match credentials.x_bearer.clone() {
        Some(bearer) => match crate::adapters::sources::x::XPulseSource::new(bearer) {
            Ok(src) => Some(src),
            Err(e) => {
                eprintln!("warning: x pulse disabled: {e}");
                None
            }
        },
        None => None,
    };
    let listening = crate::adapters::sources::build_free_listening_feeds(&credentials);
    let service = OpenIntelServer::new(social, listening, market, filings, pulse_feed)
        .serve(stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
