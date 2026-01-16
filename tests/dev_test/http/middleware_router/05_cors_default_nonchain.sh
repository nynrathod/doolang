#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../common.sh"

PORT=3115
FILE="05_cors_default_nonchain.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo ""
echo "=== CORS Default Non-Chain Test ==="
echo "Config: Default CORS (all origins allowed)"
echo ""

echo "Test 1: GET /ping with Origin header"
HDR_FILE="$(mktemp)"
BODY_FILE="$(mktemp)"
HTTP_CODE="$(curl -s -D "$HDR_FILE" -o "$BODY_FILE" -w '%{http_code}' \
  "http://127.0.0.1:$PORT/ping" \
  -H "Origin: https://example.com")" || true

echo "HTTP Status: $HTTP_CODE"
echo "Headers:"
cat "$HDR_FILE"
echo "Body:"
cat "$BODY_FILE" | pretty_json || cat "$BODY_FILE"
echo ""

# Verify expectations
if [ "$HTTP_CODE" -eq 200 ]; then
  echo "✓ Got HTTP 200"
else
  echo "❌ Expected HTTP 200 but got $HTTP_CODE"
fi

if grep -qi "access-control-allow-origin: \*" "$HDR_FILE"; then
  echo "✓ Access-Control-Allow-Origin: *"
else
  echo "❌ Missing Access-Control-Allow-Origin: *"
fi

rm -f "$HDR_FILE" "$BODY_FILE"

echo ""
echo "Test 2: GET /ping without Origin header"
HDR_FILE="$(mktemp)"
BODY_FILE="$(mktemp)"
HTTP_CODE="$(curl -s -D "$HDR_FILE" -o "$BODY_FILE" -w '%{http_code}' \
  "http://127.0.0.1:$PORT/ping")" || true

echo "HTTP Status: $HTTP_CODE"
echo "Headers:"
cat "$HDR_FILE"
echo "Body:"
cat "$BODY_FILE" | pretty_json || cat "$BODY_FILE"
echo ""

if [ "$HTTP_CODE" -eq 200 ]; then
  echo "✓ Got HTTP 200"
else
  echo "❌ Expected HTTP 200 but got $HTTP_CODE"
fi

rm -f "$HDR_FILE" "$BODY_FILE"

echo ""
echo "✅ CORS default non-chain test completed"
