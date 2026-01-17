#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../common.sh"

PORT=3112
FILE="02_cors_custom_nonchain.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo ""
echo "=== CORS Custom Non-Chain Test ==="
echo "Config: Custom CORS (only https://allowed.com, non-chained)"
echo ""

echo "Test 1: GET /ping with allowed origin"
HDR_FILE="$(mktemp)"
BODY_FILE="$(mktemp)"
HTTP_CODE="$(curl -s -D "$HDR_FILE" -o "$BODY_FILE" -w '%{http_code}' \
  "http://127.0.0.1:$PORT/ping" \
  -H "Origin: https://allowed.com")" || true

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
  rm -f "$HDR_FILE" "$BODY_FILE"
  exit 1
fi

if grep -qi "access-control-allow-origin: https://allowed.com" "$HDR_FILE"; then
  echo "✓ Access-Control-Allow-Origin: https://allowed.com"
else
  echo "❌ Missing expected CORS header"
  rm -f "$HDR_FILE" "$BODY_FILE"
  exit 1
fi

rm -f "$HDR_FILE" "$BODY_FILE"

echo ""
echo "Test 2: OPTIONS preflight with allowed origin should return 204"
HDR_FILE="$(mktemp)"
BODY_FILE="$(mktemp)"
HTTP_CODE="$(curl -s -D "$HDR_FILE" -o "$BODY_FILE" -w '%{http_code}' \
  -X OPTIONS "http://127.0.0.1:$PORT/ping" \
  -H "Origin: https://allowed.com" \
  -H "Access-Control-Request-Method: GET")" || true

echo "HTTP Status: $HTTP_CODE"
echo "Headers:"
cat "$HDR_FILE"
echo "Body:"
cat "$BODY_FILE" | pretty_json || cat "$BODY_FILE"
echo ""

if [ "$HTTP_CODE" -eq 204 ]; then
  echo "✓ Got HTTP 204 for OPTIONS preflight"
else
  echo "! Got HTTP $HTTP_CODE (expected 204)"
fi

rm -f "$HDR_FILE" "$BODY_FILE"

echo ""
echo "Test 3: OPTIONS preflight with denied origin should return 403"
HDR_FILE="$(mktemp)"
BODY_FILE="$(mktemp)"
HTTP_CODE="$(curl -s -D "$HDR_FILE" -o "$BODY_FILE" -w '%{http_code}' \
  -X OPTIONS "http://127.0.0.1:$PORT/ping" \
  -H "Origin: https://denied.com" \
  -H "Access-Control-Request-Method: GET")" || true

echo "HTTP Status: $HTTP_CODE"
echo "Headers:"
cat "$HDR_FILE"
echo "Body:"
cat "$BODY_FILE" | pretty_json || cat "$BODY_FILE"
echo ""

if [ "$HTTP_CODE" -eq 403 ]; then
  echo "✓ Got HTTP 403 (CORS blocked)"
elif [ "$HTTP_CODE" -eq 405 ]; then
  echo "! Got HTTP 405 (OPTIONS not handled)"
else
  echo "! Got HTTP $HTTP_CODE"
fi

rm -f "$HDR_FILE" "$BODY_FILE"

echo ""
echo "✅ CORS custom non-chain test completed"
