#!/bin/bash
# One-shot: add the CompactDef field to the "Japanese sentences" note type and
# render it at the top of the card back, above the existing def block.
#
# Run with Anki open and AnkiConnect reachable. This changes the note-type
# schema, so Anki will want a full sync afterwards. Idempotent: re-running skips
# the field if it already exists and leaves an already-patched template alone.
#
# Env: ANKI_CONNECT_URL (default http://localhost:8765)
#      MODEL_NAME       (default "Japanese sentences")
#      FIELD_NAME       (default CompactDef)
set -euo pipefail

URL="${ANKI_CONNECT_URL:-http://localhost:8765}"
MODEL="${MODEL_NAME:-Japanese sentences}"
FIELD="${FIELD_NAME:-CompactDef}"

command -v jq >/dev/null || { echo "jq is required"; exit 1; }

ac() { # action params-json
  curl -s -X POST "$URL" -d "$(jq -nc --arg a "$1" --argjson p "$2" \
    '{action:$a, version:6, params:$p}')"
}
check() { echo "$1" | jq -e '.error == null' >/dev/null || {
  echo "AnkiConnect error: $(echo "$1" | jq -r '.error')"; exit 1; }; }

curl -s -m 3 -X POST "$URL" -d '{"action":"version","version":6}' >/dev/null \
  || { echo "AnkiConnect not reachable at $URL — open Anki first."; exit 1; }

# 1) Add the field (skip if present).
FIELDS=$(ac modelFieldNames "$(jq -nc --arg m "$MODEL" '{modelName:$m}')")
check "$FIELDS"
if echo "$FIELDS" | jq -e --arg f "$FIELD" '.result | index($f)' >/dev/null; then
  echo "Field '$FIELD' already exists — skipping add."
else
  R=$(ac modelFieldAdd "$(jq -nc --arg m "$MODEL" --arg f "$FIELD" \
    '{modelName:$m, fieldName:$f, index:1}')")
  check "$R"
  echo "Added field '$FIELD' (position 2, after VocabKanji)."
fi

# 2) Patch the back template: insert a CompactDef block just before the existing
#    definitions div. Pull the current templates, edit the first card's afmt.
TPL=$(ac modelTemplates "$(jq -nc --arg m "$MODEL" '{modelName:$m}')")
check "$TPL"
CARD=$(echo "$TPL" | jq -r '.result | keys[0]')
AFMT=$(echo "$TPL" | jq -r --arg c "$CARD" '.result[$c].Back')

if echo "$AFMT" | grep -q "compact-def"; then
  echo "Template already has a compact-def block — skipping patch."
else
  BLOCK='<div class="compact-def mb-2">{{'"$FIELD"'}}</div>
    <div class="definitions mb-2">'
  # Insert before the first `<div class="definitions ...">`.
  NEW_AFMT=$(python3 - "$AFMT" "$BLOCK" <<'PY'
import sys, re
afmt, block = sys.argv[1], sys.argv[2]
# Match the opening definitions div (any attrs) once.
m = re.search(r'<div class="definitions[^"]*"[^>]*>', afmt)
if not m:
    sys.stderr.write("could not find definitions div; template unchanged\n")
    print(afmt, end="")
else:
    print(afmt[:m.start()] + block + afmt[m.end():], end="")
PY
)
  QFMT=$(echo "$TPL" | jq -r --arg c "$CARD" '.result[$c].Front')
  R=$(ac updateModelTemplates "$(jq -nc --arg m "$MODEL" --arg c "$CARD" \
    --arg f "$QFMT" --arg b "$NEW_AFMT" \
    '{model:{name:$m, templates:{($c):{Front:$f, Back:$b}}}}')")
  check "$R"
  echo "Patched card '$CARD' back template: CompactDef now renders above the def block."
fi

echo "Done. Sync when ready (this changed the note-type schema)."
