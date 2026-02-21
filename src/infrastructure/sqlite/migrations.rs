use rusqlite::Connection;

pub fn run_migrations(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS intel_entries (
            id TEXT PRIMARY KEY,
            category TEXT NOT NULL,
            title TEXT NOT NULL,
            body TEXT NOT NULL,
            source TEXT,
            tags TEXT NOT NULL DEFAULT '[]',
            confidence REAL NOT NULL DEFAULT 0.5,
            actionable INTEGER NOT NULL DEFAULT 0,
            metadata TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS trades (
            id TEXT PRIMARY KEY,
            ticker TEXT NOT NULL,
            series_ticker TEXT,
            direction TEXT NOT NULL,
            contracts INTEGER NOT NULL,
            entry_price REAL NOT NULL,
            exit_price REAL,
            thesis TEXT,
            outcome TEXT,
            pnl_cents INTEGER,
            created_at TEXT NOT NULL,
            resolved_at TEXT
        );

        CREATE TABLE IF NOT EXISTS vectors (
            id TEXT PRIMARY KEY,
            vector BLOB NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_intel_category ON intel_entries(category);
        CREATE INDEX IF NOT EXISTS idx_intel_created ON intel_entries(created_at);
        CREATE INDEX IF NOT EXISTS idx_trades_created ON trades(created_at);
        CREATE INDEX IF NOT EXISTS idx_trades_ticker ON trades(ticker);
        "
    ).map_err(|e| format!("Migration failed: {e}"))
}
