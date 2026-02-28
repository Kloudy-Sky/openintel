use clap::Parser;
use openintel::cli::commands::{Cli, Commands};
use openintel::domain::values::category::Category;
use openintel::domain::values::execution::{
    ExecutionMode, ExecutionResult, SkippedOpportunity, TradePlan,
};
use openintel::domain::values::portfolio::{AssetClass, Portfolio, Position};
use openintel::domain::values::source_type::SourceType;
use openintel::domain::values::trade_direction::TradeDirection;
use openintel::domain::values::trade_outcome::TradeOutcome;
use openintel::infrastructure::feeds::{Feed, FeedResult, FetchOutput};
use openintel::OpenIntel;

/// Default Yahoo Finance watchlist tickers.
const DEFAULT_YAHOO_TICKERS: &[&str] = &[
    "IONQ", "NVDA", "CRCL", "COIN", "MARA", "RIOT", "SPY", "QQQ", "SQ",
];

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let db_path = std::env::var("OPENINTEL_DB").unwrap_or_else(|_| "./openintel.db".into());

    let oi = match OpenIntel::new(&db_path) {
        Ok(oi) => oi,
        Err(e) => {
            eprintln!("Error initializing OpenIntel: {e}");
            std::process::exit(1);
        }
    };

    let result = run_command(oi, cli.command).await;
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

/// Helper function to initialize feeds based on source and ticker list.
fn initialize_feeds(source: &str, ticker_list: Vec<String>) -> Result<Vec<Box<dyn Feed>>, String> {
    match source.to_lowercase().as_str() {
        "yahoo" => {
            let tickers = if ticker_list.is_empty() {
                DEFAULT_YAHOO_TICKERS
                    .iter()
                    .map(|s| String::from(*s))
                    .collect()
            } else {
                ticker_list
            };
            Ok(vec![Box::new(
                openintel::infrastructure::feeds::yahoo::YahooFeed::new(tickers),
            )])
        }
        "nws" => Ok(vec![Box::new(
            openintel::infrastructure::feeds::nws::NwsFeed::central_park(),
        )]),
        "kalshi" => {
            let feed = if ticker_list.is_empty() {
                openintel::infrastructure::feeds::kalshi::KalshiFeed::default_series()
            } else {
                openintel::infrastructure::feeds::kalshi::KalshiFeed::new(ticker_list)
            };
            Ok(vec![Box::new(feed)])
        }
        "all" => {
            if !ticker_list.is_empty() {
                eprintln!("Note: --tickers only applies to Yahoo Finance in 'all' mode. NWS and Kalshi use defaults.");
            }
            let yahoo_tickers = if ticker_list.is_empty() {
                DEFAULT_YAHOO_TICKERS
                    .iter()
                    .map(|s| String::from(*s))
                    .collect()
            } else {
                ticker_list
            };
            Ok(vec![
                Box::new(openintel::infrastructure::feeds::yahoo::YahooFeed::new(
                    yahoo_tickers,
                )) as Box<dyn Feed>,
                Box::new(openintel::infrastructure::feeds::nws::NwsFeed::central_park()),
                Box::new(openintel::infrastructure::feeds::kalshi::KalshiFeed::default_series()),
            ])
        }
        other => Err(format!(
            "Unknown feed source: {other}. Use: yahoo, nws, kalshi, or all"
        )),
    }
}

