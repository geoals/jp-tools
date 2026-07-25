#!/usr/bin/env bash
# Move the shared tables into jp-core's knowledge.db.
#
#   dictionaries  yt-mine.db     -> knowledge.db   (dictionaries, entries, pitch, frequency)
#   reading       read-stats.db  -> knowledge.db   (works, lines, sessions -> manual_sessions,
#                                                   anki_notes, word_days, lookups)
#
# Both steps are idempotent: a table already present in knowledge.db with rows
# in it is left alone, so a re-run after a partial failure resumes rather than
# duplicating. Nothing is dropped from the source until the copy is verified
# row-for-row.
#
# Usage:
#   scripts/migrate-knowledge-db.sh              # both steps
#   scripts/migrate-knowledge-db.sh reading      # just the read-stats tables
#   scripts/migrate-knowledge-db.sh dictionaries # just the dictionary cache
#   DRY_RUN=1 scripts/migrate-knowledge-db.sh    # report what it would do
#
# Environment (defaults match the services):
#   JP_TOOLS_KNOWLEDGE_DB_PATH  ~/.local/share/jp-tools/knowledge.db
#   JP_TOOLS_STATS_DB_PATH      ~/.local/share/jp-tools/read-stats.db
#   JP_TOOLS_DB_PATH            ~/.local/share/jp-tools/yt-mine.db
set -euo pipefail

DATA_DIR="$HOME/.local/share/jp-tools"
KNOWLEDGE="${JP_TOOLS_KNOWLEDGE_DB_PATH:-$DATA_DIR/knowledge.db}"
STATS="${JP_TOOLS_STATS_DB_PATH:-$DATA_DIR/read-stats.db}"
YTMINE="${JP_TOOLS_DB_PATH:-$DATA_DIR/yt-mine.db}"
DRY_RUN="${DRY_RUN:-}"
STEP="${1:-all}"

READING_TABLES=(works lines anki_notes word_days lookups)
DICT_TABLES=(dictionaries dictionary_entries dictionary_pitch dictionary_frequency)

