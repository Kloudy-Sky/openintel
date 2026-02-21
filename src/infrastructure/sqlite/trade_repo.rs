use crate::domain::entities::trade::Trade;
use crate::domain::error::DomainError;
use crate::domain::ports::trade_repository::*;
use crate::domain::values::trade_direction::TradeDirection;
use crate::domain::values::trade_outcome::TradeOutcome;
use chrono::DateTime;
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub struct SqliteTradeRepo {
    conn: Mutex<Connection>,
}

impl SqliteTradeRepo {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
        }
    }

    fn row_to_trade(row: &rusqlite::Row) -> Result<Trade, rusqlite::Error> {
        let dir_str: String = row.get(3)?;
        let outcome_str: Option<String> = row.get(8)?;
        let created_str: String = row.get(10)?;
        let resolved_str: Option<String> = row.get(11)?;

        Ok(Trade {
            id: row.get(0)?,
            ticker: row.get(1)?,
            series_ticker: row.get(2)?,
            direction: dir_str
                .parse()
                .map_err(|_| {
                    eprintln!(
                        "Warning: invalid direction '{}' in trade, defaulting to Long",
                        dir_str
                    );
                    rusqlite::Error::InvalidParameterName(dir_str.clone())
                })
                .unwrap_or(TradeDirection::Long),
            contracts: row.get(4)?,
            entry_price: row.get(5)?,
            exit_price: row.get(6)?,
            thesis: row.get(7)?,
            outcome: outcome_str.and_then(|s| s.parse().ok()),
            pnl_cents: row.get(9)?,
            created_at: DateTime::parse_from_rfc3339(&created_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
            resolved_at: resolved_str.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&chrono::Utc))
            }),
        })
    }
}

impl TradeRepository for SqliteTradeRepo {
    fn add_trade(&self, trade: &Trade) -> Result<(), DomainError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DomainError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO trades (id, ticker, series_ticker, direction, contracts, entry_price, exit_price, thesis, outcome, pnl_cents, created_at, resolved_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                trade.id,
                trade.ticker,
                trade.series_ticker,
                trade.direction.to_string(),
                trade.contracts,
                trade.entry_price,
                trade.exit_price,
                trade.thesis,
                trade.outcome.map(|o| o.to_string()),
                trade.pnl_cents,
                trade.created_at.to_rfc3339(),
                trade.resolved_at.map(|dt| dt.to_rfc3339()),
            ],
        ).map_err(|e| DomainError::Database(format!("Failed to add trade: {e}")))?;
        Ok(())
    }

    fn resolve_trade(
        &self,
        id: &str,
        outcome: TradeOutcome,
        pnl_cents: i64,
        exit_price: Option<f64>,
    ) -> Result<(), DomainError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let rows = conn.execute(
            "UPDATE trades SET outcome = ?1, pnl_cents = ?2, resolved_at = ?3, exit_price = ?4 WHERE id = ?5",
            params![outcome.to_string(), pnl_cents, chrono::Utc::now().to_rfc3339(), exit_price, id],
        ).map_err(|e| DomainError::Database(format!("Failed to resolve trade: {e}")))?;
        if rows == 0 {
            return Err(DomainError::NotFound(format!("Trade not found: {id}")));
        }
        Ok(())
    }

    fn list_trades(&self, filter: &TradeFilter) -> Result<Vec<Trade>, DomainError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let mut sql = String::from("SELECT id, ticker, series_ticker, direction, contracts, entry_price, exit_price, thesis, outcome, pnl_cents, created_at, resolved_at FROM trades WHERE 1=1");
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(since) = &filter.since {
            sql.push_str(&format!(" AND created_at >= ?{}", param_values.len() + 1));
            param_values.push(Box::new(since.to_rfc3339()));
        }
        if let Some(resolved) = filter.resolved {
            if resolved {
                sql.push_str(" AND outcome IS NOT NULL");
            } else {
                sql.push_str(" AND outcome IS NULL");
            }
        }
        sql.push_str(" ORDER BY created_at DESC");
        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT ?{}", param_values.len() + 1));
            param_values.push(Box::new(limit as i64));
        }

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let trades = stmt
            .query_map(params_refs.as_slice(), Self::row_to_trade)
            .map_err(|e| DomainError::Database(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(trades)
    }

    fn get_trade(&self, id: &str) -> Result<Option<Trade>, DomainError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT id, ticker, series_ticker, direction, contracts, entry_price, exit_price, thesis, outcome, pnl_cents, created_at, resolved_at FROM trades WHERE id = ?1"
        ).map_err(|e| DomainError::Database(e.to_string()))?;
        let mut rows = stmt
            .query_map(params![id], Self::row_to_trade)
            .map_err(|e| DomainError::Database(e.to_string()))?;
        Ok(rows.next().and_then(|r| r.ok()))
    }
}
