#!/usr/bin/env node
/**
 * Basic smoke tests for OpenIntel.
 */

const { OpenIntel } = require('../src/index');
const path = require('path');
const fs = require('fs');

const TEST_DB = path.join(__dirname, 'test.db');

// Clean up
if (fs.existsSync(TEST_DB)) fs.unlinkSync(TEST_DB);

async function run() {
  let passed = 0;
  let failed = 0;

  function assert(name, condition) {
    if (condition) { console.log(`  ✓ ${name}`); passed++; }
    else { console.error(`  ✗ ${name}`); failed++; }
  }

  console.log('OpenIntel Tests\n');

  // --- Init ---
  console.log('Database initialization:');
  const intel = new OpenIntel({ dbPath: TEST_DB });
  assert('creates database file', fs.existsSync(TEST_DB));
  assert('stats returns object', typeof intel.stats() === 'object');
  assert('starts with 0 entries', intel.stats().total === 0);

  // --- Add ---
  console.log('\nAdd entries:');
  const id1 = await intel.add('market', { title: 'Test signal', body: 'AAPL looking strong', tags: ['AAPL', 'earnings'] });
  assert('returns numeric id', typeof id1 === 'number' && id1 > 0);

  const id2 = await intel.add('newsletter', { title: 'Morning Brew digest', body: 'Fed rate cut expected', tags: ['fed', 'macro'] });
  assert('second entry gets incremented id', id2 === id1 + 1);

  const id3 = await intel.add('market', { title: 'Crypto rally', body: 'BTC above 70K', tags: ['BTC', 'crypto'], confidence: 0.9, actionable: true });
  assert('third entry with all fields', id3 === id2 + 1);

  assert('stats shows 3 entries', intel.stats().total === 3);

  // --- Query ---
  console.log('\nQuery:');
  const allMarket = intel.query('market');
  assert('query market returns 2', allMarket.length === 2);

  const allEntries = intel.query('all');
  assert('query all returns 3', allEntries.length === 3);

  const tagFiltered = intel.query('all', { tag: 'fed' });
  assert('tag filter works', tagFiltered.length === 1 && tagFiltered[0].title === 'Morning Brew digest');

  // --- Search ---
  console.log('\nSearch:');
  const searchResults = intel.search('AAPL');
  assert('keyword search finds AAPL', searchResults.length === 1);

  const searchFed = intel.search('rate cut');
  assert('keyword search finds rate cut', searchFed.length === 1);

  const searchNone = intel.search('nonexistent_xyz_123');
  assert('search returns empty for no match', searchNone.length === 0);

  // --- Tags ---
  console.log('\nTags:');
  const allTags = intel.tags();
  assert('tags returns array', Array.isArray(allTags));
  assert('has AAPL tag', allTags.some(t => t.tag === 'AAPL'));
  assert('has fed tag', allTags.some(t => t.tag === 'fed'));

  // --- Trades ---
  console.log('\nTrades:');
  const tradeId = intel.addTrade({
    ticker: 'AAPL',
    direction: 'long',
    contracts: 100,
    entry_price: 185.50,
    thesis: 'Earnings momentum'
  });
  assert('trade created', tradeId > 0);

  const openTrades = intel.trades({ resolved: false });
  assert('open trades returns 1', openTrades.length === 1);

  intel.resolveTrade(tradeId, 'win', 350);
  const resolvedTrades = intel.trades({ resolved: true });
  assert('resolved trade found', resolvedTrades.length === 1 && resolvedTrades[0].outcome === 'win');

  // --- Export ---
  console.log('\nExport:');
  const exported = intel.export();
  assert('export returns all entries', exported.length === 3);

  // --- Cleanup ---
  intel.close();
  fs.unlinkSync(TEST_DB);

  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed > 0 ? 1 : 0);
}

run().catch(e => { console.error(e); process.exit(1); });
