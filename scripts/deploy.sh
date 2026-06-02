#!/usr/bin/env bash
#
# Production deploy for unslog.
#
# Pipeline (any failure aborts):
#   1. host prerequisites (docker, nginx, curl, openssl)
#   2. config / .env validation
#   3. verify Postgres + Redis reachability on expected ports
#   4. ensure Postgres database exists
#   5. build app docker image
#   6. (re)launch app container with persistent data volume
#   7. wait for /health to return 200
#   8. install nginx site for unslogai.com and reload
#
# Override defaults via env vars (see DEFAULTS block below).
#
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# ── DEFAULTS ──────────────────────────────────────────────────────────────
: "${APP_NAME:=unslog}"
: "${APP_IMAGE:=unslog:latest}"
: "${APP_PORT:=3000}"                  # host port → container 3000
: "${APP_DATA_DIR:=/var/lib/unslog/data}"
: "${ENV_FILE:=$PROJECT_ROOT/.env}"

: "${PG_CONTAINER:=unslog-pg}"
: "${PG_HOST:=127.0.0.1}"
: "${PG_PORT:=5432}"
: "${PG_SUPERUSER:=unslog}"
: "${PG_DB:=unslog}"

: "${REDIS_HOST:=127.0.0.1}"
: "${REDIS_PORT:=6379}"
: "${SKIP_REDIS:=0}"                   # unslog doesn't use redis yet; set 1 to skip

: "${DOMAIN:=unslogai.com}"
: "${NGINX_SITES_AVAILABLE:=/etc/nginx/sites-available}"
: "${NGINX_SITES_ENABLED:=/etc/nginx/sites-enabled}"
: "${NGINX_SITE_TEMPLATE:=$SCRIPT_DIR/nginx/unslogai.com.conf}"

: "${HEALTH_TIMEOUT_SECS:=60}"

# ── output helpers ────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; BLUE='\033[0;34m'; NC='\033[0m'
step() { echo -e "\n${BLUE}▶ $*${NC}"; }
ok()   { echo -e "  ${GREEN}✓${NC} $*"; }
warn() { echo -e "  ${YELLOW}!${NC} $*"; }
die()  { echo -e "  ${RED}✗ $*${NC}" >&2; exit 1; }

trap 's=$?; echo -e "\n${RED}deploy aborted (exit $s) at line $LINENO${NC}" >&2' ERR

