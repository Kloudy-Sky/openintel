use crate::domain::entities::intel_entry::IntelEntry;
use crate::domain::ports::intel_repository::*;
use crate::domain::values::category::Category;
use crate::domain::values::confidence::Confidence;
use chrono::DateTime;
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub struct SqliteIntelRepo {
    conn: Mutex<Connection>,
}

impl SqliteIntelRepo {
    pub fn new(conn: Connection) -> Self {
        Self { conn: Mutex::new(conn) }
    }

    fn row_to_entry(row: &rusqlite::Row) -> Result<IntelEntry, rusqlite::Error> {
        let cat_str: String = row.get(1)?;
        let tags_str: String = row.get(5)?;
        let conf_val: f64 = row.get(6)?;
        let actionable_int: i32 = row.get(7)?;
        let metadata_str: Option<String> = row.get(8)?;
        let created_str: String = row.get(9)?;
        let updated_str: String = row.get(10)?;

        Ok(IntelEntry {
            id: row.get(0)?,
            category: cat_str.parse().unwrap_or(Category::General),
            title: row.get(2)?,
            body: row.get(3)?,
            source: row.get(4)?,
            tags: serde_json::from_str(&tags_str).unwrap_or_default(),
            confidence: Confidence::new(conf_val).unwrap_or_default(),
            actionable: actionable_int != 0,
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
    fn add(&self, entry: &IntelEntry) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO intel_entries (id, category, title, body, source, tags, confidence, actionable, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            ],
        ).map_err(|e| format!("Failed to add entry: {e}"))?;
        Ok(())
    }

    fn query(&self, filter: &QueryFilter) -> Result<Vec<IntelEntry>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut sql = String::from("SELECT id, category, title, body, source, tags, confidence, actionable, metadata, created_at, updated_at FROM intel_entries WHERE 1=1");
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(cat) = &filter.category {
            sql.push_str(&format!(" AND category = ?{}", param_values.len() + 1));
            param_values.push(Box::new(cat.to_string()));
        }
        if let Some(since) = &filter.since {
            sql.push_str(&format!(" AND created_at >= ?{}", param_values.len() + 1));
            param_values.push(Box::new(since.to_rfc3339()));
        }
        if let Some(tag) = &filter.tag {
            sql.push_str(&format!(" AND tags LIKE ?{}", param_values.len() + 1));
            param_values.push(Box::new(format!("%\"{tag}\"%")));
        }

        sql.push_str(" ORDER BY created_at DESC");
        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let params_refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let entries = stmt.query_map(params_refs.as_slice(), Self::row_to_entry)
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    fn search(&self, text: &str, limit: usize) -> Result<Vec<IntelEntry>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let pattern = format!("%{text}%");
        let mut stmt = conn.prepare(
            "SELECT id, category, title, body, source, tags, confidence, actionable, metadata, created_at, updated_at
             FROM intel_entries WHERE title LIKE ?1 OR body LIKE ?1
             ORDER BY created_at DESC LIMIT ?2"
        ).map_err(|e| e.to_string())?;
        let entries = stmt.query_map(params![pattern, limit as i64], Self::row_to_entry)
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    fn get_by_id(&self, id: &str) -> Result<Option<IntelEntry>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id, category, title, body, source, tags, confidence, actionable, metadata, created_at, updated_at
             FROM intel_entries WHERE id = ?1"
        ).map_err(|e| e.to_string())?;
        let mut rows = stmt.query_map(params![id], Self::row_to_entry).map_err(|e| e.to_string())?;
        Ok(rows.next().and_then(|r| r.ok()))
    }

    fn export(&self, filter: &QueryFilter) -> Result<Vec<IntelEntry>, String> {
        self.query(filter)
    }

    fn stats(&self) -> Result<IntelStats, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let total: usize = conn.query_row("SELECT COUNT(*) FROM intel_entries", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let actionable: usize = conn.query_row("SELECT COUNT(*) FROM intel_entries WHERE actionable = 1", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;

        let mut stmt = conn.prepare("SELECT category, COUNT(*) FROM intel_entries GROUP BY category")
            .map_err(|e| e.to_string())?;
        let by_category: Vec<(String, usize)> = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        let total_tags: usize = conn.query_row(
            "SELECT COUNT(DISTINCT value) FROM intel_entries, json_each(tags)",
            [], |r| r.get(0)
        ).unwrap_or(0);

        Ok(IntelStats { total_entries: total, by_category, total_tags, actionable_count: actionable })
    }

    fn tags(&self, category: Option<Category>) -> Result<Vec<TagCount>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(cat) = category {
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
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let tags = stmt.query_map(params_refs.as_slice(), |row| {
            Ok(TagCount { tag: row.get(0)?, count: row.get(1)? })
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
        Ok(tags)
    }

    fn entries_missing_vectors(&self) -> Result<Vec<IntelEntry>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id, category, title, body, source, tags, confidence, actionable, metadata, created_at, updated_at
             FROM intel_entries WHERE id NOT IN (SELECT id FROM vectors)"
        ).map_err(|e| e.to_string())?;
        let entries = stmt.query_map([], Self::row_to_entry)
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }
}
