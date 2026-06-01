#!/usr/bin/env bash
set -euo pipefail

# unslog runs in the foreground (cargo run); ctrl-C is the normal "down".
# This script is here for symmetry and to optionally stop the local
# Mongo / Postgres containers.

# ── MongoDB ─────────────────────────────────────────────────────────────
if docker ps --format '{{.Image}}' 2>/dev/null | grep -q '^mongo'; then
  CID=$(docker ps --filter ancestor=mongo --format '{{.ID}}' | head -1)
  echo "Found running mongo container ($CID)."
  echo "Note: this Mongo instance is shared across local projects — only stop if you're sure."
  read -rp "Stop it? [y/N] " yn
  if [[ "$yn" =~ ^[Yy]$ ]]; then
    docker stop "$CID"
    echo "Stopped."
  fi
else
  echo "No running mongo container."
fi

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
