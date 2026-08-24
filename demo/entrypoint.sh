#!/bin/sh
# Copy the seed out of its read-only mount before starting.
#
# The seed has to stay untouched — it is the one copy, and every visitor shares
# this instance — but a new release may add a migration, and those have to run
# somewhere. So they run against a throwaway copy: the schema keeps up with
# whatever release is deployed, and a restart puts the data back exactly as the
# seed left it. KOTODEX_DEMO refuses the writes that would come from visitors.
set -e

SCRATCH=/var/lib/kotodex
rm -rf "$SCRATCH"
mkdir -p "$SCRATCH/covers"
cp /seed/knowledge.db /seed/read-stats.db "$SCRATCH/"
cp /seed/covers/*.jpg "$SCRATCH/covers/" 2>/dev/null || true
chmod -R u+w "$SCRATCH"

exec /app/target/release/read-stats
