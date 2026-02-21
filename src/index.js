/**
 * OpenIntel — Structured intelligence knowledge base with hybrid semantic + keyword search.
 * 
 * @module openintel
 */

const Database = require('better-sqlite3');
const path = require('path');

const DEFAULT_EMBEDDING_DIM = 1024;

class OpenIntel {
  /**
   * Create or open an OpenIntel database.
   * @param {Object} options
   * @param {string} options.dbPath - Path to the SQLite database file
   * @param {Object} [options.embedding] - Embedding configuration
   * @param {string} [options.embedding.provider] - 'voyage', 'openai', 'cohere', or 'custom'
   * @param {string} [options.embedding.apiKey] - API key for the provider
   * @param {string} [options.embedding.apiUrl] - Custom API URL (default per provider)
   * @param {string} [options.embedding.model] - Model name
   * @param {number} [options.embedding.dimensions] - Embedding dimensions (default: 1024)
   * @param {Function} [options.embedding.embedFn] - Custom embed function (for provider='custom')
   * @param {string} [options.vecExtPath] - Path to sqlite-vec extension (.so/.dylib/.dll)
   */
  constructor(options = {}) {
    this.dbPath = options.dbPath || path.join(process.cwd(), 'openintel.db');
    this.embeddingConfig = options.embedding || null;
    this.dimensions = options.embedding?.dimensions || DEFAULT_EMBEDDING_DIM;
    this.vecExtPath = options.vecExtPath || null;
    this.db = null;
    this._init();
  }

