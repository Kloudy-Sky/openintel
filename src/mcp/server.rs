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
    market: YahooMarketSource,
    pulse_feed: Option<Arc<crate::adapters::sources::x::XPulseSource>>,
}

impl OpenIntelServer {
    pub fn new(
        social: Vec<Box<dyn SocialDataSource>>,
        market: YahooMarketSource,
        pulse_feed: Option<crate::adapters::sources::x::XPulseSource>,
    ) -> Self {
        Self {
            tool_router: Self::tool_router(),
            social: Arc::new(social),
            market,
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
                       alignment = confirming/diverging/quiet). Read-only — does not trade."
    )]
    async fn analyze_ticker(
        &self,
        Parameters(args): Parameters<tools::AnalyzeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = tools::run_analyze(args, &self.social, &self.market)
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
                       (limit × $0.005) to the user and get their confirmation. Also propose \
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
                 report (crowding, divergence, sentiment). READ-ONLY: it never places trades.",
            )
    }
}

/// Run the MCP server over stdio (blocks until the client disconnects).
pub async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let store = crate::config::store::KeychainStore::new();
    let credentials = Credentials::load(&store);
    let social = crate::adapters::sources::build_social_sources(&credentials);

    let market = YahooMarketSource::new()?;
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
    let service = OpenIntelServer::new(social, market, pulse_feed)
        .serve(stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
