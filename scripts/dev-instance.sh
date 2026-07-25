#!/usr/bin/env bash
# Run read-stats in isolation — a copy of the databases, a different port —
# so it can be exercised while the real one keeps tracking a reading session.
#
#   run                 boot on :3299 against the frozen copy (^C to stop)
#   snapshot <name>     boot, record every GET endpoint, stop
#   diff <a> <b>        compare two snapshots
#   check <baseline>    snapshot as "current" and diff against <baseline>
#   browser             render the dashboard headless and fail on a blank page
#   refresh             re-take the frozen copy from the live databases
#
# The intended workflow for a behaviour-preserving change:
#
#   scripts/dev-instance.sh snapshot before   # on the unmodified tree
#   ...make the change...
#   scripts/dev-instance.sh check before      # must print IDENTICAL
#
# The copy is taken once and *frozen*: two snapshots have to see the same rows
# to be comparable, and the live database is being written to the whole time.
# `refresh` re-takes it from the live database.
#
# Snapshots live in target/dev-instance/ and are disposable.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="$REPO/target/dev-instance"
PORT="${DEV_PORT:-3299}"
LIVE_STATS="${JP_TOOLS_STATS_DB_PATH:-$HOME/.local/share/jp-tools/read-stats.db}"
LIVE_KNOWLEDGE="${JP_TOOLS_KNOWLEDGE_DB_PATH:-$HOME/.local/share/jp-tools/knowledge.db}"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# Take the frozen copy, unless it is already there. Through SQLite rather than
# cp, so a WAL mid-write can't produce a torn file; read-only on the live
# database, and safe to run while it is being written.
freeze() {
  mkdir -p "$WORK/frozen"
  if [ -f "$WORK/frozen/read-stats.db" ] && [ -z "${REFRESH:-}" ]; then
    return 0
  fi
  rm -f "$WORK"/frozen/*.db*
  [ -f "$LIVE_STATS" ] && sqlite3 "$LIVE_STATS" ".backup '$WORK/frozen/read-stats.db'"
  if [ -f "$LIVE_KNOWLEDGE" ]; then
    sqlite3 "$LIVE_KNOWLEDGE" ".backup '$WORK/frozen/knowledge.db'"
  else
    # Pre-cutover: build the knowledge half from the copy, which also rehearses
    # the migration against real data.
    JP_TOOLS_KNOWLEDGE_DB_PATH="$WORK/frozen/knowledge.db" \
      JP_TOOLS_STATS_DB_PATH="$WORK/frozen/read-stats.db" \
      "$REPO/scripts/migrate-knowledge-db.sh" reading >"$WORK/migrate.log" 2>&1 ||
      { tail -5 "$WORK/migrate.log"; die "migration of the copy failed"; }
  fi
  say "froze a copy of the live databases in $WORK/frozen"
}

# A run mutates its databases (cover reconcile, the char recount), so each one
# starts from the frozen copy rather than from the last run's leftovers.
fresh_dbs() {
  freeze
  rm -f "$WORK"/run.db* "$WORK"/run-knowledge.db*
  cp "$WORK/frozen/read-stats.db" "$WORK/run.db"
  cp "$WORK/frozen/knowledge.db" "$WORK/run-knowledge.db"
}

# env -i so nothing from .env or the shell leaks in: the point is that this
# instance cannot reach Anki, whisper, or the capture script.
#
# `exec` so that backgrounding this function gives $! the server's own pid: a
# plain call would put a subshell in between, and killing the subshell would
# leave the server holding the port — which then answers the *next* run's
# requests from a database that has since been replaced.
serve() {
  exec env -i HOME="$HOME" PATH="$PATH" \
    JP_TOOLS_STATS_DB_PATH="$WORK/run.db" \
    JP_TOOLS_KNOWLEDGE_DB_PATH="$WORK/run-knowledge.db" \
    JP_TOOLS_STATS_LISTEN_ADDR="127.0.0.1:$PORT" \
    JP_TOOLS_ANKI_URL="http://127.0.0.1:9" \
    JP_TOOLS_WHISPER_URL="http://127.0.0.1:9" \
    JP_TOOLS_ANTHROPIC_API_KEY="" \
    JP_TOOLS_AUTO_CAPTURE_ON_ADD=0 \
    "$REPO/target/debug/read-stats" "$@"
}

# A leftover instance from an earlier run would answer on this port and the
# snapshot would silently be of *its* data, taken from a copy that no longer
# exists. Refuse rather than compare two different databases.
require_port_free() {
  if curl -sf "http://127.0.0.1:$PORT/api/settings" >/dev/null 2>&1; then
    die "something is already serving :$PORT — stop it first (DEV_PORT=... to use another)"
  fi
}

# Kill and *wait*: SQLite hands the file over on process exit, and the next run
# copies over it immediately.
stop_server() {
  [ -n "${SRV:-}" ] || return 0
  kill "$SRV" 2>/dev/null || true
  wait "$SRV" 2>/dev/null || true
  SRV=""
}

wait_up() {
  for _ in $(seq 1 60); do
    curl -sf "http://127.0.0.1:$PORT/api/settings" >/dev/null && return 0
    sleep 0.2
  done
  die "instance did not come up — see $WORK/server.log"
}

build() { (cd "$REPO" && cargo build -p read-stats 2>&1 | tail -3); }

cmd_run() {
  build; require_port_free; fresh_dbs
  say "http://127.0.0.1:$PORT  (databases: $WORK, discarded on next run)"
  serve
}

cmd_snapshot() {
  local name="${1:?usage: snapshot <name>}"
  local out="$WORK/snapshots/$name"
  build; require_port_free; fresh_dbs
  rm -rf "$out"; mkdir -p "$out"
  serve >"$WORK/server.log" 2>&1 &
  SRV=$!
  # The trap outlives the function, so SRV cannot be `local`.
  trap stop_server EXIT
  wait_up

  get() {
    curl -sS "http://127.0.0.1:$PORT$2" | python3 -m json.tool --sort-keys >"$out/$1.json"
  }
  local yesterday
  yesterday=$(date -d yesterday +%F)
  get settings        "/api/settings"
  get summary         "/api/summary"
  get days30          "/api/days?days=30"
  get days365         "/api/days?days=365"
  get works           "/api/works"
  get sessions_all    "/api/sessions"
  get sessions_day    "/api/sessions?date=$yesterday"
  get timeline        "/api/day/timeline?date=$yesterday"
  get timeline_bucket "/api/day/timeline?date=$yesterday&bucket_secs=300"
  get anki_summary    "/api/anki/summary"
  get lookups_summary "/api/lookups/summary"
  get dialogue        "/api/dialogue/summary?days=90"
  get reader_state    "/api/reader/state"

  stop_server
  say "snapshot -> $out ($(ls "$out" | wc -l) endpoints)"
}

cmd_diff() {
  local a="$WORK/snapshots/${1:?usage: diff <a> <b>}" b="$WORK/snapshots/${2:?}"
  [ -d "$a" ] || die "no snapshot $1"
  [ -d "$b" ] || die "no snapshot $2"
  local fail=0
  # card_age_days is floor((now - card creation)/86400) — it moves on its own
  # between two snapshots taken on different days, for no reason to do with the
  # code, so it is normalised out.
  strip() { sed 's/"card_age_days": [0-9.]*/"card_age_days": X/' "$1"; }
  for f in "$a"/*.json; do
    local n; n=$(basename "$f")
    if ! diff -q <(strip "$f") <(strip "$b/$n") >/dev/null 2>&1; then
      echo "### $n"
      diff <(strip "$f") <(strip "$b/$n") | head -40
      fail=1
    fi
  done
  [ $fail -eq 0 ] && say "IDENTICAL" || die "endpoints differ"
}

cmd_check() { cmd_snapshot current >/dev/null; cmd_diff "${1:?usage: check <baseline>}" current; }

# The client is unbundled ES modules loaded straight from disk, so a bad import
# path renders nothing at all and every JSON endpoint still passes. This is the
# check that catches it.
cmd_browser() {
  command -v chromium >/dev/null || die "chromium not installed"
  build; require_port_free; fresh_dbs
  serve >"$WORK/server.log" 2>&1 &
  SRV=$!
  # The trap outlives the function, so SRV cannot be `local`.
  trap stop_server EXIT
  wait_up

  chromium --headless --disable-gpu --no-sandbox --dump-dom --virtual-time-budget=15000 \
    "http://127.0.0.1:$PORT/" >"$WORK/dom.html" 2>"$WORK/console.log"
  stop_server

  grep -q '<div id="app"></div>' "$WORK/dom.html" &&
    die "the dashboard rendered nothing — see $WORK/console.log"
  for want in "Today" "Currently reading"; do
    grep -qF "$want" "$WORK/dom.html" || die "rendered page is missing: $want"
  done
  say "dashboard renders ($(wc -c <"$WORK/dom.html") bytes) -> $WORK/dom.html"
  # The reading view is not rendered here: it holds an SSE connection open, so
  # --dump-dom never returns. Its modules are covered by the import check.
  local fail=0
  while read -r file spec; do
    local dir url code
    dir=$(dirname "${file#"$REPO"/read-stats/static/}")
    url=$(python3 -c 'import posixpath,sys; print(posixpath.normpath(posixpath.join(*sys.argv[1:3])))' "$dir" "$spec")
    code=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:$PORT/static/$url" || true)
    [ "$code" = 200 ] || { echo "unresolved import in $file: $spec"; fail=1; }
  done < <(grep -rnoE 'from "(\.[^"]+)"' "$REPO/read-stats/static" --include="*.js" |
           sed -E 's/:[0-9]+:from "/ /; s/"$//')
  [ $fail -eq 0 ] || die "some module imports do not resolve"
}

case "${1:-run}" in
  run)      cmd_run ;;
  refresh)  REFRESH=1 freeze ;;
  snapshot) shift; cmd_snapshot "$@" ;;
  diff)     shift; cmd_diff "$@" ;;
  check)    shift; cmd_check "$@" ;;
  browser)  cmd_browser ;;
  *)        sed -n '2,20p' "${BASH_SOURCE[0]}"; exit 1 ;;
esac
