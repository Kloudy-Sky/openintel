use crate::domain::ports::vector_store::VectorStore;
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub struct SqliteVectorStore {
    conn: Mutex<Connection>,
}

impl SqliteVectorStore {
    pub fn new(conn: Connection) -> Self {
        Self { conn: Mutex::new(conn) }
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }
        let mut dot = 0.0_f64;
        let mut norm_a = 0.0_f64;
        let mut norm_b = 0.0_f64;
        for (x, y) in a.iter().zip(b.iter()) {
            let x = *x as f64;
            let y = *y as f64;
            dot += x * y;
            norm_a += x * x;
            norm_b += y * y;
        }
        let denom = norm_a.sqrt() * norm_b.sqrt();
        if denom == 0.0 { 0.0 } else { dot / denom }
    }

    fn serialize_vector(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    fn deserialize_vector(bytes: &[u8]) -> Vec<f32> {
        bytes.chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    }
}

impl VectorStore for SqliteVectorStore {
    fn store(&self, id: &str, vector: &[f32]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let blob = Self::serialize_vector(vector);
        conn.execute(
            "INSERT OR REPLACE INTO vectors (id, vector) VALUES (?1, ?2)",
            params![id, blob],
        ).map_err(|e| format!("Failed to store vector: {e}"))?;
        Ok(())
    }

    fn search_similar(&self, vector: &[f32], limit: usize) -> Result<Vec<(String, f64)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT id, vector FROM vectors")
            .map_err(|e| e.to_string())?;
        let mut results: Vec<(String, f64)> = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((id, blob))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .map(|(id, blob)| {
            let stored = Self::deserialize_vector(&blob);
            let sim = Self::cosine_similarity(vector, &stored);
            (id, sim)
        })
        .collect();

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        Ok(results)
    }

    fn has_vector(&self, id: &str) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM vectors WHERE id = ?1",
            params![id],
            |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        Ok(count > 0)
    }
}