say()  { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

q()  { sqlite3 "$1" "$2"; }
rows() { sqlite3 "$1" "SELECT COUNT(*) FROM $2" 2>/dev/null || echo missing; }
has_table() {
  [ "$(sqlite3 "$1" "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='$2'")" != 0 ]
}

# --- safety ----------------------------------------------------------------
# The logger and the capture script read and write these files. Migrating
# underneath a running logger would strand every line it writes next in the old
# database, where nothing will ever read them again.
check_idle() {
  local busy=""
  pgrep -f "vn-ws-logger.py" >/dev/null 2>&1 && busy="$busy vn-ws-logger"
  pgrep -x "Textractor.exe" >/dev/null 2>&1 && busy="$busy Textractor"
  pgrep -f "target/debug/read-stats" >/dev/null 2>&1 && busy="$busy read-stats"
  pgrep -f "target/debug/yt-mine" >/dev/null 2>&1 && busy="$busy yt-mine"
  pgrep -f "target/debug/manga-mine" >/dev/null 2>&1 && busy="$busy manga-mine"
  if [ -n "$busy" ]; then
    die "still running:$busy
  Stop the VN and Textractor, then:
    systemctl --user stop vn-buffer
    scripts/start-all.sh stop
  and run this again. (The logger must be restarted afterwards so it writes to
  the new database — see vn-mine/README.md.)"
  fi
}

backup() {
  local db="$1"
  [ -f "$db" ] || return 0
  local dest="$db.pre-knowledge-$(date +%Y%m%d-%H%M%S)"
  say "backing up $(basename "$db") -> $(basename "$dest")"
  [ -n "$DRY_RUN" ] || sqlite3 "$db" ".backup '$dest'"
}

# Create knowledge.db with jp-core's own schema.
#
# The destination DDL has to come from the migrations, not from `CREATE TABLE AS
# SELECT` on the source: that form copies column names and types and drops
# everything else — the INTEGER PRIMARY KEY that makes `lines.id` autoincrement,
# the (lemma, date) key `word_days`' upsert conflicts on, and every index. The
# tables would look right and misbehave on the first write.
create_schema() {
  local dir
  dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/jp-core/migrations/knowledge"
  [ -d "$dir" ] || die "migrations not found at $dir"
  say "applying jp-core's knowledge schema"
  [ -n "$DRY_RUN" ] && return 0
  for f in "$dir"/*.sql; do
    sqlite3 "$KNOWLEDGE" < "$f" || die "failed to apply $(basename "$f")"
  done
}

# Column names of <db>.<table>, one per line.
cols_of() { sqlite3 "$1" "SELECT name FROM pragma_table_info('$2')"; }

# copy_table <source-db> <table> [<dest-table>]
#
# Copies by explicit column name, so the two schemas need not agree on column
# order, and a source column with nowhere to go is an error rather than silent
# data loss. Skips a destination that already holds rows, which is what makes a
# re-run after a partial failure safe.
copy_table() {
  local src="$1" table="$2" dest="${3:-$2}"
  if ! has_table "$src" "$table"; then
    warn "$table: not in $(basename "$src") — skipping"
    return 0
  fi
  local n_src n_dest
  n_src=$(rows "$src" "$table")
  n_dest=$(rows "$KNOWLEDGE" "$dest")
  if [ "$n_dest" != "missing" ] && [ "$n_dest" -gt 0 ] 2>/dev/null; then
    say "$dest: already holds $n_dest rows — skipping (source has $n_src)"
    return 0
  fi
  say "$table -> $dest ($n_src rows)"
  [ -n "$DRY_RUN" ] && return 0

  local shared="" missing=""
  local dest_cols
  dest_cols=$(cols_of "$KNOWLEDGE" "$dest")
  while read -r col; do
    [ -z "$col" ] && continue
    if grep -qx "$col" <<<"$dest_cols"; then
      shared="${shared:+$shared, }$col"
    else
      missing="${missing:+$missing }$col"
    fi
  done <<<"$(cols_of "$src" "$table")"
  [ -z "$missing" ] || die "$table has columns $dest cannot hold: $missing"
  [ -n "$shared" ] || die "$table: no columns in common with $dest"

  sqlite3 "$KNOWLEDGE" <<SQL
ATTACH DATABASE '$src' AS src;
INSERT INTO main.$dest ($shared) SELECT $shared FROM src.$table;
DETACH DATABASE src;
SQL

  local n_after
  n_after=$(rows "$KNOWLEDGE" "$dest")
  [ "$n_after" = "$n_src" ] ||
    die "$dest: copied $n_after of $n_src rows — source left untouched, nothing dropped"
}

# drop_table <db> <table> <expected-rows-in-knowledge> [<dest-table>]
drop_table() {
  local db="$1" table="$2" dest="${3:-$2}"
  has_table "$db" "$table" || return 0
  # In a dry run nothing was copied, so the row-count check below has nothing to
  # compare against; report the intent instead.
  if [ -n "$DRY_RUN" ]; then
    say "would drop $table from $(basename "$db") once $dest verifies"
    return 0
  fi
  local n_src n_dest
  n_src=$(rows "$db" "$table")
  n_dest=$(rows "$KNOWLEDGE" "$dest")
  if [ "$n_dest" = "missing" ] || [ "$n_dest" -lt "$n_src" ] 2>/dev/null; then
    die "refusing to drop $table: knowledge.db has $n_dest rows, source has $n_src"
  fi
  say "dropping $table from $(basename "$db") (safely copied: $n_dest rows)"
  [ -n "$DRY_RUN" ] || sqlite3 "$db" "DROP TABLE $table"
}

migrate_reading() {
  [ -f "$STATS" ] || { warn "no $STATS — nothing to migrate"; return 0; }
  say "reading tables: $(basename "$STATS") -> $(basename "$KNOWLEDGE")"
  backup "$STATS"
  for t in "${READING_TABLES[@]}"; do copy_table "$STATS" "$t"; done
  # Renamed on the way across: a *derived* session is the other meaning of the
  # word in read-stats, and that one is computed rather than stored.
  copy_table "$STATS" sessions manual_sessions
  for t in "${READING_TABLES[@]}"; do drop_table "$STATS" "$t"; done
  drop_table "$STATS" sessions manual_sessions
  [ -n "$DRY_RUN" ] || sqlite3 "$STATS" "VACUUM"
}

migrate_dictionaries() {
  [ -f "$YTMINE" ] || { warn "no $YTMINE — nothing to migrate"; return 0; }
  say "dictionary cache: $(basename "$YTMINE") -> $(basename "$KNOWLEDGE")"
  local size
  size=$(du -h "$YTMINE" | cut -f1)
  say "this copies most of $size and takes a few minutes"
  backup "$YTMINE"
  for t in "${DICT_TABLES[@]}"; do copy_table "$YTMINE" "$t"; done
  for t in "${DICT_TABLES[@]}"; do drop_table "$YTMINE" "$t"; done
  # `vocabulary` stays in yt-mine.db: it is empty, and the ledger that replaces
  # it is designed but not built (spec/knowledge-db.md).
  [ -n "$DRY_RUN" ] || { say "reclaiming space in $(basename "$YTMINE")"; sqlite3 "$YTMINE" "VACUUM"; }
}

command -v sqlite3 >/dev/null || die "sqlite3 not found"
[ -n "$DRY_RUN" ] && say "DRY RUN — nothing will be written"
# The running-process check only makes sense for the databases those processes
# actually use. Pointed anywhere else (a copy, a test fixture) there is nothing
# to race with, and requiring the stack to be down to migrate a copy would make
# rehearsing the migration impossible.
if [ "$KNOWLEDGE" = "$DATA_DIR/knowledge.db" ] && [ "$STATS" = "$DATA_DIR/read-stats.db" ]; then
  [ -n "$DRY_RUN" ] || check_idle
else
  say "non-default paths — skipping the running-process check"
fi

mkdir -p "$(dirname "$KNOWLEDGE")"
create_schema

case "$STEP" in
  reading)      migrate_reading ;;
  dictionaries) migrate_dictionaries ;;
  all)          migrate_reading; migrate_dictionaries ;;
  *)            die "unknown step: $STEP (reading | dictionaries | all)" ;;
esac

say "done. Contents of $(basename "$KNOWLEDGE"):"
for t in "${READING_TABLES[@]}" manual_sessions "${DICT_TABLES[@]}"; do
  printf '  %-22s %s\n' "$t" "$(rows "$KNOWLEDGE" "$t")"
done
cat <<'NEXT'

Next:
  1. cargo build
  2. scripts/start-all.sh start
  3. systemctl --user start vn-buffer     (the logger now writes to knowledge.db)
NEXT
