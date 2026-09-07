#!/usr/bin/env bash
# Morning brief: a headless agent run in the trading desk. Writes briefs/<date>.md
# and pushes the headline plus body to the ntfy topic in .ntfy_topic.
# Read-only by construction: the tool allowlist has no order, cancel, or paid tool.
set -u
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:-unix:path=$XDG_RUNTIME_DIR/bus}"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:/usr/local/bin:/usr/bin:/bin:$PATH"

DESK="$(cd "$(dirname "$0")/.." && pwd)"
cd "$DESK" || exit 1
mkdir -p briefs
F="briefs/$(date +%F).md"
TOPIC="$(cat .ntfy_topic)"

OPENINTEL="mcp__openintel__analyze_ticker,mcp__openintel__scan_watchlist,mcp__openintel__compare_tickers,mcp__openintel__list_sources,mcp__openintel__risk_frame,mcp__openintel__dip_scan,mcp__openintel__dip_review,mcp__openintel__discover,mcp__openintel__open_positions,mcp__openintel__review_trades"
ROBINHOOD="mcp__robinhood-trading__get_accounts,mcp__robinhood-trading__get_equity_positions,mcp__robinhood-trading__get_equity_quotes,mcp__robinhood-trading__get_equity_news,mcp__robinhood-trading__get_earnings_calendar,mcp__robinhood-trading__get_earnings_results,mcp__robinhood-trading__get_equity_historicals,mcp__robinhood-trading__get_equity_fundamentals,mcp__robinhood-trading__get_equity_technical_indicators,mcp__robinhood-trading__get_sec_filing,mcp__robinhood-trading__get_sec_filing_index,mcp__robinhood-trading__get_index_quotes,mcp__robinhood-trading__get_portfolio,mcp__robinhood-trading__get_equity_orders,mcp__robinhood-trading__search"

claude -p "$(cat prompts/morning.md)" \
  --allowedTools "Read,Glob,$OPENINTEL,$ROBINHOOD" \
  < /dev/null > "$F" 2> "briefs/$(date +%F).log"

if [ ! -s "$F" ] || ! grep -q '^PUSH: ' "$F"; then
  curl -s -o /dev/null -H "Title: morning brief failed" -d "no brief written; see briefs/$(date +%F).log" "https://ntfy.sh/$TOPIC"
  exit 1
fi

TITLE="$(grep -m1 '^PUSH: ' "$F" | sed 's/^PUSH: //')"
SUMMARY="$(sed -n '/^PUSH: /,/^# /p' "$F" | sed '1d;$d' | sed '/^[[:space:]]*$/d')"
curl -s -o /dev/null -H "Title: $TITLE" -d "$SUMMARY
full brief: briefs/$(date +%F).md" "https://ntfy.sh/$TOPIC"
