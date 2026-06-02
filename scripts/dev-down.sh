#!/usr/bin/env bash
set -euo pipefail

# unslog runs in the foreground (cargo run); ctrl-C is the normal "down".
# This script is here for symmetry and to optionally stop the local
# Postgres container.

# ── Postgres ────────────────────────────────────────────────────────────
PG_NAME="unslog-pg"
if docker ps --format '{{.Names}}' 2>/dev/null | grep -q "^${PG_NAME}$"; then
  echo "Found running ${PG_NAME} container."
  echo "Note: other local projects may share a Postgres instance — only stop if you're sure."
  read -rp "Stop it? [y/N] " yn
  if [[ "$yn" =~ ^[Yy]$ ]]; then
    docker stop "${PG_NAME}" >/dev/null
    echo "Stopped."
  fi
else
  echo "No running ${PG_NAME} container."
fi
