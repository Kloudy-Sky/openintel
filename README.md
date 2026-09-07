# OpenIntel

**Screens, gates, and journals for your trading agent. Never picks, never trades.**

![rust](https://img.shields.io/badge/rust-CLI%20%2B%20MCP-orange) ![read-only](https://img.shields.io/badge/broker-read--only%20by%20design-blue) ![not advice](https://img.shields.io/badge/not-financial%20advice-yellow) ![license](https://img.shields.io/badge/license-MIT-lightgrey)

OpenIntel turns market and social data into evidence-gated analysis: crowding for a ticker, movers with their evidence attached, dips graded through hard gates, mention velocity from the accounts you actually listen to, exact risk numbers, and a journal that freezes your thesis at entry and grades it later. It runs as a CLI or as a local MCP server your AI agent can call. Execution stays with your broker's own MCP and your own thumb on the approve button.

```text
your agent (Claude Code / ChatGPT / Codex / Cursor / Grok)     the brain
  ├─ MCP → openintel                                          the analysis   (this repo)
  └─ MCP → agent.robinhood.com/mcp/trading                    the execution  (broker's MCP, broker's approvals)
```

> **Not financial advice.** OpenIntel computes; it does not predict or recommend. Social data is noisy and easily manipulated. Do your own diligence, and read [before connecting a broker](#broker-warning).

## Sixty seconds

```bash
cargo install --path .                                             # 1. install (or try it first: cargo run -- analyze AAPL)
openintel analyze AAPL                                             # 2. works right now: market data is keyless (Yahoo)
openintel setup reddit; openintel setup bluesky                    # 3. optional free social sources, guided, saved to your OS keychain

claude mcp add --scope user openintel -- openintel mcp             # 4. wire the analysis into your agent, for every directory
claude mcp add --scope user --transport http robinhood-trading https://agent.robinhood.com/mcp/trading   # 5. wire execution (broker's MCP)
```

Then, in a chat: *"call open_positions, then run discover and tell me what has evidence."* Or skip the agent and use the CLI directly. Other agents add the same two MCP commands in their own settings. Offline or unconfigured sources degrade with a note, never a fabricated value.

## Cheat sheet

Every capability, both surfaces. Every MCP tool is read-only toward markets and brokers; the journal tools write only your local journal.

| Question | CLI | MCP tool | Cost |
|---|---|---|---|
| Is this ticker crowded? | `analyze AAPL` | `analyze_ticker` · `scan_watchlist` · `compare_tickers` | free |
| Where are trades today? | `discover` | `discover` | free |
| Is this dip a setup? | `dip` · `dip NVDA` | `dip_scan` | free |
| Did the dip score mean anything? | `dip --review` | `dip_review` | free |
| What's my listening set talking about? | `discover --chatter` | `discover` with `mode: chatter` | free (X leg paid, opt-in) |
| What did that X account just post? | `pulse TSLA` | `x_pulse` | **paid**, opt-in, confirmed first |
| How much can I lose on this idea? | `risk NVDA --budget 200` | `risk_frame` | free |
| What am I holding and why? | `journal positions` | `open_positions` | free |
| Log, amend, close a trade | `journal log` · `amend` · `close` | `log_trade` · `update_trade` | free |
| How's my record, honestly? | `journal review` | `review_trades` | free |
| Which sources are live? | | `list_sources` | free |

## Things it will never do

- **Place a trade, hold broker credentials, or touch your account.** It writes only its own files under `~/.openintel/` (journals, the chatter baseline, your listening set) and, through `setup`, your OS keychain.
- **Upgrade a verdict on evidence it couldn't fetch.** Every gate is pass / fail / unknown. Unknown caps the verdict, never lifts it.
- **Tell you something is likely to profit.** `high_confidence` means "matches the setup template". Scores are unvalidated until the review loop grades them.
- **Draw a conclusion from a small sample.** Reviews call themselves anecdote below n ≥ 30.
- **Spend money without asking.** X reads cost real money and are opt-in every single time.

## Features

Each fold is one command and its MCP tool. The cheat sheet above is the index.

<details>
<summary><b>Analyze</b> · crowding and divergence for one ticker</summary>

```bash
openintel analyze AAPL                                  # all social sources + market snapshot
openintel analyze AAPL --enable-reddit --enable-bluesky # restrict to these sources
openintel analyze AAPL --no-market --format json        # social only, JSON
```

| Flag | Meaning |
|---|---|
| `--enable-reddit` / `--enable-bluesky` | Restrict to these sources (none given → all configured) |
| `--no-market` | Skip the market snapshot |
| `--limit <N>` | Posts per source (default 50) |
| `--format table\|json` | Output format (default table) |

What it computes:

- **net sentiment**: mean per-post polarity in `[-1, 1]`
- **speculation index**: share of posts using options and leverage jargon
- **rvol / pct change**: volume vs average, day move
- **crowding**: blended speculation + RVOL + IV rank in `[0, 1]`
- **alignment**: `ConfirmingBullish` / `ConfirmingBearish` / `Diverging` / `Quiet`

On a ≤ −4% down day the report also carries a `dip_signal` (see Dip). MCP: `analyze_ticker` for one symbol, `scan_watchlist` for many run concurrently, `compare_tickers` to rank a set by `crowding`, `speculation_index`, `net_sentiment`, or `divergence`.

</details>

<details>
<summary><b>Discover</b> · movers with evidence, never picks</summary>

Answers "where are trades today?" without a ticker. Pulls Yahoo's predefined screens (gainers, losers, most actives, keyless), applies the same quality floor as Dip, and annotates a bounded slice with evidence:

- **Tape:** day change, RVOL, ATR-stretch vs SMA20, RSI(14).
- **Period extremes:** distance in ATRs to the 3-month and ~1-year high/low. A cheap proxy for where the crowd's reference points sit, not support or resistance. The output states the actual span covered.
- **Catalyst gates:** same-day SEC filings (EDGAR) and company-referencing catalyst headlines, with the usual fail-closed `unknown` when evidence can't be fetched.
- **Attention:** social mentions and net sentiment where sources are configured. Crowding context, not a signal.

```bash
openintel discover                          # all three screens
openintel discover --screens losers --deep 5
openintel discover --format json
```

No ranking, no verdicts, no picks. The evidence is the product. Losers carry a pointer to Dip for the gated verdict instead of restating it. MCP: `discover`.

</details>

<details>
<summary><b>Dip</b> · gated setups, not picks</summary>

Pulls the day's 100 biggest losers (Yahoo's keyless screener), applies a quality floor (≥ $5, ≥ $500M cap, ≥ 1M avg volume, listed ≥ 180 days, no OTC) and a drop band of −15%…−4% (the catastrophic tail is excluded by design), then grades survivors through hard gates into a tiered verdict:

| Verdict | Meaning |
|---|---|
| `no_setup` | Eligibility failed or a catalyst was confirmed: same-day SEC 8-K / 6-K / 424B5 / S-3 / FWP via EDGAR, or a catalyst-keyword headline naming the company. Evidence shown. |
| `watch` | Eligible, no confirmed catalyst, but at least one gate failed or could not be verified. **Unverifiable evidence fails closed.** |
| `high_confidence` | Every gate passes post-close: no filing, no catalyst headline, idiosyncratic vs SPY (≤ −3% excess), closed in the upper half of the day's range, score ≥ 65. |

```bash
openintel dip                                 # scan the losers universe
openintel dip NVDA                            # one ticker (floor unverifiable → caps at watch)
openintel dip --equity 25000 --leverage 2     # adds ATR-stop sizing + margin mechanics
openintel dip --review                        # grade past scans against forward returns
```

`high_confidence` means **conformance to the setup template, never probability of profit**. Zero candidates is a normal result. Intraday runs always cap at `watch` because the day bar isn't final; run after the close for real verdicts. Keyword hits in headlines that clearly reference the company confirm a catalyst; hits only in generic market-roundup headlines cap at `watch` instead of killing the candidate.

**The score is v0 and unvalidated.** Every scan appends a line to `~/.openintel/dip_journal.jsonl` (opt out with `--no-journal`). `dip --review` grades that journal against subsequent prices: 1/5/10-trading-day raw and SPY-adjusted returns per verdict, win rates, and a score-to-return correlation. Until the graded sample is meaningful (n ≥ 30, and only ~3 months of entries are gradable per run) the review says so instead of pretending.

With `--equity`, each candidate gets a risk frame (1% of equity to a 2×ATR stop by default) plus margin mechanics: buying-power cap, borrowed amount, margin-call price and its distance, and a warning when the margin call would fire **before** your stop. Overnight Reg-T caps leverage at 2x; `--intraday-bp` unlocks 4x with a not-holdable-overnight note. Single-position model; interest not modeled.

Data: Yahoo screener and news (keyless, unofficial; failure mode is a clean error) and SEC EDGAR (keyless, official; identify yourself via `OPENINTEL_SEC_CONTACT` if you fork this). MCP: `dip_scan` (pass `ticker` for one symbol, `equity` for sizing) and `dip_review`.

</details>

<details>
<summary><b>Chatter</b> · mention velocity from your listening set</summary>

`openintel discover --chatter` measures **mention velocity from the accounts and communities worth hearing**, stored at `~/.openintel/listening.json` (seeded on first run with a macro default; edit it, or let your agent propose additions for you to approve). Legs, per what's configured:

- **Bluesky** (free): author feeds of your listed accounts.
- **Reddit** (optional): hot posts from your listed subreddits.
- **X** (paid, strictly opt-in via `--x` / `include_x`): everything your listed accounts posted in the window. About $0.005 per post returned, deduped over 24h, capped at `--x-limit` (default 20, roughly $0.10 max per run).

```bash
openintel discover --chatter                    # free legs
openintel discover --chatter --hours 48         # wider window (1-167)
openintel discover --chatter --x --x-limit 10   # paid X leg, capped
```

Cashtag mentions are counted once per post and graded against a **baseline journal** (`~/.openintel/chatter_baseline.jsonl`, appended every run). Velocity is today's mention *share* vs the trailing mean, so growing your listening set can't fake a spike, and a changed set is flagged. **Honesty gates:** no velocity claim below 5 mentions today and 3 prior baseline days; until then the report shows raw counts and says "baseline building". The headline flag is **chatter leading the chart**: velocity ≥ 3× baseline while the day move is under 2% and RVOL is unremarkable.

Influencers write "Tesla", not "$TSLA", so the report also returns the recent posts themselves for you or your agent to read. Attention is the signal being measured, not information: chatter never ranks, never predicts, and is exactly the kind of thing crowding vetoes exist for. The baseline only matures with daily runs; the cron in [Recommended setup](#recommended-setup) does that for free. MCP: `discover` with `mode: chatter`; `include_x` spends real money and requires your confirmation first.

</details>

<details>
<summary><b>Pulse</b> · catalyst posts from specific X accounts (paid)</summary>

Catalyst posts from high-impact X accounts (a POTUS tariff post, a CEO announcement), surfaced as **events to reason about**, never averaged into sentiment. X's API is pay-per-use, so the pulse is strictly opt-in: nothing calls X unless you run it, and that includes the one verify read during `setup x`.

```bash
openintel setup x                                                    # guided token setup (verify reads ≈ $0.05)
openintel pulse TSLA --accounts elonmusk --keywords tesla,robotaxi   # ≤ 20 reads ≈ $0.10 max
```

Add `--keywords` with the company's own vocabulary; influencer posts say "Tesla", not "$TSLA". No `--accounts` gives a small macro default list (POTUS, White House, Musk, the Fed).

Billing is per post returned (about $0.005, deduped over a 24h UTC day, prepaid credits, no per-call minimum), but the search endpoint's `max_results` floor is 10, so a call can return up to 10 posts even at `--limit 1`. Budget about $0.05 worst case per call. MCP: `x_pulse` asks the agent to research which accounts matter for your ticker and confirm the cost with you before spending.

</details>

<details>
<summary><b>Risk</b> · ATR stop, size, and R levels for one idea</summary>

Turns a trade idea into exact numbers. `openintel risk NVDA --budget 200` returns an ATR(14)-based stop, the whole-share size that caps a stop-out at your budget, max loss, and 1R / 2R / 3R reference levels. Deterministic math over free Yahoo daily bars; it never recommends taking the trade.

Run intraday, the entry default is the live price and ATR includes today's still-forming bar. Re-run near the close for settled numbers. MCP: `risk_frame`, whose contract requires presenting the numbers and getting your explicit approval before any execution step.

</details>

<details>
<summary><b>Journal</b> · frozen theses, graded record</summary>

An append-only journal at `~/.openintel/trade_journal.jsonl`. Every trade is logged with its **thesis and plan frozen at entry**; they can never be retroactively edited to match the outcome. Amendments and closes append; a trade is a fold over its events.

```bash
openintel journal log NVDA --qty 10 --entry 178.50 --stop 172 \
  --thesis "bounce off weekly support at 175" --tag sr-support-bounce \
  --risk-usd 65 --wallet 5000                # equity; add --option call --strike/--expiry, or --crypto
openintel journal amend NVDA-20260822-1 --note "trailing after 1R" --stop 178.50
openintel journal close NVDA-20260822-1 --exit 191.20 --reason target
openintel journal positions                  # every open trade with its thesis and plan
openintel journal review                     # grade the whole record
```

The review grades realized **R vs the original stop** (amendments never soften the grade), win rates, **discipline flags** (widened stops, exits beyond the planned stop), and for equities 1/5/10-trading-day forward returns, raw and SPY-adjusted, bucketed by setup tag and instrument. Honesty gates: per-bucket numbers appear only at n ≥ 10 closed trades and overall conclusions need n ≥ 30; below that the report calls itself anecdote. Options grade on realized P&L plus the underlying's direction (historical option prices aren't available keyless); crypto on realized P&L only. Long-only, matching what a broker's agentic account can hold.

In an agent chat the MCP tools are the primary surface: `log_trade` journals the entry in the same moment you approve the trade (frozen thesis, required stop, risk-of-wallet snapshot, same-day duplicate guard), `update_trade` amends or closes, `open_positions` gives the next conversation the full context of what's held and why, and `review_trades` grades the record.

</details>

## Recommended setup

You don't need a bot, a memory system, or an always-on agent. Three small pieces cover the whole loop; the rest is a normal chat.

**1. A trading folder, not this repo.** Wire both MCPs at user scope (the `--scope user` flag above) so they follow you to any directory. Start market conversations from a dedicated directory whose `CLAUDE.md` (or your agent's equivalent) holds your standing rules. That file is the agent's stable memory:

```markdown
# Trading rules
- Wallet: $5,000 agentic sub-account. Max risk per trade: $65 (1.3%).
- Call `open_positions` first in every session.
- Never place a trade without my explicit approval in this chat.
- Log every approved trade with `log_trade` in the same breath it's placed.
- Read today's digest in ~/.openintel/digests/ before scanning.
```

A complete starter lives in [`examples/trading-desk/`](examples/trading-desk/): session order (anchor the date, read positions, read the digest), freshness rules so a next-day question triggers a fresh call instead of yesterday's numbers, money rules, and voice. Copy both files, edit the numbers.

**2. The journal is the memory.** `log_trade` at entry, `open_positions` at the start of the next chat, `review_trades` when the record is big enough to grade. A conversational "learning" memory would remember hunches with no grade attached; the journal remembers theses and scores them against what the market did.

**3. A post-close cron for the free scans.** One script, weekdays after the bell, no paid legs. Tomorrow's session reads the digest instead of re-running everything:

```bash
#!/usr/bin/env bash
# ~/.openintel/bin/daily-close.sh
OUT="$HOME/.openintel/digests"; mkdir -p "$OUT"; F="$OUT/$(date +%F).txt"
{ echo "--- dip ---"; openintel dip
  echo "--- discover ---"; openintel discover
  echo "--- chatter ---"; openintel discover --chatter; } > "$F" 2>&1
```

```bash
crontab -e   # 20 16 * * 1-5 $HOME/.openintel/bin/daily-close.sh   (16:20 local, Mon–Fri)
```

Cron can't see your session keychain by default. On Linux, add these two lines under the shebang so the script reaches it; secrets stay in the keychain, never in the script:

```bash
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:-unix:path=$XDG_RUNTIME_DIR/bus}"
```

**Optional: push the digest to Slack, Telegram, or Discord.** Not necessary, but if you want the close-of-day scan on your phone, add one line to the end of the script. A Slack incoming webhook, for example:

```bash
curl -s -X POST "$SLACK_WEBHOOK_URL" -H 'Content-type: application/json' \
  --data "$(jq -n --rawfile t "$F" '{text: $t}')"
```

Keep it one-way. A channel that receives the digest is a read-only surface. A chat bot that holds a live broker session is an always-on execution rail behind a chat token, and the next section is exactly why we don't ship one.

<a id="broker-warning"></a>
## ⚠️ Read before connecting a broker

Connecting an AI agent to a brokerage MCP means **an AI can place real trades with real money in your account.** Understand exactly what you're authorizing:

- **OpenIntel is a screener, not advice, and not a proven edge.** It surfaces *attention* and *crowding / divergence* signals from social chatter. Social sentiment is noisy, easily manipulated (bots, coordinated pumps), and mostly coincident-to-lagging, not predictive. Treat its output as one input to your own judgment, never as a buy or sell instruction.
- **AI agents make mistakes.** They hallucinate, misread data, act on stale or incomplete information, and can behave unexpectedly, including placing a wrong or oversized trade. Trading automatically on automated signals can lose money quickly.
- **You are fully responsible for every trade placed.** This software has no warranty and is not financial advice. Nothing here is a strategy shown to be profitable.
- **Only fund money you can afford to lose entirely.** Use a dedicated broker *agentic sub-account* and fund a deliberately small wallet. **That balance is your hard blast-radius cap.** The agent cannot spend beyond it.
- **Keep the broker's approval-required mode on.** Review and approve trades before they execute. Do not authorize unattended or autonomous trading until you genuinely trust the setup. Connecting also grants the agent broad **read** access to your accounts, which is a privacy surface.
- **Scope and status:** Robinhood's Agentic Trading is a **US** product covering equities, options, and crypto as of September 2026; check its current terms and scope yourself. OpenIntel itself is early software (live market data via Yahoo; Reddit and Bluesky sentiment live when configured); the intelligence layer is meant to be iterated on.

By design, **OpenIntel never executes trades, touches a broker, or holds credentials.** Execution happens only through the broker's own MCP, gated by the broker's controls and your approval. That boundary *is* the safety model. Keep it.

## Sources and secrets

Market data is keyless (Yahoo Finance, unofficial) and SEC EDGAR is keyless and official. Social sources are optional and credentialed. Reddit and Bluesky credentials are free; X bills about $0.05 for its verify read. Each `setup` command walks you through creating the credential, verifies it live, and saves it to your OS keychain. Rotate by re-running it; remove with `--forget`. Environment variables always override the keychain, which is what CI and cron want.

<details>
<summary><b>Reddit</b> · <code>openintel setup reddit</code></summary>

Create a **script** app at <https://www.reddit.com/prefs/apps>, then either follow the guided setup or:

```bash
export OPENINTEL_REDDIT_CLIENT_ID=your_client_id
export OPENINTEL_REDDIT_CLIENT_SECRET=your_secret
openintel setup reddit   # non-interactive when piped; verifies from env
```

</details>

<details>
<summary><b>Bluesky</b> · <code>openintel setup bluesky</code></summary>

Create an app password at <https://bsky.app/settings/app-passwords>, then either follow the guided setup or:

```bash
export OPENINTEL_BLUESKY_HANDLE=yourname.bsky.social      # custom-domain handles and a leading @ both work
export OPENINTEL_BLUESKY_APP_PASSWORD=xxxx-xxxx-xxxx-xxxx
openintel setup bluesky   # non-interactive when piped; verifies from env
```

</details>

<details>
<summary><b>X</b> · <code>openintel setup x</code> (paid)</summary>

Guided bearer-token setup. The verify step reads about $0.05 of posts. After that nothing calls X unless you run `pulse` or pass `--x` / `include_x` to chatter.

</details>

Secrets (`OPENINTEL_REDDIT_*`, `OPENINTEL_BLUESKY_*`, `OPENINTEL_MARKET_API_KEY`, the X token) are wrapped in `SecretString` end to end: plaintext never touches disk (the keychain is written only by `setup` after a live verify) and is never logged. Every request identifies itself to EDGAR via `OPENINTEL_SEC_CONTACT`; set it if you fork this.

## Under the hood

<details>
<summary><b>Architecture</b> · hexagonal, pure domain, IO at the edge</summary>

Ports and adapters. The domain is pure and synchronous: no IO, no clock, no filesystem. Time and paths are stamped at the application edge and injected.

- `domain/`: entities, value objects, port traits, and the pure engines. `SpeculationEngine`, `risk` (ATR stop and size), `margin` (buying power, margin-call price), `dip` (gates, score, verdict), `dip_review` (forward-return grading), `trade_journal` (event fold, frozen theses), `trade_review` (R-multiples, discipline, buckets).
- `application/`: orchestration at the IO edge. `analyze`, `pulse`, `risk`, `dip` (scan, check, journal), `review` (journal grading). Clock and filesystem stamped here.
- `adapters/`: `LexiconAnalyzer`; `YahooMarketSource` (keyless: chart bars, snapshot, `day_losers` screener, news and company names); `EdgarSource` (keyless SEC filings, ticker to CIK cached); `RedditSource`, `BlueskySource`, `XPulseSource` (credential-gated).
- `config/`: secrets resolution (env, then OS keychain, via `secrecy`) and runtime settings.
- `cli/`: clap args, per-command leaves, rendering. Only `main.rs` and the renderers print.
- `mcp/`: the rmcp stdio server exposing the tools in the cheat sheet.

Adapters are constructed only at the two composition roots, `main.rs` and `mcp::server::serve()`. A new source is a new adapter plus wiring at both roots, with zero engine or application changes.

</details>

<details>
<summary><b>Extending</b> · a new source is one adapter and two wires</summary>

**Add a social source:**
1. New struct in `src/adapters/sources/`, `impl SocialDataSource`.
2. Add a `SourceKind` variant in `src/domain/values/source_kind.rs` if new.
3. Construct it at both composition roots and push it onto the injected social list.

**Add a market source** (e.g. a keyed provider):
1. New struct in `src/adapters/market/`, `impl MarketDataSource`.
2. Construct it at both composition roots; it flows in through the injected `&dyn MarketDataSource`.

**Swap the analyzer** (lexicon to LLM or ML):
1. New struct in `src/adapters/analyzer/`, `impl PostAnalyzer`. No engine change.

</details>

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Tests marked `#[ignore]` hit live networks (Yahoo, EDGAR, Reddit, Bluesky, X, keychain) and are excluded by default. The X one spends real money; run it only on purpose.
