#!/bin/bash
input=$(cat)

MODEL=$(echo "$input" | jq -r '.model.display_name')
DIR=$(echo "$input" | jq -r '.workspace.current_dir')
SESSION_ID=$(echo "$input" | jq -r '.session_id // empty')

# The name OTHER Claude sessions use to address this one (what ListAgents
# prints and SendMessage takes). It isn't in the statusline payload — it lives
# in the session registry, keyed by pid, alongside the messaging socket.
# Falls back to a scan by session id if CLAUDE_PID isn't set, then to the
# session's human label, then to the directory name.
PEER=""
if [ -n "${CLAUDE_PID:-}" ] && [ -f "$HOME/.claude/sessions/$CLAUDE_PID.json" ]; then
  PEER=$(jq -r '.name // empty' "$HOME/.claude/sessions/$CLAUDE_PID.json" 2>/dev/null)
fi
if [ -z "$PEER" ] && [ -n "$SESSION_ID" ]; then
  PEER=$(jq -rs --arg s "$SESSION_ID" \
    'map(select(.sessionId == $s)) | .[0].name // empty' \
    "$HOME"/.claude/sessions/*.json 2>/dev/null)
fi
[ -n "$PEER" ] || PEER=$(echo "$input" | jq -r '.session_name // empty')
COST=$(echo "$input" | jq -r '.cost.total_cost_usd // 0')
PCT=$(echo "$input" | jq -r '.context_window.used_percentage // 0' | cut -d. -f1)
DURATION_MS=$(echo "$input" | jq -r '.cost.total_duration_ms // 0')

CYAN='\033[36m'; GREEN='\033[32m'; YELLOW='\033[33m'; RED='\033[31m'; RESET='\033[0m'

# Pick bar color based on context usage
if [ "$PCT" -ge 90 ]; then BAR_COLOR="$RED"
elif [ "$PCT" -ge 70 ]; then BAR_COLOR="$YELLOW"
else BAR_COLOR="$GREEN"; fi

FILLED=$((PCT / 10)); EMPTY=$((10 - FILLED))
printf -v FILL "%${FILLED}s"; printf -v PAD "%${EMPTY}s"
BAR="${FILL// /█}${PAD// /░}"

MINS=$((DURATION_MS / 60000)); SECS=$(((DURATION_MS % 60000) / 1000))

BRANCH=""
git rev-parse --git-dir > /dev/null 2>&1 && BRANCH=" | 🌿 $(git branch --show-current 2>/dev/null)"

# Prefer the peer name over the directory: it's what you'd type to message
# this session, and it already carries the directory name in its derived form.
if [ -n "$PEER" ]; then
  WHERE="📡 $PEER"
else
  WHERE="📁 ${DIR##*/}"
fi

echo -e "${CYAN}[$MODEL]${RESET} $WHERE$BRANCH"
COST_FMT=$(printf '$%.2f' "$COST")
echo -e "${BAR_COLOR}${BAR}${RESET} ${PCT}% | ${YELLOW}${COST_FMT}${RESET} | ⏱️ ${MINS}m ${SECS}s"
