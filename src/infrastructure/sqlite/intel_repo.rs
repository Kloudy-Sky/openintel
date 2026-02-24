use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::error::DomainError;
use crate::domain::ports::intel_repository::*;
use crate::domain::values::category::Category;
use crate::domain::values::confidence::Confidence;
use crate::domain::values::source_type::SourceType;
use chrono::DateTime;
use rusqlite::{params, Connection};
use std::sync::Mutex;

/// Column list used in all SELECT queries. source_type is added via ALTER TABLE
/// so it appears after the original columns.
const SELECT_COLS: &str = "id, category, title, body, source, tags, confidence, actionable, metadata, created_at, updated_at, source_type";

pub struct SqliteIntelRepo {
    conn: Mutex<Connection>,
}

impl SqliteIntelRepo {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
        }
    }

    fn row_to_entry(row: &rusqlite::Row) -> Result<IntelEntry, rusqlite::Error> {
        let cat_str: String = row.get(1)?;
        let tags_str: String = row.get(5)?;
        let conf_val: f64 = row.get(6)?;
        let actionable_int: i32 = row.get(7)?;
        let metadata_str: Option<String> = row.get(8)?;
        let created_str: String = row.get(9)?;
        let updated_str: String = row.get(10)?;
        let source_type_str: String = row.get(11)?;

        Ok(IntelEntry {
            id: row.get(0)?,
            category: cat_str
                .parse()
                .map_err(|_| {
                    eprintln!(
                        "Warning: invalid category '{}' in entry, defaulting to General",
                        cat_str
                    );
                    rusqlite::Error::InvalidParameterName(cat_str.clone())
                })
                .unwrap_or(Category::General),
            title: row.get(2)?,
            body: row.get(3)?,
            source: row.get(4)?,
            tags: serde_json::from_str(&tags_str).unwrap_or_default(),
            confidence: Confidence::new(conf_val).unwrap_or_default(),
            actionable: actionable_int != 0,
            source_type: source_type_str.parse().unwrap_or_else(|_| {
                eprintln!(
                    "Warning: invalid source_type '{}' in entry, defaulting to External",
                    source_type_str
                );
                SourceType::default()
            }),
            metadata: metadata_str.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: DateTime::parse_from_rfc3339(&created_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&updated_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
        })
    }
}