need_cmd() { command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"; }

port_open() {
  local host=$1 port=$2
  if command -v nc >/dev/null 2>&1; then nc -z -w 2 "$host" "$port" >/dev/null 2>&1; return $?; fi
  (exec 3<>"/dev/tcp/$host/$port") >/dev/null 2>&1
}

# ── 1. host prerequisites ─────────────────────────────────────────────────
check_host_prereqs() {
  step "Checking host prerequisites"
  need_cmd docker
  docker info >/dev/null 2>&1 || die "docker daemon not reachable"
  ok "docker $(docker --version | awk '{print $3}' | tr -d ,)"

  need_cmd nginx;   ok "nginx $(nginx -v 2>&1 | awk -F'/' '{print $2}')"
  need_cmd curl;    ok "curl present"
  need_cmd openssl; ok "openssl present"
  need_cmd awk;     need_cmd sed;  ok "core utils present"
}

# ── 2. config / .env ──────────────────────────────────────────────────────
load_env() {
  step "Loading environment ($ENV_FILE)"
  [[ -f "$ENV_FILE" ]] || die ".env not found at $ENV_FILE"
  set -a; # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
  : "${OPENROUTER_API_KEY:?OPENROUTER_API_KEY must be set in $ENV_FILE}"
  : "${MASTER_INVITE_CODE:?MASTER_INVITE_CODE must be set in $ENV_FILE}"
  ok ".env loaded; required secrets present"
}

# ── 3. postgres + redis port verification ────────────────────────────────
verify_postgres() {
  step "Verifying Postgres on ${PG_HOST}:${PG_PORT}"
  port_open "$PG_HOST" "$PG_PORT" || die "Postgres unreachable on ${PG_HOST}:${PG_PORT}"
  ok "tcp ${PG_HOST}:${PG_PORT} reachable"
  docker ps --format '{{.Names}}' | grep -qx "$PG_CONTAINER" \
    || die "container '$PG_CONTAINER' is not running (postgres expected in docker)"
  ok "container '$PG_CONTAINER' running"
}

verify_redis() {
  if [[ "$SKIP_REDIS" == "1" ]]; then warn "Redis check skipped (SKIP_REDIS=1)"; return; fi
  step "Verifying Redis on ${REDIS_HOST}:${REDIS_PORT}"
  port_open "$REDIS_HOST" "$REDIS_PORT" || die "Redis unreachable on ${REDIS_HOST}:${REDIS_PORT}"
  ok "tcp ${REDIS_HOST}:${REDIS_PORT} reachable"
}

# ── 4. ensure database exists ─────────────────────────────────────────────
ensure_database() {
  step "Ensuring Postgres database '$PG_DB' exists"
  local exists
  exists=$(docker exec -i "$PG_CONTAINER" \
    psql -U "$PG_SUPERUSER" -tAc "SELECT 1 FROM pg_database WHERE datname='${PG_DB}'" 2>/dev/null || true)
  if [[ "$exists" == "1" ]]; then
    ok "database '$PG_DB' already exists"
  else
    docker exec -i "$PG_CONTAINER" \
      psql -U "$PG_SUPERUSER" -d postgres -c "CREATE DATABASE \"${PG_DB}\";" >/dev/null \
      || die "failed to create database '$PG_DB'"
    ok "database '$PG_DB' created"
  fi
}

# ── 5. build image ────────────────────────────────────────────────────────
build_image() {
  step "Building docker image '$APP_IMAGE'"
  docker build -t "$APP_IMAGE" "$PROJECT_ROOT" >/dev/null
  ok "image built"
}

# ── 6. run container ──────────────────────────────────────────────────────
run_container() {
  step "Launching app container '$APP_NAME'"
  mkdir -p "$APP_DATA_DIR"
  docker rm -f "$APP_NAME" >/dev/null 2>&1 || true

  # Use host-gateway so the container can reach postgres/redis on the host's
  # docker bridge regardless of OS (Linux ≥ 20.10 and Docker Desktop).
  local db_url="postgres://${PG_SUPERUSER}:${POSTGRES_PASSWORD:-unslog}@host.docker.internal:${PG_PORT}/${PG_DB}"

  docker run -d \
    --name "$APP_NAME" \
    --restart unless-stopped \
    --add-host=host.docker.internal:host-gateway \
    -p "127.0.0.1:${APP_PORT}:3000" \
    -v "${APP_DATA_DIR}:/app/data" \
    --env-file "$ENV_FILE" \
    -e DATABASE_URL="$db_url" \
    -e HOST=0.0.0.0 \
    -e PORT=3000 \
    -e DATA_DIR=/app/data \
    -e LOG_DIR=/app/data/logs \
    -e DEV_INSECURE=false \
    "$APP_IMAGE" >/dev/null
  ok "container '$APP_NAME' started on 127.0.0.1:${APP_PORT}"
}

# ── 7. wait for health ────────────────────────────────────────────────────
wait_for_health() {
  step "Waiting for /health (up to ${HEALTH_TIMEOUT_SECS}s)"
  local url="http://127.0.0.1:${APP_PORT}/health" deadline=$(( $(date +%s) + HEALTH_TIMEOUT_SECS ))
  while (( $(date +%s) < deadline )); do
    if curl -fsS -o /dev/null "$url"; then ok "app is healthy"; return; fi
    sleep 2
  done
  docker logs --tail 100 "$APP_NAME" >&2 || true
  die "app failed health check at $url within ${HEALTH_TIMEOUT_SECS}s"
}

# ── 8. nginx site ─────────────────────────────────────────────────────────
install_nginx_site() {
  step "Installing nginx site for $DOMAIN"
  [[ -f "$NGINX_SITE_TEMPLATE" ]] || die "nginx template missing at $NGINX_SITE_TEMPLATE"
  [[ -d "$NGINX_SITES_AVAILABLE" ]] || die "$NGINX_SITES_AVAILABLE does not exist"
  [[ -d "$NGINX_SITES_ENABLED"   ]] || die "$NGINX_SITES_ENABLED does not exist"

  local target="$NGINX_SITES_AVAILABLE/$DOMAIN.conf"
  local link="$NGINX_SITES_ENABLED/$DOMAIN.conf"
  local tmp; tmp=$(mktemp)
  sed "s|__APP_PORT__|${APP_PORT}|g" "$NGINX_SITE_TEMPLATE" > "$tmp"

  if [[ -f "$target" ]] && cmp -s "$tmp" "$target"; then
    ok "nginx site already up-to-date"
    rm -f "$tmp"
  else
    install -m 0644 "$tmp" "$target"
    rm -f "$tmp"
    ok "wrote $target"
  fi

  [[ -L "$link" ]] || ln -sf "$target" "$link"
  ok "enabled $link"

  nginx -t >/dev/null 2>&1 || { nginx -t; die "nginx config test failed"; }
  systemctl reload nginx 2>/dev/null || nginx -s reload
  ok "nginx reloaded"
}

# ── orchestrate ───────────────────────────────────────────────────────────
main() {
  echo "Deploying unslog → domain=${DOMAIN} port=${APP_PORT}"
  check_host_prereqs
  load_env
  verify_postgres
  verify_redis
  ensure_database
  build_image
  run_container
  wait_for_health
  install_nginx_site
  echo -e "\n${GREEN}✓ deploy complete${NC} — http://${DOMAIN} (run certbot --nginx -d ${DOMAIN} -d www.${DOMAIN} for TLS)"
}

main "$@"
