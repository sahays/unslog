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

# ── 4. Tests ────────────────────────────────────────────────
echo "  Running tests..."
cargo test --quiet

echo "All checks passed."
