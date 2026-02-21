#!/usr/bin/env node
/**
 * OpenIntel CLI — Command-line interface for the OpenIntel knowledge base.
 */

const { OpenIntel } = require('./index');
const path = require('path');

// --- Config Resolution ---

function resolveConfig() {
  // Check env vars first
  const dbPath = process.env.OPENINTEL_DB || path.join(process.cwd(), 'openintel.db');
  const vecExtPath = process.env.OPENINTEL_VEC_EXT || null;

  const embedding = {};
  if (process.env.OPENINTEL_EMBEDDING_PROVIDER) {
    embedding.provider = process.env.OPENINTEL_EMBEDDING_PROVIDER;
    embedding.apiKey = process.env.OPENINTEL_EMBEDDING_API_KEY;
    embedding.apiUrl = process.env.OPENINTEL_EMBEDDING_API_URL;
    embedding.model = process.env.OPENINTEL_EMBEDDING_MODEL;
    embedding.dimensions = parseInt(process.env.OPENINTEL_EMBEDDING_DIMENSIONS) || 1024;
  }

  // Try to read from openclaw config (backward compat)
  if (!embedding.provider) {
    try {
      const configPath = path.join(process.env.HOME || '/home/node', '.openclaw', 'openclaw.json');
      const config = JSON.parse(require('fs').readFileSync(configPath, 'utf8'));
      const remote = config?.agents?.defaults?.memorySearch?.remote;
      if (remote?.apiKey) {
        embedding.provider = 'voyage';
        embedding.apiKey = remote.apiKey;
        embedding.apiUrl = remote.baseURL ? `${remote.baseURL}/embeddings` : 'https://ai.mongodb.com/v1/embeddings';
        embedding.model = 'voyage-4-lite';
        embedding.dimensions = 1024;
      }
    } catch { /* no openclaw config */ }
  }

  return {
    dbPath,
    vecExtPath,
    embedding: embedding.provider ? embedding : null
  };
}

function parseFlags(args) {
  const flags = {};
  for (let i = 0; i < args.length; i++) {
    if (args[i].startsWith('--')) {
      flags[args[i].slice(2)] = args[i + 1] || true;
      i++;
    }
  }
  return flags;
}

const HELP = `OpenIntel — Structured Intelligence Knowledge Base

Usage: openintel <command> [args]

Commands:
  add <category> <json>          Add an intel entry (auto-embeds if configured)
  query <category> [flags]       Query entries (--limit, --since, --tag)
  search <text> [--limit N]      Keyword search (title/body/source)
  semantic <query> [--limit N]   Vector similarity search
  think <query> [--limit N]      Hybrid: 70% semantic + 30% keyword (RRF)
  stats                          Overview statistics
  tags [category]                List all tags with counts
  trade-add <json>               Log a trade
  trade-resolve <id> <outcome> <pnl_cents>  Resolve a trade
  trades [--limit] [--since] [--resolved true/false]
  export [--since] [--category]  Export entries as JSON
  reindex                        Embed all entries missing vectors

Categories: market, newsletter, social, trading, opportunity, competitor, general

Environment variables:
  OPENINTEL_DB                   Database path (default: ./openintel.db)
  OPENINTEL_VEC_EXT              Path to sqlite-vec extension
  OPENINTEL_EMBEDDING_PROVIDER   Embedding provider (voyage/openai/cohere)
  OPENINTEL_EMBEDDING_API_KEY    API key for embeddings
  OPENINTEL_EMBEDDING_MODEL      Embedding model name
  OPENINTEL_EMBEDDING_DIMENSIONS Embedding dimensions (default: 1024)
`;

async function main() {
  const args = process.argv.slice(2);
  const cmd = args[0];

  if (!cmd || cmd === '--help' || cmd === '-h') {
    console.log(HELP);
    return;
  }

  const config = resolveConfig();
  const intel = new OpenIntel(config);

  try {
    switch (cmd) {
      case 'add': {
        const cat = args[1];
        const jsonStr = args.slice(2).join(' ');
        if (!cat || !jsonStr) { console.error('Usage: openintel add <category> <json>'); process.exit(1); }
        const id = await intel.add(cat, JSON.parse(jsonStr));
        console.log(JSON.stringify({ ok: true, id }));
        break;
      }
      case 'query': {
        const cat = args[1] || 'all';
        const f = parseFlags(args.slice(2));
        const results = intel.query(cat, { limit: parseInt(f.limit) || 20, since: f.since, tag: f.tag });
        console.log(JSON.stringify({ ok: true, count: results.length, results }));
        break;
      }
      case 'search': {
        const text = args[1];
        const f = parseFlags(args.slice(2));
        if (!text) { console.error('Usage: openintel search <text>'); process.exit(1); }
        const results = intel.search(text, parseInt(f.limit) || 10);
        console.log(JSON.stringify({ ok: true, count: results.length, results }));
        break;
      }
      case 'semantic': {
        const q = args[1];
        const f = parseFlags(args.slice(2));
        if (!q) { console.error('Usage: openintel semantic <query>'); process.exit(1); }
        const results = await intel.semantic(q, parseInt(f.limit) || 10);
        console.log(JSON.stringify({ ok: true, count: results.length, results }));
        break;
      }
      case 'think': {
        const q = args[1];
        const f = parseFlags(args.slice(2));
        if (!q) { console.error('Usage: openintel think <query>'); process.exit(1); }
        const results = await intel.think(q, parseInt(f.limit) || 10);
        console.log(JSON.stringify({ ok: true, count: results.length, results }));
        break;
      }
      case 'reindex': {
        const result = await intel.reindex();
        console.log(JSON.stringify({ ok: true, ...result }));
        break;
      }
      case 'stats': {
        const s = intel.stats();
        console.log(JSON.stringify({ ok: true, ...s }));
        break;
      }
      case 'tags': {
        const t = intel.tags(args[1]);
        console.log(JSON.stringify({ ok: true, tags: t }));
        break;
      }
      case 'trade-add': {
        const jsonStr = args.slice(1).join(' ');
        const id = intel.addTrade(JSON.parse(jsonStr));
        console.log(JSON.stringify({ ok: true, id }));
        break;
      }
      case 'trade-resolve': {
        intel.resolveTrade(parseInt(args[1]), args[2], parseInt(args[3]));
        console.log(JSON.stringify({ ok: true, id: parseInt(args[1]), outcome: args[2], pnl_cents: parseInt(args[3]) }));
        break;
      }
      case 'trades': {
        const f = parseFlags(args.slice(1));
        const trades = intel.trades({
          limit: parseInt(f.limit) || 20,
          since: f.since,
          resolved: f.resolved === 'true' ? true : f.resolved === 'false' ? false : undefined
        });
        console.log(JSON.stringify({ ok: true, count: trades.length, trades }));
        break;
      }
      case 'export': {
        const f = parseFlags(args.slice(1));
        const data = intel.export({ since: f.since, category: f.category });
        console.log(JSON.stringify({ ok: true, count: data.length, data }, null, 2));
        break;
      }
      default:
        console.log(HELP);
    }
  } finally {
    intel.close();
  }
}

main().catch(e => {
  console.error(JSON.stringify({ ok: false, error: e.message }));
  process.exit(1);
});
