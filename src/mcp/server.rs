use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::io::stdio;
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt};

use crate::mcp::tools;

#[derive(Clone)]
pub struct OpenIntelServer {
    tool_router: ToolRouter<OpenIntelServer>,
}

impl OpenIntelServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

impl Default for OpenIntelServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router]
impl OpenIntelServer {
    #[tool(
        description = "List the social and market data sources OpenIntel can analyze. Read-only metadata."
    )]
    async fn list_sources(&self) -> Result<CallToolResult, ErrorData> {
        let json = serde_json::to_string_pretty(&tools::run_list_sources())
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
        let out = tools::run_analyze(args)
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
        let out = tools::run_scan(args).await;
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
        let out = tools::run_compare(args).await;
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
    let service = OpenIntelServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
