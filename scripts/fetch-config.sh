#!/usr/bin/env bash
# Quick helper to fetch the upstream config and verify the parser locally.
set -euo pipefail
URL="${1:-https://incss.ru/vless.conf}"
echo "Fetching $URL"
curl -sSL "$URL" | tee /tmp/vless.conf | jq '.bridge_rsa_id, .bridge_ed25519_id, .doh_server, .outbounds[0].streamSettings.realitySettings'