impl IntelRepository for SqliteIntelRepo {
    fn add(&self, entry: &IntelEntry) -> Result<(), DomainError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DomainError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO intel_entries (id, category, title, body, source, tags, confidence, actionable, metadata, created_at, updated_at, source_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                entry.id,
                entry.category.to_string(),
                entry.title,
                entry.body,
                entry.source,
                serde_json::to_string(&entry.tags).unwrap_or_default(),
                entry.confidence.value(),
                entry.actionable as i32,
                entry.metadata.as_ref().map(|m| serde_json::to_string(m).unwrap_or_default()),
                entry.created_at.to_rfc3339(),
                entry.updated_at.to_rfc3339(),
                entry.source_type.to_string(),
            ],
        ).map_err(|e| DomainError::Database(format!("Failed to add entry: {e}")))?;
        Ok(())
    }

    fn query(&self, filter: &QueryFilter) -> Result<Vec<IntelEntry>, DomainError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let mut sql = format!("SELECT {} FROM intel_entries WHERE 1=1", SELECT_COLS);
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(cat) = &filter.category {
            sql.push_str(&format!(" AND category = ?{}", param_values.len() + 1));
            param_values.push(Box::new(cat.to_string()));
        }
        if let Some(since) = &filter.since {
            sql.push_str(&format!(" AND created_at >= ?{}", param_values.len() + 1));
            param_values.push(Box::new(since.to_rfc3339()));
        }
        if let Some(until) = &filter.until {
            sql.push_str(&format!(" AND created_at <= ?{}", param_values.len() + 1));
            param_values.push(Box::new(until.to_rfc3339()));
        }
        if let Some(tag) = &filter.tag {
            sql.push_str(&format!(
                " AND tags LIKE ?{} ESCAPE '\\'",
                param_values.len() + 1
            ));
            let escaped_tag = tag
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            param_values.push(Box::new(format!("%\"{escaped_tag}\"%")));
        }
        if let Some(exclude) = &filter.exclude_source_type {
            sql.push_str(&format!(" AND source_type != ?{}", param_values.len() + 1));
            param_values.push(Box::new(exclude.to_string()));
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
        let entries = stmt
            .query_map(params_refs.as_slice(), Self::row_to_entry)
            .map_err(|e| DomainError::Database(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    fn search_with_time(
        &self,
        text: &str,
        limit: usize,
        since: Option<DateTime<chrono::Utc>>,
        until: Option<DateTime<chrono::Utc>>,
    ) -> Result<Vec<IntelEntry>, DomainError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let pattern = format!("%{text}%");
        let mut sql = String::from(
            "SELECT id, category, title, body, source, tags, confidence, actionable, metadata, created_at, updated_at, source_type FROM intel_entries WHERE (title LIKE ?1 OR body LIKE ?1)",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        param_values.push(Box::new(pattern));

        if let Some(since) = &since {
            sql.push_str(&format!(" AND created_at >= ?{}", param_values.len() + 1));
            param_values.push(Box::new(since.to_rfc3339()));
        }
        if let Some(until) = &until {
            sql.push_str(&format!(" AND created_at <= ?{}", param_values.len() + 1));
            param_values.push(Box::new(until.to_rfc3339()));
        }

        sql.push_str(&format!(
            " ORDER BY created_at DESC LIMIT ?{}",
            param_values.len() + 1
        ));
        param_values.push(Box::new(limit as i64));

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let entries = stmt
            .query_map(params_refs.as_slice(), Self::row_to_entry)
            .map_err(|e| DomainError::Database(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    fn get_by_id(&self, id: &str) -> Result<Option<IntelEntry>, DomainError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let sql = format!("SELECT {} FROM intel_entries WHERE id = ?1", SELECT_COLS);
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let mut rows = stmt
            .query_map(params![id], Self::row_to_entry)
            .map_err(|e| DomainError::Database(e.to_string()))?;
        Ok(rows.next().and_then(|r| r.ok()))
    }

    fn stats(&self) -> Result<IntelStats, DomainError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let total: usize = conn
            .query_row("SELECT COUNT(*) FROM intel_entries", [], |r| r.get(0))
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let actionable: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM intel_entries WHERE actionable = 1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| DomainError::Database(e.to_string()))?;

        let mut stmt = conn
            .prepare("SELECT category, COUNT(*) FROM intel_entries GROUP BY category")
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let by_category: Vec<(String, usize)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
            })
            .map_err(|e| DomainError::Database(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        let total_tags: usize = conn
            .query_row(
                "SELECT COUNT(DISTINCT value) FROM intel_entries, json_each(tags)",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        Ok(IntelStats {
            total_entries: total,
            by_category,
            total_tags,
            actionable_count: actionable,
        })
    }

    fn tags(&self, category: Option<Category>) -> Result<Vec<TagCount>, DomainError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(cat) =
            category
        {
            (
                "SELECT value, COUNT(*) as cnt FROM intel_entries, json_each(tags) WHERE category = ?1 GROUP BY value ORDER BY cnt DESC".into(),
                vec![Box::new(cat.to_string()) as Box<dyn rusqlite::types::ToSql>],
            )
        } else {
            (
                "SELECT value, COUNT(*) as cnt FROM intel_entries, json_each(tags) GROUP BY value ORDER BY cnt DESC".into(),
                vec![],
            )
        };
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let tags = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(TagCount {
                    tag: row.get(0)?,
                    count: row.get(1)?,
                })
            })
            .map_err(|e| DomainError::Database(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tags)
    }

    fn entries_missing_vectors(&self) -> Result<Vec<IntelEntry>, DomainError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let sql = format!(
            "SELECT {} FROM intel_entries WHERE id NOT IN (SELECT id FROM vectors)",
            SELECT_COLS
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let entries = stmt
            .query_map([], Self::row_to_entry)
            .map_err(|e| DomainError::Database(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    fn find_duplicate(
        &self,
        category: &Category,
        title: &str,
        window: chrono::Duration,
    ) -> Result<Option<IntelEntry>, DomainError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let since = (chrono::Utc::now() - window).to_rfc3339();
        // Normalize: case-insensitive exact match on title within the time window and category.
        let sql = format!(
            "SELECT {} FROM intel_entries WHERE category = ?1 AND LOWER(title) = LOWER(?2) AND created_at >= ?3 LIMIT 1",
            SELECT_COLS
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| DomainError::Database(e.to_string()))?;
        let mut rows = stmt
            .query_map(
                params![category.to_string(), title, since],
                Self::row_to_entry,
            )
            .map_err(|e| DomainError::Database(e.to_string()))?;
        Ok(rows.next().and_then(|r| r.ok()))
    }
}
