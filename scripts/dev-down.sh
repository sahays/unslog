#!/usr/bin/env bash
set -euo pipefail

# unslog runs in the foreground (cargo run); ctrl-C is the normal "down".
# This script is here for symmetry and to optionally stop the local Mongo container.

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
