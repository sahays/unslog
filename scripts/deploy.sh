#!/usr/bin/env bash
#
# Production deploy for unslog (alongside bharatsc).
#
# Pipeline (any failure aborts):
#   1. host prerequisites (docker + compose, curl)
#   2. .env loaded; required secrets present
#   3. bharatsc reachable (shared network + postgres service running)
#   4. ensure Postgres database 'unslog' exists in bharatsc-postgres
#   5. render nginx vhost (substitute __PORT__)
#   6. compose build + up -d
#   7. wait for /health
#   8. install vhost into bharatsc/nginx/conf.d/ and reload bharatsc-nginx
#
# Flags:
#   --no-build       skip docker compose build
#   --foreground|-f  run compose in foreground (skips health-check and nginx install)
#
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

SKIP_BUILD=false
FOREGROUND=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build) SKIP_BUILD=true; shift ;;
    --foreground|-f) FOREGROUND=true; shift ;;
    *) echo "Unknown flag: $1" >&2; exit 2 ;;
  esac
done

cd "$PROJECT_ROOT"

# ── output helpers ────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; BLUE='\033[0;34m'; NC='\033[0m'
step() { echo -e "\n${BLUE}▶ $*${NC}"; }
ok()   { echo -e "  ${GREEN}✓${NC} $*"; }
warn() { echo -e "  ${YELLOW}!${NC} $*"; }
die()  { echo -e "  ${RED}✗ $*${NC}" >&2; exit 1; }
trap 's=$?; echo -e "\n${RED}deploy aborted (exit $s) at line $LINENO${NC}" >&2' ERR

# ── 1. host prerequisites ─────────────────────────────────────────────────
step "Checking host prerequisites"
command -v docker >/dev/null 2>&1 || die "docker is required"
docker info >/dev/null 2>&1 || die "docker daemon not reachable"
docker compose version >/dev/null 2>&1 || die "docker compose plugin not found"
command -v curl >/dev/null 2>&1 || die "curl is required"
ok "docker, compose plugin, curl present"

# ── 2. .env ───────────────────────────────────────────────────────────────
step "Loading .env"
[[ -f "$PROJECT_ROOT/.env" ]] || die ".env not found at $PROJECT_ROOT/.env"
set -a
# shellcheck disable=SC1091
source "$PROJECT_ROOT/.env"
set +a
: "${DOMAIN:?DOMAIN must be set in .env (e.g. unslogai.com)}"
: "${OPENROUTER_API_KEY:?OPENROUTER_API_KEY must be set in .env}"
: "${MASTER_INVITE_CODE:?MASTER_INVITE_CODE must be set in .env}"
: "${DATABASE_URL:?DATABASE_URL must be set in .env (e.g. postgres://bharatsc:PW@postgres:5432/unslog)}"
PORT="${PORT:-3000}"
BHARATSC_DIR="${BHARATSC_DIR:-$(dirname "$PROJECT_ROOT")/bharatsc}"
ok ".env loaded — DOMAIN=$DOMAIN PORT=$PORT"

# ── 3. bharatsc reachable ─────────────────────────────────────────────────
step "Verifying bharatsc infrastructure"
docker network inspect shared >/dev/null 2>&1 \
  || die "docker network 'shared' missing — start bharatsc first (cd $BHARATSC_DIR && ./scripts/deploy.sh)"
ok "'shared' network present"

[[ -d "$BHARATSC_DIR" ]] || die "BHARATSC_DIR not found: $BHARATSC_DIR"

PG_CID=$(docker compose -f "$BHARATSC_DIR/docker-compose.yml" ps -q postgres 2>/dev/null || true)
[[ -n "$PG_CID" ]] || die "bharatsc postgres container is not running"
ok "bharatsc-postgres reachable ($PG_CID)"

NGINX_CID=$(docker compose -f "$BHARATSC_DIR/docker-compose.yml" ps -q nginx 2>/dev/null || true)
[[ -n "$NGINX_CID" ]] || die "bharatsc nginx container is not running"
ok "bharatsc-nginx reachable ($NGINX_CID)"

# ── 4. ensure database exists ─────────────────────────────────────────────
step "Ensuring Postgres database 'unslog' exists"
PG_USER="${POSTGRES_USER:-bharatsc}"
EXISTS=$(docker exec -i "$PG_CID" \
  psql -U "$PG_USER" -tAc "SELECT 1 FROM pg_database WHERE datname='unslog'" 2>/dev/null || true)
if [[ "$EXISTS" == "1" ]]; then
  ok "database 'unslog' already exists"
else
  docker exec -i "$PG_CID" \
    psql -U "$PG_USER" -d postgres -c 'CREATE DATABASE "unslog"' >/dev/null \
    || die "failed to create database 'unslog' (check POSTGRES_USER and that the role can CREATEDB)"
  ok "database 'unslog' created"
fi

# ── 5. render nginx vhost ─────────────────────────────────────────────────
step "Rendering nginx vhost"
TEMPLATE="$PROJECT_ROOT/nginx/conf.d/unslogai.com.conf.template"
GENERATED="$PROJECT_ROOT/nginx/conf.d/unslogai.com.conf"
[[ -f "$TEMPLATE" ]] || die "vhost template missing at $TEMPLATE"
sed "s/__PORT__/${PORT}/g" "$TEMPLATE" > "$GENERATED"
ok "rendered → $GENERATED"

# ── 6. compose build + up ─────────────────────────────────────────────────
if [[ "$SKIP_BUILD" == false ]]; then
  step "Building image (docker compose build web)"
  docker compose build web
  ok "image built"
fi

if [[ "$FOREGROUND" == true ]]; then
  step "Starting unslog-web (foreground — skips health-check + nginx install)"
  exec docker compose up
fi

step "Starting unslog-web"
docker compose up -d
ok "container up"

# ── 7. wait for health ────────────────────────────────────────────────────
step "Waiting for /health"
HEALTHY=false
for _ in $(seq 1 30); do
  if docker compose exec -T web sh -c "curl -sf http://localhost:${PORT}/health" >/dev/null 2>&1; then
    HEALTHY=true; break
  fi
  sleep 2
done
if [[ "$HEALTHY" == false ]]; then
  docker compose logs --tail=120 web >&2 || true
  die "app failed health check within 60s"
fi
ok "unslog-web is healthy"

# ── 8. install vhost into bharatsc nginx ──────────────────────────────────
step "Installing vhost into bharatsc nginx"
BHARATSC_CONFD="$BHARATSC_DIR/nginx/conf.d"
[[ -d "$BHARATSC_CONFD" ]] || die "bharatsc nginx/conf.d not found at $BHARATSC_CONFD"
TARGET="$BHARATSC_CONFD/unslogai.com.conf"

if diff -q "$GENERATED" "$TARGET" >/dev/null 2>&1; then
  ok "vhost already up to date in bharatsc"
else
  cp "$GENERATED" "$TARGET"
  ok "copied vhost → $TARGET"
fi

if docker compose -f "$BHARATSC_DIR/docker-compose.yml" exec -T nginx nginx -t 2>/dev/null; then
  docker compose -f "$BHARATSC_DIR/docker-compose.yml" exec -T nginx nginx -s reload
  ok "bharatsc nginx reloaded"
else
  warn "nginx -t failed — likely missing TLS cert for $DOMAIN"
  warn "run scripts/init-ssl.sh first, then re-run this deploy"
  exit 1
fi

echo
docker compose ps
echo -e "\n${GREEN}✓ deploy complete${NC} — https://${DOMAIN}"
