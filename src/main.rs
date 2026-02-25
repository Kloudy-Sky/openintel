use clap::Parser;
use openintel::cli::commands::{Cli, Commands};
use openintel::domain::values::category::Category;
use openintel::domain::values::source_type::SourceType;
use openintel::domain::values::trade_direction::TradeDirection;
use openintel::domain::values::trade_outcome::TradeOutcome;
use openintel::OpenIntel;

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
        Commands::Opportunities { hours, min_score } => {
            let scan = oi.opportunities(hours, min_score)?;
            println!("{}", serde_json::to_string_pretty(&scan).unwrap());
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
