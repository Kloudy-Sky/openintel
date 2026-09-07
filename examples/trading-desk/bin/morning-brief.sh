#!/usr/bin/env bash
# Morning brief: a headless agent run in the trading desk. Writes briefs/<date>.md
# and pushes the headline plus body to the ntfy topic in .ntfy_topic.
# Read-only by construction: the tool allowlist has no order, cancel, paid, or journal-writing tool;
# dip verdicts come from the post-close digest, never a pre-market rescan.
set -u
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:-unix:path=$XDG_RUNTIME_DIR/bus}"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:/usr/local/bin:/usr/bin:/bin:$PATH"

DESK="$(cd "$(dirname "$0")/.." && pwd)"
cd "$DESK" || exit 1
mkdir -p briefs
F="briefs/$(date +%F).md"
TOPIC="$(cat .ntfy_topic 2>/dev/null || true)"
if [ -z "$TOPIC" ]; then echo "morning-brief: .ntfy_topic is missing or empty" >&2; exit 1; fi
NTFY="https://ntfy.sh/$TOPIC"

push() { curl -fsS --max-time 20 -o /dev/null -H "Title: $1" -d "$2" "$NTFY"; }

OPENINTEL="mcp__openintel__analyze_ticker,mcp__openintel__scan_watchlist,mcp__openintel__compare_tickers,mcp__openintel__list_sources,mcp__openintel__risk_frame,mcp__openintel__open_positions,mcp__openintel__review_trades"
ROBINHOOD="mcp__robinhood-trading__get_accounts,mcp__robinhood-trading__get_equity_positions,mcp__robinhood-trading__get_equity_quotes,mcp__robinhood-trading__get_equity_news,mcp__robinhood-trading__get_earnings_calendar,mcp__robinhood-trading__get_earnings_results,mcp__robinhood-trading__get_equity_historicals,mcp__robinhood-trading__get_equity_fundamentals,mcp__robinhood-trading__get_equity_technical_indicators,mcp__robinhood-trading__get_sec_filing,mcp__robinhood-trading__get_sec_filing_index,mcp__robinhood-trading__get_index_quotes,mcp__robinhood-trading__get_portfolio,mcp__robinhood-trading__get_equity_orders,mcp__robinhood-trading__search"

claude -p "$(cat prompts/morning.md)" \
  --allowedTools "Read,Glob,$OPENINTEL,$ROBINHOOD" \
  < /dev/null > "$F" 2> "briefs/$(date +%F).log" || {
  push "morning brief failed" "claude exited non-zero; see briefs/$(date +%F).log"
  exit 1
}

TITLE="$(head -1 "$F" | sed -n 's/^PUSH: //p')"
SUMMARY="$(sed -n '/^PUSH: /,/^# /p' "$F" | sed '1d;$d' | sed '/^[[:space:]]*$/d')"
if [ -z "$TITLE" ] || [ -z "$SUMMARY" ] || ! grep -q '^# ' "$F"; then
  push "morning brief malformed" "first line, summary, or heading missing; see briefs/$(date +%F).md"
  exit 1
fi

push "$TITLE" "$SUMMARY
full brief: briefs/$(date +%F).md"
