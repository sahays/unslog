#!/usr/bin/env bash
#
# scripts/seed-import.sh — apply a seed file (produced by
# scripts/seed-export.sh) to a target Postgres database. Run on the
# production host after transferring the file.
#
# The file already carries `ON CONFLICT DO NOTHING` on every INSERT, so
# re-running on the same file is a no-op. Idempotency lives in the
# file, not in this script.
#
# This script does NOT apply schema. Migrations own the schema on the
# target — they run automatically when the app boots, or run them
# ahead of time with sqlx-cli.
#
# Required env:
#   TARGET_DATABASE_URL   Postgres connection string for the target
#
# Required tools:
#   psql (Postgres 14+ recommended)
#
# Args:
#   $1   Input seed file path (required).
#
# Usage:
#   TARGET_DATABASE_URL=postgres://bharatsc:PW@localhost:5432/unslog \
#     ./scripts/seed-import.sh ./seed.sql

set -euo pipefail

: "${TARGET_DATABASE_URL:?TARGET_DATABASE_URL must be set}"

if [ $# -lt 1 ]; then
  echo "usage: $0 <seed-file.sql>" >&2
  exit 2
fi
in=$1
if [ ! -f "$in" ]; then
  echo "error: file not found: $in" >&2
  exit 1
fi

if ! command -v psql >/dev/null 2>&1; then
  echo "error: psql not found on PATH" >&2
  exit 1
fi

echo "→ checking target connection"
psql "$TARGET_DATABASE_URL" -c 'SELECT 1' >/dev/null

echo "→ verifying target schema (looking for users table)"
schema_present=$(psql -tA "$TARGET_DATABASE_URL" \
  -c "SELECT to_regclass('public.users') IS NOT NULL")
if [ "$schema_present" != "t" ]; then
  echo "error: target database has no schema. Run migrations first (boot the app once, or use sqlx-cli)." >&2
  exit 1
fi

echo "→ applying $in (FK checks deferred via session_replication_role)"
# Some unslog tables are mutually referenced (e.g. stories ↔ story_versions
# with cyclic FKs). Wrap the apply in a transaction that puts the session
# in 'replica' role — this disables FK-check triggers for the duration so
# rows can land in any order. After COMMIT, the role resets and future
# writes enforce FKs normally. Requires the connecting role to be a
# superuser (the default postgres docker setup satisfies this; some
# managed Postgres deployments don't — if you see "permission denied to
# set session_replication_role", grant the role REPLICATION or run the
# import as the superuser).
{
  echo "BEGIN;"
  echo "SET session_replication_role = replica;"
  cat "$in"
  echo "SET session_replication_role = origin;"
  echo "COMMIT;"
} | psql -v ON_ERROR_STOP=1 -q "$TARGET_DATABASE_URL"

echo "→ row counts"
tables=$(psql -tA "$TARGET_DATABASE_URL" \
  -c "SELECT tablename FROM pg_tables WHERE schemaname='public' AND tablename NOT LIKE '\\_sqlx%' ESCAPE '\\' ORDER BY tablename")
printf "  %-32s %10s\n" "table" "rows"
printf "  %-32s %10s\n" "--------------------------------" "------"
for t in $tables; do
  n=$(psql -tA "$TARGET_DATABASE_URL" -c "SELECT count(*) FROM \"$t\"")
  printf "  %-32s %10s\n" "$t" "$n"
done

echo "✓ import complete"
