#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

"$SCRIPT_DIR/check-deps.sh"

if [[ ! -d "node_modules" ]]; then
  echo "Installing npm devDeps for Tailwind..."
  npm install --silent
fi

# ── MongoDB ─────────────────────────────────────────────────────────────
# Keep using whatever mongo:* container the user already runs (shared
# across local projects). Only start a new one if none is running.
if ! docker ps --format '{{.Image}}' 2>/dev/null | grep -q '^mongo'; then
  if docker ps -a --format '{{.Names}}' 2>/dev/null | grep -q '^mongo$'; then
    echo "Starting existing 'mongo' container..."
    docker start mongo >/dev/null
  else
    echo "Starting a fresh mongo:latest container..."
    docker run -d -p 27017:27017 --name mongo mongo:latest >/dev/null
  fi
fi

# ── Postgres (Phase A: dev only, no app code reads from it yet) ─────────
# Project-scoped container name (`unslog-pg`) so it doesn't collide with
# any other local Postgres the user might be running.
PG_NAME="unslog-pg"
if ! docker ps --format '{{.Names}}' 2>/dev/null | grep -q "^${PG_NAME}$"; then
  if docker ps -a --format '{{.Names}}' 2>/dev/null | grep -q "^${PG_NAME}$"; then
    echo "Starting existing '${PG_NAME}' container..."
    docker start "${PG_NAME}" >/dev/null
  else
    echo "Starting a fresh postgres:17 container as '${PG_NAME}'..."
    docker run -d \
      -p 5432:5432 \
      -e POSTGRES_USER=unslog \
      -e POSTGRES_PASSWORD=unslog \
      -e POSTGRES_DB=unslog \
      --name "${PG_NAME}" \
      postgres:17 >/dev/null
  fi
fi

echo ""
echo "Starting unslog (cargo run)..."
exec cargo run
