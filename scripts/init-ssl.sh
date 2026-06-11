#!/usr/bin/env bash
#
# One-shot TLS bootstrap for unslogai.com using bharatsc's certbot volumes
# and GCP DNS-01 service account key. Run BEFORE the first deploy.
#
# After this completes successfully, scripts/deploy.sh's nginx -t will
# pass and the vhost will load on bharatsc-nginx.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

if [[ -f ".env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

DOMAIN="${DOMAIN:-unslogai.com}"
CERTBOT_EMAIL="${CERTBOT_EMAIL:-admin@${DOMAIN}}"
BHARATSC_DIR="${BHARATSC_DIR:-$(dirname "$PROJECT_ROOT")/bharatsc}"

GCP_KEY="$BHARATSC_DIR/bharatsc-gcp-service-account-key.json"
if [[ ! -f "$GCP_KEY" ]]; then
  echo "ERROR: GCP service account key not found at $GCP_KEY" >&2
  echo "DNS-01 challenge requires a GCP SA key with DNS Admin role." >&2
  exit 1
fi

echo "Acquiring SSL cert for $DOMAIN (and www.$DOMAIN) via DNS-01..."
echo

docker run --rm \
  -v "bharatsc_certbot-etc:/etc/letsencrypt" \
  -v "bharatsc_certbot-var:/var/lib/letsencrypt" \
  -v "$GCP_KEY:/etc/gcp/sa-key.json:ro" \
  certbot/dns-google \
  certonly \
  --dns-google \
  --dns-google-credentials /etc/gcp/sa-key.json \
  --dns-google-propagation-seconds 60 \
  -d "$DOMAIN" \
  -d "www.$DOMAIN" \
  --email "$CERTBOT_EMAIL" \
  --agree-tos \
  --no-eff-email \
  --non-interactive

echo
echo "Certificate acquired for $DOMAIN."
echo
echo "Reload nginx so it picks up the new cert:"
echo "  docker compose -f $BHARATSC_DIR/docker-compose.yml exec nginx nginx -s reload"
echo
echo "Then deploy unslog:"
echo "  ./scripts/deploy.sh"