  _init() {
    this.db = new Database(this.dbPath);
    this.db.pragma('journal_mode = WAL');
    this.db.pragma('foreign_keys = ON');

    // Try to load sqlite-vec
    this._vecLoaded = false;
    if (this.vecExtPath) {
      try {
        this.db.loadExtension(this.vecExtPath);
        this._vecLoaded = true;
      } catch (e) {
        // sqlite-vec not available — keyword search only
      }
    } else {
      // Try common paths
      const commonPaths = [
        // npm installed
        path.join(__dirname, '..', 'node_modules', 'sqlite-vec-linux-x64', 'vec0.so'),
        path.join(__dirname, '..', 'node_modules', 'sqlite-vec-darwin-x64', 'vec0.dylib'),
        path.join(__dirname, '..', 'node_modules', 'sqlite-vec-darwin-arm64', 'vec0.dylib'),
        // pnpm
        '/app/node_modules/.pnpm/sqlite-vec-linux-x64@0.1.7-alpha.2/node_modules/sqlite-vec-linux-x64/vec0.so',
      ];
      for (const p of commonPaths) {
        try {
          this.db.loadExtension(p);
          this._vecLoaded = true;
          break;
        } catch { /* continue */ }
      }
    }

    // Create tables
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS intel (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        category TEXT NOT NULL,
        title TEXT NOT NULL,
        body TEXT,
        source TEXT,
        tags TEXT DEFAULT '[]',
        confidence REAL DEFAULT 0.5,
        actionable INTEGER DEFAULT 0,
        metadata TEXT DEFAULT '{}',
        created_at TEXT DEFAULT (datetime('now')),
        expires_at TEXT
      );

      CREATE INDEX IF NOT EXISTS idx_intel_category ON intel(category);
      CREATE INDEX IF NOT EXISTS idx_intel_created ON intel(created_at);
      CREATE INDEX IF NOT EXISTS idx_intel_actionable ON intel(actionable);

      CREATE TABLE IF NOT EXISTS trades (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        ticker TEXT NOT NULL,
        series_ticker TEXT,
        direction TEXT NOT NULL,
        contracts INTEGER NOT NULL,
        entry_price REAL NOT NULL,
        exit_price REAL,
        thesis TEXT,
        outcome TEXT,
        pnl_cents INTEGER,
        placed_at TEXT DEFAULT (datetime('now')),
        resolved_at TEXT,
        metadata TEXT DEFAULT '{}'
      );

      CREATE INDEX IF NOT EXISTS idx_trades_ticker ON trades(ticker);
      CREATE INDEX IF NOT EXISTS idx_trades_placed ON trades(placed_at);
    `);

    // Create vector table if extension loaded
    if (this._vecLoaded) {
      try {
        this.db.exec(`
          CREATE VIRTUAL TABLE IF NOT EXISTS intel_vec USING vec0(
            intel_id INTEGER PRIMARY KEY,
            embedding float[${this.dimensions}]
          );
        `);
      } catch (e) {
        this._vecLoaded = false;
      }
    }
  }

  // --- Embedding ---

  async _embed(texts, inputType = 'document') {
    if (!this.embeddingConfig) throw new Error('No embedding provider configured');

    const { provider, apiKey, apiUrl, model, embedFn } = this.embeddingConfig;

    if (provider === 'custom' && embedFn) {
      return embedFn(texts, inputType);
    }

    const providers = {
      voyage: {
        url: apiUrl || 'https://api.voyageai.com/v1/embeddings',
        model: model || 'voyage-3-lite',
        body: (texts, inputType) => ({ model: providers.voyage.model, input: texts, input_type: inputType }),
        parse: (data) => data.data.map(d => d.embedding)
      },
      openai: {
        url: apiUrl || 'https://api.openai.com/v1/embeddings',
        model: model || 'text-embedding-3-small',
        body: (texts) => ({ model: providers.openai.model, input: texts }),
        parse: (data) => data.data.map(d => d.embedding)
      },
      cohere: {
        url: apiUrl || 'https://api.cohere.ai/v1/embed',
        model: model || 'embed-english-v3.0',
        body: (texts, inputType) => ({ model: providers.cohere.model, texts, input_type: inputType === 'query' ? 'search_query' : 'search_document' }),
        parse: (data) => data.embeddings
      }
    };

    const prov = providers[provider];
    if (!prov) throw new Error(`Unknown embedding provider: ${provider}. Use 'voyage', 'openai', 'cohere', or 'custom'.`);

    const res = await fetch(prov.url, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${apiKey}`
      },
      body: JSON.stringify(prov.body(texts, inputType))
    });

    if (!res.ok) throw new Error(`Embedding API ${res.status}: ${await res.text()}`);
    const data = await res.json();
    return prov.parse(data);
  }

  _float32Buffer(arr) {
    return Buffer.from(new Float32Array(arr).buffer);
  }

  // --- Intel Operations ---

  /**
   * Add an intelligence entry.
   * @param {string} category - Entry category
   * @param {Object} data - Entry data
   * @param {string} data.title - Title (required)
   * @param {string} [data.body] - Full content
   * @param {string} [data.source] - Source reference
   * @param {string[]} [data.tags] - Tags array
   * @param {number} [data.confidence] - 0.0 to 1.0
   * @param {boolean} [data.actionable] - Is this actionable?
   * @param {Object} [data.metadata] - Arbitrary metadata
   * @param {string} [data.expires_at] - Expiration timestamp
   * @returns {Promise<number>} Entry ID
   */
  async add(category, data) {
    const { title, body, source, tags, confidence, actionable, metadata, expires_at } = data;
    if (!title) throw new Error('title is required');

    const result = this.db.prepare(`
      INSERT INTO intel (category, title, body, source, tags, confidence, actionable, metadata, expires_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
    `).run(
      category, title, body || null, source || null,
      JSON.stringify(tags || []), confidence ?? 0.5,
      actionable ? 1 : 0, JSON.stringify(metadata || {}), expires_at || null
    );

    const id = Number(result.lastInsertRowid);

    // Embed and store vector
    if (this.embeddingConfig && this._vecLoaded) {
      try {
        const text = `${title}. ${body || ''}`.trim();
        const [vector] = await this._embed([text]);
        this.db.prepare('INSERT INTO intel_vec (intel_id, embedding) VALUES (?, ?)').run(BigInt(id), this._float32Buffer(vector));
      } catch (e) {
        // Embedding failed — entry still saved without vector
      }
    }

    return id;
  }

  /**
   * Query entries by category.
   * @param {string} [category='all'] - Category filter
   * @param {Object} [options]
   * @param {number} [options.limit=20]
   * @param {string} [options.since] - ISO date string
   * @param {string} [options.tag] - Filter by tag
   * @returns {Object[]} Matching entries
   */
  query(category = 'all', options = {}) {
    const { limit = 20, since, tag } = options;

    let sql = 'SELECT * FROM intel WHERE 1=1';
    const params = [];

    if (category && category !== 'all') { sql += ' AND category = ?'; params.push(category); }
    if (since) { sql += ' AND created_at >= ?'; params.push(since); }
    if (tag) { sql += ' AND tags LIKE ?'; params.push(`%"${tag}"%`); }

    sql += ' ORDER BY created_at DESC LIMIT ?';
    params.push(limit);

    const rows = this.db.prepare(sql).all(...params);
    rows.forEach(r => { r.tags = JSON.parse(r.tags); r.metadata = JSON.parse(r.metadata); });
    return rows;
  }

  /**
   * Keyword search across title, body, and source.
   * @param {string} text - Search query
   * @param {number} [limit=10]
   * @returns {Object[]} Matching entries
   */
  search(text, limit = 10) {
    const pattern = `%${text}%`;
    const rows = this.db.prepare(`
      SELECT * FROM intel WHERE title LIKE ? OR body LIKE ? OR source LIKE ?
      ORDER BY created_at DESC LIMIT ?
    `).all(pattern, pattern, pattern, limit);

    rows.forEach(r => { r.tags = JSON.parse(r.tags); r.metadata = JSON.parse(r.metadata); });
    return rows;
  }

  /**
   * Semantic (vector) search.
   * @param {string} queryText - Search query
   * @param {number} [limit=10]
   * @returns {Promise<Object[]>} Results with similarity scores
   */
  async semantic(queryText, limit = 10) {
    if (!this.embeddingConfig) throw new Error('No embedding provider configured');
    if (!this._vecLoaded) throw new Error('sqlite-vec not loaded — semantic search unavailable');

    const [queryVec] = await this._embed([queryText], 'query');

    const vecResults = this.db.prepare(`
      SELECT intel_id, distance FROM intel_vec
      WHERE embedding MATCH ? ORDER BY distance LIMIT ?
    `).all(this._float32Buffer(queryVec), limit);

    if (vecResults.length === 0) return [];

    const ids = vecResults.map(r => r.intel_id);
    const distMap = Object.fromEntries(vecResults.map(r => [r.intel_id, r.distance]));

    const placeholders = ids.map(() => '?').join(',');
    const rows = this.db.prepare(`SELECT * FROM intel WHERE id IN (${placeholders})`).all(...ids);

    rows.forEach(r => {
      r.tags = JSON.parse(r.tags);
      r.metadata = JSON.parse(r.metadata);
      r.similarity = 1 / (1 + distMap[r.id]);
    });
    rows.sort((a, b) => b.similarity - a.similarity);

    return rows;
  }

  /**
   * Hybrid search: 70% semantic + 30% keyword (Reciprocal Rank Fusion).
   * @param {string} queryText - Search query
   * @param {number} [limit=10]
   * @returns {Promise<Object[]>} Results with hybrid_score
   */
  async think(queryText, limit = 10) {
    // Keyword results
    const pattern = `%${queryText}%`;
    const kwRows = this.db.prepare(`
      SELECT id, title, body FROM intel WHERE title LIKE ? OR body LIKE ? LIMIT ?
    `).all(pattern, pattern, limit * 2);
    const kwSet = new Map(kwRows.map(r => [r.id, 0.3]));

    // Semantic results
    let vecScores = new Map();
    if (this.embeddingConfig && this._vecLoaded) {
      try {
        const [queryVec] = await this._embed([queryText], 'query');
        const vecResults = this.db.prepare(`
          SELECT intel_id, distance FROM intel_vec
          WHERE embedding MATCH ? ORDER BY distance LIMIT ?
        `).all(this._float32Buffer(queryVec), limit * 2);
        vecScores = new Map(vecResults.map(r => [r.intel_id, 1 / (1 + r.distance)]));
      } catch { /* fallback to keyword only */ }
    }

    // Fuse scores
    const allIds = new Set([...kwSet.keys(), ...vecScores.keys()]);
    const scored = [];
    for (const id of allIds) {
      const kwScore = kwSet.has(id) ? 0.3 : 0;
      const vecScore = vecScores.get(id) || 0;
      scored.push({ id, score: 0.7 * vecScore + kwScore });
    }
    scored.sort((a, b) => b.score - a.score);

    const topIds = scored.slice(0, limit).map(s => s.id);
    const scoreMap = Object.fromEntries(scored.map(s => [s.id, s.score]));

    if (topIds.length === 0) return [];

    const placeholders = topIds.map(() => '?').join(',');
    const rows = this.db.prepare(`SELECT * FROM intel WHERE id IN (${placeholders})`).all(...topIds);

    rows.forEach(r => {
      r.tags = JSON.parse(r.tags);
      r.metadata = JSON.parse(r.metadata);
      r.hybrid_score = scoreMap[r.id];
    });
    rows.sort((a, b) => b.hybrid_score - a.hybrid_score);

    return rows;
  }

  /**
   * Re-embed all entries missing vector representations.
   * @returns {Promise<{embedded: number, total: number}>}
   */
  async reindex() {
    if (!this.embeddingConfig) throw new Error('No embedding provider configured');
    if (!this._vecLoaded) throw new Error('sqlite-vec not loaded');

    const missing = this.db.prepare(`
      SELECT i.id, i.title, i.body FROM intel i
      LEFT JOIN intel_vec v ON i.id = v.intel_id
      WHERE v.intel_id IS NULL
    `).all();

    if (missing.length === 0) return { embedded: 0, total: 0 };

    const batchSize = 128;
    let embedded = 0;

    for (let i = 0; i < missing.length; i += batchSize) {
      const batch = missing.slice(i, i + batchSize);
      const texts = batch.map(r => `${r.title}. ${r.body || ''}`.trim());

      try {
        const vectors = await this._embed(texts);
        const insert = this.db.prepare('INSERT OR REPLACE INTO intel_vec (intel_id, embedding) VALUES (?, ?)');
        const tx = this.db.transaction(() => {
          batch.forEach((entry, j) => {
            insert.run(BigInt(entry.id), this._float32Buffer(vectors[j]));
          });
        });
        tx();
        embedded += batch.length;
      } catch (e) {
        // Batch failed — continue with next
      }
    }

    return { embedded, total: missing.length };
  }

  /**
   * Get database statistics.
   * @returns {Object} Stats object
   */
  stats() {
    const categories = this.db.prepare(`
      SELECT category, COUNT(*) as count, MAX(created_at) as latest
      FROM intel GROUP BY category ORDER BY count DESC
    `).all();
    const total = this.db.prepare('SELECT COUNT(*) as total FROM intel').get();
    const actionable = this.db.prepare('SELECT COUNT(*) as count FROM intel WHERE actionable = 1').get();

    let vectored = { count: 0 };
    if (this._vecLoaded) {
      try { vectored = this.db.prepare('SELECT COUNT(*) as count FROM intel_vec').get(); } catch {}
    }

    const trades = this.db.prepare(`
      SELECT COUNT(*) as total,
             SUM(CASE WHEN outcome IS NOT NULL THEN 1 ELSE 0 END) as resolved,
             SUM(COALESCE(pnl_cents, 0)) as total_pnl_cents
      FROM trades
    `).get();

    return {
      total: total.total,
      vectored: vectored.count,
      actionable: actionable.count,
      vecEnabled: this._vecLoaded,
      categories,
      trades: {
        total: trades.total,
        resolved: trades.resolved,
        total_pnl_cents: trades.total_pnl_cents
      }
    };
  }

  /**
   * List all tags with counts.
   * @param {string} [category] - Optional category filter
   * @returns {Array<{tag: string, count: number}>}
   */
  tags(category) {
    let sql = 'SELECT tags FROM intel';
    const params = [];
    if (category) { sql += ' WHERE category = ?'; params.push(category); }

    const rows = this.db.prepare(sql).all(...params);
    const tagCounts = {};
    rows.forEach(r => JSON.parse(r.tags).forEach(t => { tagCounts[t] = (tagCounts[t] || 0) + 1; }));

    return Object.entries(tagCounts)
      .sort((a, b) => b[1] - a[1])
      .map(([tag, count]) => ({ tag, count }));
  }

  // --- Trade Operations ---

  /**
   * Add a trade entry.
   * @param {Object} data - Trade data
   * @returns {number} Trade ID
   */
  addTrade(data) {
    const { ticker, series_ticker, direction, contracts, entry_price, thesis, metadata } = data;

    const result = this.db.prepare(`
      INSERT INTO trades (ticker, series_ticker, direction, contracts, entry_price, thesis, metadata)
      VALUES (?, ?, ?, ?, ?, ?, ?)
    `).run(ticker, series_ticker || null, direction, contracts, entry_price, thesis || null, JSON.stringify(metadata || {}));

    return Number(result.lastInsertRowid);
  }

  /**
   * Resolve a trade with outcome and P&L.
   * @param {number} id - Trade ID
   * @param {string} outcome - 'win', 'loss', or 'scratch'
   * @param {number} pnl_cents - P&L in cents
   */
  resolveTrade(id, outcome, pnl_cents) {
    this.db.prepare("UPDATE trades SET outcome = ?, pnl_cents = ?, resolved_at = datetime('now') WHERE id = ?").run(outcome, pnl_cents, id);
  }

  /**
   * List trades with optional filters.
   * @param {Object} [options]
   * @param {number} [options.limit=20]
   * @param {string} [options.since] - ISO date
   * @param {boolean} [options.resolved] - Filter by resolved status
   * @returns {Object[]} Trades
   */
  trades(options = {}) {
    const { limit = 20, since, resolved } = options;

    let sql = 'SELECT * FROM trades WHERE 1=1';
    const params = [];
    if (since) { sql += ' AND placed_at >= ?'; params.push(since); }
    if (resolved === true) sql += ' AND outcome IS NOT NULL';
    if (resolved === false) sql += ' AND outcome IS NULL';
    sql += ' ORDER BY placed_at DESC LIMIT ?';
    params.push(limit);

    const rows = this.db.prepare(sql).all(...params);
    rows.forEach(r => r.metadata = JSON.parse(r.metadata));
    return rows;
  }

  /**
   * Export all entries with optional filters.
   * @param {Object} [options]
   * @returns {Object[]} All matching entries
   */
  export(options = {}) {
    const { since, category } = options;
    let sql = 'SELECT * FROM intel WHERE 1=1';
    const params = [];
    if (since) { sql += ' AND created_at >= ?'; params.push(since); }
    if (category) { sql += ' AND category = ?'; params.push(category); }
    sql += ' ORDER BY created_at ASC';

    const rows = this.db.prepare(sql).all(...params);
    rows.forEach(r => { r.tags = JSON.parse(r.tags); r.metadata = JSON.parse(r.metadata); });
    return rows;
  }

  /**
   * Close the database connection.
   */
  close() {
    if (this.db) this.db.close();
  }
}

module.exports = { OpenIntel };
