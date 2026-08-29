#!/usr/bin/env bash
# PostToolUse on Edit/Write: reject the edit when it adds a comment line to a
# source file. Blocked by default; `touch .claude/allow-comments` unlocks the
# next edit only, so writing a comment is a visible, deliberate act.
set -uo pipefail

input="$(cat)"
q() { jq -r "$1 // \"\"" <<<"$input"; }

path="$(q '.tool_input.file_path')"
[ -n "$path" ] || exit 0

case "$path" in
*/.claude/*) exit 0 ;;
*.rs | *.js | *.jsx | *.mjs | *.ts | *.py | *.css | *.sh | *.html | *.toml | *.yml | *.yaml | *.sql) ;;
*) exit 0 ;;
esac

allow="${CLAUDE_PROJECT_DIR:-.}/.claude/allow-comments"
if [ -e "$allow" ]; then
  rm -f "$allow"
  exit 0
fi

comments() {
  printf '%s\n' "$1" |
    grep -E '^[[:space:]]*(//|#|/\*|\*|--|<!--)' |
    grep -Ev '^[[:space:]]*#!' |
    sed 's/^[[:space:]]*//' |
    sort -u
}

new="$(q '.tool_input.new_string')"
[ -n "$new" ] || new="$(q '.tool_input.content')"
[ -n "$new" ] || exit 0

old="$(q '.tool_input.old_string')"
if [ -z "$old" ] && [ -n "${CLAUDE_PROJECT_DIR:-}" ]; then
  rel="${path#"$CLAUDE_PROJECT_DIR"/}"
  old="$(git -C "$CLAUDE_PROJECT_DIR" show "HEAD:$rel" 2>/dev/null)"
fi

added="$(comm -23 <(comments "$new") <(comments "$old"))"
[ -n "$added" ] || exit 0

{
  echo "Blocked: this edit adds comment lines to $path."
  echo
  printf '%s\n' "$added" | sed 's/^/    /'
  echo
  echo "CLAUDE.md: write no comments. Not a why comment, not a one-liner above a"
  echo "default, not a doc comment on a new field. Redo the edit without them —"
  echo "the explanation belongs in the commit message or in your answer."
  echo
  echo "If the user explicitly asked for a comment, run"
  echo "\`touch .claude/allow-comments\` first; it unlocks one edit and is deleted."
} >&2
exit 2
