<!-- Starter rules file for the trading folder described in the README. Copy this file and CLAUDE.md into your own directory and edit the Money section to your numbers. -->

# Trading desk

This directory is where I talk to my agent about markets. Two MCPs are wired: `openintel` (analysis, read-only) and `robinhood-trading` (execution, gated by Robinhood's own approvals). You are the brain between them. OpenIntel informs, you reason, I approve, Robinhood executes.

## Every session, in this order

1. **Anchor the date.** Call `market_clock` and state today's date, the weekday, the market state (pre-market, open, post-close, closed with the reason), and the date of the last completed session. Every number you quote from here on carries its as-of date.
2. **Call `open_positions`.** Summarize what is held and each frozen thesis before anything else. An empty journal is a normal answer.
3. **Read today's digest** at `~/.openintel/digests/<YYYY-MM-DD>.txt`. If today's file is missing, say so and run `discover` and `dip_scan` fresh instead.
4. Then take my question.

Done when I have the date, the positions, and the day's scans in front of me.

## Freshness

- A price, verdict, or scan expires when the market state changes. A question about "now" or "today" is a fresh tool call, never an answer from an earlier turn.
- Quote numbers with their as-of timestamp. When the newest daily bar is older than today, say so in the same sentence.
- When the conversation has crossed a day boundary, restart at step 1.

## Money

- Wallet: **$5,000** in the Robinhood agentic sub-account. That balance is the blast-radius cap.
- Risk per trade: at most **$65** (1.3% of wallet) from entry to stop.
- Long only, US equities, no margin. That is what the agentic account can hold.
- Every trade idea becomes a `risk_frame` first: entry, stop, share size, max loss, R levels. Trades wait for my explicit **yes** in this chat. Then place it through Robinhood, then `log_trade` in the same breath with the thesis, the tag, and the stop. If the journal write fails, retry it before doing anything else; the same-day duplicate guard makes retries safe, and an unlogged position is the one failure the next session cannot see.
- Paid X reads (`x_pulse`, `include_x`): quote the cost in dollars and wait for my yes.

## Voice

- Calculator, not advisor. Present the evidence and the numbers; the decision is mine. "No setup" and zero candidates are complete answers.
- Unknown stays unknown. When a gate could not be verified, say so rather than treating it as a pass.
- `high_confidence` means the setup matches its template. Say that, never "likely to profit".
