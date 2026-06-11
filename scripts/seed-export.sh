#!/usr/bin/env bash
#
# scripts/seed-export.sh — dump all data from a source Postgres database
# into a portable SQL file with idempotent INSERTs. Run locally; transfer
# the file to production; apply it with `scripts/seed-import.sh`.
#
# Schema is NOT dumped — migrations own the schema on the target. This
# script only captures rows.
#
# Idempotency lives in the FILE: every INSERT ends with
# `ON CONFLICT DO NOTHING`, so re-applying the same file on a target
# that already has those rows is a safe no-op. Postgres picks each
# table's PK / UNIQUE constraint to decide.
#
# Required env:
#   SOURCE_DATABASE_URL   Postgres connection string for the source
#
# Required tools:
#   pg_dump (Postgres 14+ recommended)
#
# Args:
#   $1   Output file path. Defaults to ./seed-<utc-timestamp>.sql.
#
# Usage:
#   SOURCE_DATABASE_URL=postgres://unslog:unslog@localhost:5432/unslog \
#     ./scripts/seed-export.sh ./seed.sql
#
# Then on prod: scp seed.sql to the box and run scripts/seed-import.sh.

set -euo pipefail

: "${SOURCE_DATABASE_URL:?SOURCE_DATABASE_URL must be set}"

if ! command -v pg_dump >/dev/null 2>&1; then
  echo "error: pg_dump not found on PATH" >&2
  exit 1
fi

out=${1:-"./seed-$(date -u +%Y%m%dT%H%M%SZ).sql"}

echo "→ dumping source → $out"
pg_dump \
  --data-only \
  --inserts \
  --no-owner \
  --no-privileges \
  --no-comments \
  --schema=public \
  "$SOURCE_DATABASE_URL" \
  | sed -E '/^INSERT INTO /s/;[[:space:]]*$/ ON CONFLICT DO NOTHING;/' \
  > "$out"

size=$(wc -c < "$out" | tr -d ' ')
echo "✓ wrote $out ($size bytes)"
echo
echo "Next: transfer $out to the production host and run"
echo "  TARGET_DATABASE_URL=postgres://... ./scripts/seed-import.sh $out"
