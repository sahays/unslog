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

echo ""
echo "Starting unslog (cargo run)..."
exec cargo run
