#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

pass() { echo -e "  ${GREEN}[OK]${NC} $1"; }
fail() { echo -e "  ${RED}[FAIL]${NC} $1"; }
warn() { echo -e "  ${YELLOW}[WARN]${NC} $1"; }

ERRORS=0

echo "Pre-flight dependency checks"
echo "============================="

echo "Cargo:"
if command -v cargo &>/dev/null; then pass "cargo available ($(cargo --version))"; else fail "cargo not found"; ERRORS=$((ERRORS+1)); fi

echo "Node / npm:"
if command -v npm &>/dev/null; then pass "npm available ($(npm --version))"; else fail "npm not found (Tailwind build needs it)"; ERRORS=$((ERRORS+1)); fi

echo "Docker:"
if command -v docker &>/dev/null && docker info &>/dev/null; then
  pass "docker is installed and running"
else
  fail "docker is not running (MongoDB container needs it)"
  ERRORS=$((ERRORS+1))
fi

echo "MongoDB container:"
if docker ps --format '{{.Image}}' 2>/dev/null | grep -q '^mongo'; then
  pass "MongoDB container is running on $(docker ps --format '{{.Ports}}' --filter ancestor=mongo | head -1)"
else
  warn "no running mongo:* container detected"
  warn "start one with: docker run -d -p 27017:27017 --name mongo mongo:latest"
fi

echo "pdftotext (poppler) — optional fallback for PDF extraction:"
if command -v pdftotext &>/dev/null; then pass "pdftotext available"; else warn "pdftotext not installed (brew install poppler) — pdf-extract crate will be sole path"; fi

echo "Environment:"
if [[ -f "$PROJECT_ROOT/.env" ]]; then
  pass ".env exists"
  if grep -qE '^OPENROUTER_API_KEY=.+' "$PROJECT_ROOT/.env"; then
    pass "OPENROUTER_API_KEY is set"
  else
    warn "OPENROUTER_API_KEY is empty — AI features will be unavailable until set"
  fi
else
  warn ".env not found — copying from .env.example"
  cp "$PROJECT_ROOT/.env.example" "$PROJECT_ROOT/.env"
  pass "created .env from .env.example (review and update values)"
fi

echo "node_modules:"
if [[ -d "$PROJECT_ROOT/node_modules" ]]; then
  pass "node_modules present"
else
  warn "node_modules missing — run: npm install"
fi

echo ""
if [[ $ERRORS -gt 0 ]]; then
  echo -e "${RED}$ERRORS critical check(s) failed.${NC}"
  exit 1
else
  echo -e "${GREEN}All checks passed.${NC}"
fi
