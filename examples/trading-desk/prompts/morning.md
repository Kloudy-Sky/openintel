Write this morning's brief. This is a headless run: nobody can answer a question, so place no orders, spend no money (`x_pulse` stays unused and `include_x` stays false), and pass `no_journal: true` to `dip_scan` if you rescan anything. Output only the brief in markdown, nothing before or after it.

The output opens with a phone summary, then the brief.

The phone summary is what reaches a phone notification, so it is plain text: no markdown, no tables, no backticks, no bold. Its first line is exactly `PUSH: ` followed by one line under 120 characters: setup count, top ticker and why in a few words, any position alert. Then at most six short lines, under 500 characters in total: market state and the as-of time of the numbers, positions and alerts, each proposal as ticker, entry, stop, shares, max loss, and the one decision you want. The summary ends where the first markdown heading begins.

Then the brief, in this order:

# Morning brief <YYYY-MM-DD, weekday>, market <pre-market | open | closed>

## Positions
`open_positions` plus `get_equity_positions` for your broker's agentic account only. For each held name: last price against entry and stop, and any overnight `get_equity_news` or filing that touches the frozen thesis. Flag a name that is through its stop, or that has a catalyst, on its own line starting with **ALERT**.

## Yesterday's scans
Read `~/.openintel/digests/<most recent date>.txt`. List the dip candidates at `watch` or better and the chatter "leading the chart" flags, each with a fresh price from `get_equity_quotes`. Note which ones moved overnight.

## Calendar
`get_earnings_calendar` for today. Names that are held, on yesterday's lists, or large-cap, reporting before the open or after the close.

## Proposals, require your yes in a session
Up to three setups ranked by evidence, zero being a complete answer. Each one: ticker, thesis in one sentence, the `risk_frame` at the risk budget in AGENTS.md (entry, stop, shares, max loss, 1R/2R/3R), what invalidates it, and every gate with its status as pass, fail, or unknown.

## Decide today
One line: the single decision you want from me this morning.

Every number carries its as-of time. Unknown stays unknown. Calculator, not advisor.
