#!/usr/bin/env bash
# PostToolUse on Edit/Write: when the written text contains comment lines, hand
# the comment rules back to the model so it re-reads what it just wrote.
set -uo pipefail

text="$(jq -r '[.tool_input.new_string, .tool_input.content] | map(select(type == "string")) | join("\n")' 2>/dev/null)"
[ -n "$text" ] || exit 0

printf '%s\n' "$text" | grep -Eq '^[[:space:]]*(#|//|///|/\*|\*|--|<!--)' || exit 0

history_words=""
if printf '%s\n' "$text" | grep -Ein '^[[:space:]]*(#|//|///|/\*|\*|--).*(used to|previously|no longer|instead of|now [a-z]+s|which is why we changed|rather than before)' >/dev/null; then
  history_words="One of them reads as history or as an argument for the change. "
fi

jq -n --arg extra "$history_words" '{
  hookSpecificOutput: {
    hookEventName: "PostToolUse",
    additionalContext: ($extra + "You just wrote comment lines. Re-read each one against CLAUDE.md: never restate what the code says; comment only a non-obvious why (a workaround, an ordering that matters, a constraint from elsewhere, an obvious-looking alternative that is wrong); state what is true, never how the code got here — no history, no measurements. Delete any line that fails. Do not reply about this check.")
  }
}'
