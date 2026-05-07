#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

if [[ ! -d "node_modules" ]]; then
  echo "Installing npm devDeps..."
  npm install --silent
fi

echo "Building release binary..."
cargo build --release

echo ""
echo "Built target/release/unslog"
ls -lh target/release/unslog