async fn run_command(oi: OpenIntel, cmd: Commands) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Commands::Add { category, json } => {
            let cat: Category = category.parse().map_err(|e: String| e)?;
            let data: serde_json::Value = serde_json::from_str(&json)?;

            let title = data["title"]
                .as_str()
                .ok_or("Missing required field: title")?
                .to_string();
            let body = data["body"]
                .as_str()
                .ok_or("Missing required field: body")?
                .to_string();
            let source = data["source"].as_str().map(|s| s.to_string());
            let tags: Vec<String> = data["tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let confidence = data["confidence"].as_f64();
            let actionable = data["actionable"].as_bool();
            let source_type: SourceType = match data["source_type"].as_str() {
                Some(s) => s.parse().map_err(|e: String| e)?,
                None => SourceType::default(),
            };
            let skip_dedup = data["skip_dedup"].as_bool().unwrap_or(false);
            let metadata = data.get("metadata").cloned();

            let result = oi
                .add_intel(
                    cat,
                    title,
                    body,
                    source,
                    tags,
                    confidence,
                    actionable,
                    source_type,
                    metadata,
                    skip_dedup,
                )
                .await?;
            if result.deduplicated {
                eprintln!(
                    "⚠️  Duplicate detected — returning existing entry (id: {})",
                    result.entry.id
                );
            }
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
        }
        Commands::Query {
            category,
            limit,
            since,
            tag,
            from,
            to,
            last,
            exclude_internal,
            decay,
        } => {
            let cat: Category = category.parse().map_err(|e: String| e)?;
            let range = resolve_time_range(&from, &to, &last)?;
            let since_dt = range.since.or(parse_date_as_start(&since)?);
            let exclude = if exclude_internal {
                Some(SourceType::Internal)
            } else {
                None
            };
            let mut entries =
                oi.query(Some(cat), tag, since_dt, range.until, Some(limit), exclude)?;
            if decay {
                let now = chrono::Utc::now();
                entries.sort_by(|a, b| {
                    b.decayed_confidence_at(now)
                        .partial_cmp(&a.decayed_confidence_at(now))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            println!("{}", serde_json::to_string_pretty(&entries).unwrap());
        }
        Commands::Search {
            text,
            limit,
            from,
            to,
            last,
        } => {
            let DateRange { since, until } = resolve_time_range(&from, &to, &last)?;
            if since.is_some() || until.is_some() {
                let entries = oi.keyword_search_with_time(&text, limit, since, until)?;
                println!("{}", serde_json::to_string_pretty(&entries).unwrap());
            } else {
                let entries = oi.keyword_search(&text, limit)?;
                println!("{}", serde_json::to_string_pretty(&entries).unwrap());
            }
        }
        Commands::Semantic { query, limit } => {
            let entries = oi.semantic_search(&query, limit).await?;
            println!("{}", serde_json::to_string_pretty(&entries).unwrap());
        }
        Commands::Think { query, limit } => {
            let entries = oi.hybrid_search(&query, limit).await?;
            println!("{}", serde_json::to_string_pretty(&entries).unwrap());
        }
        Commands::Stats => {
            let stats = oi.stats()?;
            println!("{}", serde_json::to_string_pretty(&stats).unwrap());
        }
        Commands::Tags { category } => {
            let cat = category
                .map(|c| c.parse())
                .transpose()
                .map_err(|e: String| e)?;
            let tags = oi.tags(cat)?;
            for t in &tags {
                println!("{}: {}", t.tag, t.count);
            }
        }
        Commands::TradeAdd { json } => {
            let data: serde_json::Value = serde_json::from_str(&json)?;

            let ticker = data["ticker"]
                .as_str()
                .ok_or("ticker required")?
                .to_string();
            let series_ticker = data["series_ticker"].as_str().map(String::from);
            let direction: TradeDirection = data["direction"]
                .as_str()
                .ok_or("direction required")?
                .parse()
                .map_err(|e: String| e)?;
            let contracts = data["contracts"].as_i64().ok_or("contracts required")?;
            let entry_price = data["entry_price"].as_f64().ok_or("entry_price required")?;
            let thesis = data["thesis"].as_str().map(String::from);

            let trade = oi.trade_add(
                ticker,
                series_ticker,
                direction,
                contracts,
                entry_price,
                thesis,
            )?;
            println!("{}", serde_json::to_string_pretty(&trade).unwrap());
        }
        Commands::TradeResolve {
            id,
            outcome,
            pnl_cents,
            exit_price,
        } => {
            let out: TradeOutcome = outcome.parse().map_err(|e: String| e)?;
            oi.trade_resolve(&id, out, pnl_cents, exit_price)?;
            println!("Trade {id} resolved as {outcome} ({pnl_cents} cents)");
        }
        Commands::Trades {
            limit,
            since,
            resolved,
            from,
            to,
            last,
        } => {
            let range = resolve_time_range(&from, &to, &last)?;
            let since_dt = range.since.or(parse_date_as_start(&since)?);
            let trades = oi.trade_list(Some(limit), since_dt, range.until, resolved)?;
            println!("{}", serde_json::to_string_pretty(&trades).unwrap());
        }
        Commands::Export {
            since,
            category,
            from,
            to,
            last,
            exclude_internal,
        } => {
            let range = resolve_time_range(&from, &to, &last)?;
            let since_dt = range.since.or(parse_date_as_start(&since)?);
            let cat = category
                .map(|c| c.parse())
                .transpose()
                .map_err(|e: String| e)?;
            let exclude = if exclude_internal {
                Some(SourceType::Internal)
            } else {
                None
            };
            let entries = oi.query(cat, None, since_dt, range.until, None, exclude)?;
            println!("{}", serde_json::to_string_pretty(&entries).unwrap());
        }
        Commands::Summarize { hours } => {
            let summary = oi.summarize(hours)?;
            println!("{}", serde_json::to_string_pretty(&summary).unwrap());
        }
        Commands::Scan { hours } => {
            let scan = oi.scan_alerts(hours)?;
            println!("{}", serde_json::to_string_pretty(&scan).unwrap());
        }
        Commands::Pending => {
            let report = oi.pending_trades()?;
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        }
        Commands::Reindex => {
            let count = oi.reindex().await?;
            println!("Reindexed {count} entries");
        }
        Commands::Opportunities {
            hours,
            min_score,
            entry_limit,
            limit,
            bankroll,
            kelly_fraction,
            max_position,
        } => {
            let scan = if bankroll.is_some() || kelly_fraction.is_some() || max_position.is_some() {
                let mut config = openintel::domain::values::kelly::KellyConfig::default();
                if let Some(f) = kelly_fraction {
                    config.fraction = f.clamp(0.0, 1.0);
                }
                if let Some(m) = max_position {
                    config.max_position_cents = m;
                }
                oi.opportunities_with_sizing(
                    hours,
                    min_score,
                    entry_limit,
                    limit,
                    bankroll,
                    Some(config),
                )?
            } else {
                oi.opportunities(hours, min_score, entry_limit, limit)?
            };
            println!("{}", serde_json::to_string_pretty(&scan).unwrap());
        }
        Commands::Portfolio {
            positions_json,
            threshold,
        } => {
            let mut positions: Vec<Position> = serde_json::from_str(&positions_json)?;
            // Infer asset class from ticker only when not explicitly provided
            for pos in &mut positions {
                if pos.asset_class == AssetClass::Unknown {
                    pos.asset_class = AssetClass::from_ticker(&pos.ticker);
                }
            }
            let portfolio = Portfolio::from_positions(positions, threshold);
            println!("{}", serde_json::to_string_pretty(&portfolio).unwrap());
        }
        Commands::Feed { source, tickers } => {
            let ticker_list: Vec<String> = tickers
                .map(|t| t.split(',').map(|s| s.trim().to_uppercase()).collect())
                .unwrap_or_default();

            let feeds = initialize_feeds(&source, ticker_list)?;

            let mut results = Vec::new();
            for feed in feeds {
                let feed_name = feed.name().to_string();
                match feed.fetch().await {
                    Ok(FetchOutput {
                        entries,
                        fetch_errors,
                    }) => {
                        let fetched = entries.len() + fetch_errors.len();
                        let mut added = 0;
                        let mut deduped = 0;
                        let mut errors = fetch_errors;

                        for entry in entries {
                            match oi
                                .add_intel(
                                    entry.category,
                                    entry.title.clone(),
                                    entry.body.clone(),
                                    entry.source.clone(),
                                    entry.tags.clone(),
                                    Some(entry.confidence.value()),
                                    Some(entry.actionable),
                                    entry.source_type,
                                    entry.metadata.clone(),
                                    false, // allow dedup
                                )
                                .await
                            {
                                Ok(result) => {
                                    if result.deduplicated {
                                        deduped += 1;
                                    } else {
                                        added += 1;
                                    }
                                }
                                Err(e) => {
                                    errors.push(format!("{}: {e}", entry.title));
                                }
                            }
                        }

                        results.push(FeedResult {
                            feed_name,
                            entries_fetched: fetched,
                            entries_added: added,
                            entries_deduped: deduped,
                            errors,
                        });
                    }
                    Err(e) => {
                        results.push(FeedResult {
                            feed_name,
                            entries_fetched: 0,
                            entries_added: 0,
                            entries_deduped: 0,
                            errors: vec![e.to_string()],
                        });
                    }
                }
            }

            println!("{}", serde_json::to_string_pretty(&results).unwrap());
        }
        Commands::Execute {
            bankroll,
            dry_run,
            min_confidence,
            min_score,
            max_position,
            max_daily,
            kelly_fraction,
            hours,
            tickers,
        } => {
            use openintel::infrastructure::feeds::FetchOutput;

            // Validate numeric parameters
            if bankroll == 0 {
                return Err("--bankroll must be > 0".into());
            }
            if max_daily == 0 {
                return Err("--max-daily must be > 0".into());
            }
            if max_position == 0 {
                return Err("--max-position must be > 0".into());
            }
            let clamped_confidence = min_confidence.clamp(0.0, 1.0);
            if clamped_confidence != min_confidence {
                eprintln!(
                    "Warning: --min-confidence {min_confidence} out of range, clamped to {clamped_confidence}"
                );
            }
            let min_confidence = clamped_confidence;
            let clamped_score = min_score.max(0.0);
            if clamped_score != min_score {
                eprintln!(
                    "Warning: --min-score {min_score} out of range, clamped to {clamped_score}"
                );
            }
            let min_score = clamped_score;
            let clamped_kelly = kelly_fraction.clamp(0.001, 1.0);
            if clamped_kelly != kelly_fraction {
                eprintln!(
                    "Warning: --kelly-fraction {kelly_fraction} out of range, clamped to {clamped_kelly}"
                );
            }
            let kelly_fraction = clamped_kelly;

            if !dry_run {
                return Err(
                    "Live execution is not yet implemented. Use --dry-run true (default) to preview trade plans."
                        .into(),
                );
            }

            // Step 1: Run all feeds
            eprintln!("📡 Step 1: Ingesting live data feeds...");
            let ticker_list: Vec<String> = tickers
                .map(|t| t.split(',').map(|s| s.trim().to_uppercase()).collect())
                .unwrap_or_default();

            let feeds = initialize_feeds("all", ticker_list)?;

            let mut total_ingested = 0usize;
            let mut feed_errors = Vec::new();
            for feed in feeds {
                let feed_name = feed.name().to_string();
                match feed.fetch().await {
                    Ok(FetchOutput {
                        entries,
                        fetch_errors,
                    }) => {
                        if !fetch_errors.is_empty() {
                            feed_errors
                                .extend(fetch_errors.iter().map(|e| format!("{feed_name}: {e}")));
                        }
                        for entry in entries {
                            match oi
                                .add_intel(
                                    entry.category,
                                    entry.title.clone(),
                                    entry.body.clone(),
                                    entry.source.clone(),
                                    entry.tags.clone(),
                                    Some(entry.confidence.value()),
                                    Some(entry.actionable),
                                    entry.source_type,
                                    entry.metadata.clone(),
                                    false,
                                )
                                .await
                            {
                                Ok(result) => {
                                    if !result.deduplicated {
                                        total_ingested += 1;
                                    }
                                }
                                Err(e) => {
                                    feed_errors.push(format!("{feed_name}: {}: {e}", entry.title));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        feed_errors.push(format!("{feed_name}: {e}"));
                    }
                }
            }
            eprintln!(
                "   ✅ Ingested {total_ingested} new entries ({})",
                if feed_errors.is_empty() {
                    "no errors".to_string()
                } else {
                    format!("{} errors", feed_errors.len())
                }
            );

            // Step 2: Run opportunity detection (without Kelly — prices not yet resolved)
            eprintln!("🔍 Step 2: Scanning for opportunities...");
            let mut scan = oi.opportunities(hours, None, None, None)?;

            eprintln!(
                "   ✅ Found {} opportunities from {} entries",
                scan.total_opportunities, scan.entries_scanned
            );

            // Step 2b: Resolve market prices from intel DB
            eprintln!("💱 Step 2b: Resolving market prices...");
            let resolver = openintel::infrastructure::resolvers::intel_resolver::IntelResolver::new(
                oi.intel_repo(),
            );
            use openintel::domain::ports::market_resolver::{Exchange, MarketResolver};
            let kelly_config = openintel::domain::values::kelly::KellyConfig {
                fraction: kelly_fraction,
                max_position_cents: max_position,
                ..Default::default()
            };
            let mut resolved_count = 0usize;
            let mut unresolved_count = 0usize;
            for opp in &mut scan.opportunities {
                if opp.market_price.is_some() {
                    continue; // already has a price
                }
                if let Some(ticker) = &opp.market_ticker.clone() {
                    if let Some(resolved) = resolver.resolve(ticker).await {
                        opp.market_price = Some(resolved.price_cents);
                        // Only apply Kelly sizing to Kalshi binary contracts (1–99¢).
                        // Equity prices are in dollar-cents and use different sizing logic.
                        if resolved.exchange == Exchange::Kalshi {
                            if let Some(sizing) = openintel::domain::values::kelly::compute_kelly(
                                opp.confidence,
                                resolved.price_cents,
                                bankroll,
                                &kelly_config,
                            ) {
                                if sizing.suggested_size_cents > 0 {
                                    opp.suggested_size_cents = Some(sizing.suggested_size_cents);
                                }
                            }
                        }
                        resolved_count += 1;
                    } else {
                        unresolved_count += 1;
                    }
                }
            }
            eprintln!(
                "   ✅ Resolved {resolved_count} market prices ({unresolved_count} unresolved)"
            );

            // Step 3: Filter by confidence, score, and build trade plan
            eprintln!("📋 Step 3: Building trade plan...");

            let mut trades: Vec<TradePlan> = Vec::new();
            let mut skipped: Vec<SkippedOpportunity> = Vec::new();
            let mut daily_deployed: u64 = 0;

            for opp in &scan.opportunities {
                // Filter: minimum score (#4 — now visible in skipped list)
                if opp.score < min_score {
                    skipped.push(SkippedOpportunity {
                        title: opp.title.clone(),
                        confidence: opp.confidence,
                        score: opp.score,
                        reason: format!("Score {:.2} < {:.2} threshold", opp.score, min_score),
                    });
                    continue;
                }

                // Filter: must have market ticker
                let ticker = match &opp.market_ticker {
                    Some(t) => t.clone(),
                    None => {
                        skipped.push(SkippedOpportunity {
                            title: opp.title.clone(),
                            confidence: opp.confidence,
                            score: opp.score,
                            reason: "No market ticker".to_string(),
                        });
                        continue;
                    }
                };

                // Filter: minimum confidence
                if opp.confidence < min_confidence {
                    skipped.push(SkippedOpportunity {
                        title: opp.title.clone(),
                        confidence: opp.confidence,
                        score: opp.score,
                        reason: format!(
                            "Confidence {:.0}% < {:.0}% threshold",
                            opp.confidence * 100.0,
                            min_confidence * 100.0
                        ),
                    });
                    continue;
                }

                // Get Kelly-sized position (#3 — Kelly already caps via KellyConfig)
                let size = match opp.suggested_size_cents {
                    Some(s) => s,
                    None => {
                        skipped.push(SkippedOpportunity {
                            title: opp.title.clone(),
                            confidence: opp.confidence,
                            score: opp.score,
                            reason: "No Kelly sizing available (missing market price)".to_string(),
                        });
                        continue;
                    }
                };

                if size == 0 {
                    skipped.push(SkippedOpportunity {
                        title: opp.title.clone(),
                        confidence: opp.confidence,
                        score: opp.score,
                        reason: "Kelly sizing returned 0 (no edge)".to_string(),
                    });
                    continue;
                }

                // Check daily deployment limit
                if daily_deployed + size > max_daily {
                    skipped.push(SkippedOpportunity {
                        title: opp.title.clone(),
                        confidence: opp.confidence,
                        score: opp.score,
                        reason: format!(
                            "Daily limit: ${:.2} deployed + ${:.2} would exceed ${:.2} cap",
                            daily_deployed as f64 / 100.0,
                            size as f64 / 100.0,
                            max_daily as f64 / 100.0
                        ),
                    });
                    continue;
                }

                let direction = opp
                    .suggested_direction
                    .as_ref()
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let action = opp.suggested_action.clone().unwrap_or_else(|| {
                    format!("Buy {} @ {}¢", ticker, opp.market_price.unwrap_or(0.0))
                });

                trades.push(TradePlan {
                    ticker,
                    direction,
                    size_cents: size,
                    confidence: opp.confidence,
                    score: opp.score,
                    edge_cents: opp.edge_cents,
                    action,
                    description: opp.description.clone(),
                });
                daily_deployed += size;
            }

            // Step 4: Output results
            // Note: live mode returns early above; this will need updating
            // when live execution is implemented.
            let execution_mode = ExecutionMode::DryRun;

            let result = ExecutionResult {
                timestamp: chrono::Utc::now().to_rfc3339(),
                mode: execution_mode,
                bankroll_cents: bankroll,
                feeds_ingested: total_ingested,
                feed_errors,
                opportunities_scanned: scan.total_opportunities,
                trades_qualified: trades.len(),
                trades_skipped: skipped.len(),
                total_deployment_cents: daily_deployed,
                trades,
                skipped,
            };

            eprintln!(
                "   🏁 DRY RUN: {} trades qualified, ${:.2} total deployment",
                result.trades_qualified,
                daily_deployed as f64 / 100.0
            );

            println!("{}", serde_json::to_string_pretty(&result).unwrap());
        }
        Commands::Kelly {
            probability,
            market_price,
            bankroll,
            kelly_fraction,
            max_position,
        } => {
            let mut config = openintel::domain::values::kelly::KellyConfig::default();
            if let Some(f) = kelly_fraction {
                config.fraction = f.clamp(0.0, 1.0);
            }
            if let Some(m) = max_position {
                config.max_position_cents = m;
            }
            match openintel::domain::values::kelly::compute_kelly(
                probability,
                market_price,
                bankroll,
                &config,
            ) {
                Some(sizing) => {
                    println!("{}", serde_json::to_string_pretty(&sizing).unwrap());
                }
                None => {
                    return Err("Invalid inputs: probability must be (0,1), market_price (0,100), bankroll > 0".into());
                }
            }
        }
    }
    Ok(())
}

fn parse_duration(s: &str) -> Result<chrono::Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("Empty duration string".into());
    }
    let (num_str, unit) = if let Some(n) = s.strip_suffix('h') {
        (n, 'h')
    } else if let Some(n) = s.strip_suffix('d') {
        (n, 'd')
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 'm')
    } else if let Some(n) = s.strip_suffix('w') {
        (n, 'w')
    } else {
        return Err(format!(
            "Invalid duration '{s}'. Use format like 24h, 7d, 30m, 2w"
        ));
    };
    let num: i64 = num_str
        .parse()
        .map_err(|_| format!("Invalid number in duration: {num_str}"))?;
    if num <= 0 {
        return Err(format!("Duration must be positive, got {num}"));
    }
    match unit {
        'm' => Ok(chrono::Duration::minutes(num)),
        'h' => Ok(chrono::Duration::hours(num)),
        'd' => Ok(chrono::Duration::days(num)),
        'w' => Ok(chrono::Duration::weeks(num)),
        _ => unreachable!(),
    }
}

/// A named date range with explicit since/until fields.
struct DateRange {
    since: Option<chrono::DateTime<chrono::Utc>>,
    until: Option<chrono::DateTime<chrono::Utc>>,
}

/// Resolve --from/--to/--last into a DateRange.
/// --last and --from are mutually exclusive (enforced by clap).
fn resolve_time_range(
    from: &Option<String>,
    to: &Option<String>,
    last: &Option<String>,
) -> Result<DateRange, String> {
    let since = if let Some(last_str) = last {
        let dur = parse_duration(last_str)?;
        Some(chrono::Utc::now() - dur)
    } else {
        parse_date_as_start(from)?
    };
    let until = parse_date_as_end(to)?;
    Ok(DateRange { since, until })
}

/// Parse a date string as a lower bound (start of day for YYYY-MM-DD).
fn parse_date_as_start(
    s: &Option<String>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, String> {
    parse_date_inner(s, false)
}

/// Parse a date string as an upper bound (end of day for YYYY-MM-DD).
fn parse_date_as_end(s: &Option<String>) -> Result<Option<chrono::DateTime<chrono::Utc>>, String> {
    parse_date_inner(s, true)
}

fn parse_date_inner(
    s: &Option<String>,
    end_of_day: bool,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, String> {
    match s {
        None => Ok(None),
        Some(s) => {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                return Ok(Some(dt.with_timezone(&chrono::Utc)));
            }
            if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                let dt = if end_of_day {
                    date.and_hms_opt(23, 59, 59).unwrap()
                } else {
                    date.and_hms_opt(0, 0, 0).unwrap()
                };
                return Ok(Some(chrono::DateTime::from_naive_utc_and_offset(
                    dt,
                    chrono::Utc,
                )));
            }
            Err(format!(
                "Invalid date format: {s}. Use YYYY-MM-DD or RFC3339"
            ))
        }
    }
}
