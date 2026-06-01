#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

echo "Running pre-deploy checks..."

# ── 1. Format ────────────────────────────────────────────────
echo "  Formatting Rust (cargo fmt)..."
cargo fmt --all

# ── 2. Lint — Rust (strict) ─────────────────────────────────
echo "  Linting Rust (cargo clippy — strict)..."
cargo clippy --all-targets -- \
  -D warnings \
  -A clippy::needless_pass_by_value \
  -A clippy::redundant_closure_for_method_calls

# ── 3. Build ────────────────────────────────────────────────
echo "  Building (cargo check)..."
cargo check

# ── 4. Postgres reachability — required by the cross-tenant
#       integration suite (`#[sqlx::test]`), which spins up a
#       per-test database on the live Postgres instance and
#       therefore cannot run offline.
# ─────────────────────────────────────────────────────────────
echo "  Checking Postgres reachability for sqlx::test..."
PG_HOST="${PG_HOST:-localhost}"
PG_PORT="${PG_PORT:-5433}"
if command -v pg_isready &>/dev/null; then
  if ! pg_isready -h "$PG_HOST" -p "$PG_PORT" -q; then
    echo "ERROR: Postgres unreachable on ${PG_HOST}:${PG_PORT}."
    echo "       The cross-tenant isolation suite requires a live Postgres."
    echo "       Start one with:"
    echo "         docker run -d -p 5433:5432 \\"
    echo "           -e POSTGRES_USER=unslog -e POSTGRES_PASSWORD=unslog \\"
    echo "           -e POSTGRES_DB=unslog --name unslog-pg postgres:17"
    exit 1
  fi
elif command -v nc &>/dev/null; then
  if ! nc -z "$PG_HOST" "$PG_PORT" 2>/dev/null; then
    echo "ERROR: Postgres tcp port ${PG_HOST}:${PG_PORT} unreachable."
    exit 1
  fi
else
  echo "WARN: neither pg_isready nor nc available; skipping Postgres check"
fi

# ── 5. Tests ────────────────────────────────────────────────
echo "  Running tests..."
cargo test --quiet

echo "All checks passed."
