#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../common.sh"

PORT=3116
FILE="06_cors_custom_chain.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo ""
echo "=== CORS Custom Chain Test ==="
echo "Config: Custom CORS (only https://allowed.com)"
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
fi

if grep -qi "access-control-allow-origin: https://allowed.com" "$HDR_FILE"; then
  echo "✓ Access-Control-Allow-Origin: https://allowed.com"
else
  echo "❌ Missing expected CORS header"
fi

rm -f "$HDR_FILE" "$BODY_FILE"

echo ""
echo "Test 2: GET /ping with denied origin"
HDR_FILE="$(mktemp)"
BODY_FILE="$(mktemp)"
HTTP_CODE="$(curl -s -D "$HDR_FILE" -o "$BODY_FILE" -w '%{http_code}' \
  "http://127.0.0.1:$PORT/ping" \
  -H "Origin: https://denied.com")" || true

echo "HTTP Status: $HTTP_CODE"
echo "Headers:"
cat "$HDR_FILE"
echo "Body:"
cat "$BODY_FILE" | pretty_json || cat "$BODY_FILE"
echo ""

# Note: Denied origin on GET should still return 200 but without CORS headers
# CORS preflight (OPTIONS) blocks, but regular requests may proceed
if [ "$HTTP_CODE" -eq 200 ]; then
  echo "✓ Got HTTP 200 (CORS is checked on preflight, not regular requests)"
elif [ "$HTTP_CODE" -eq 403 ]; then
  echo "✓ Got HTTP 403 (CORS blocked)"
else
  echo "! Got HTTP $HTTP_CODE"
fi

rm -f "$HDR_FILE" "$BODY_FILE"

echo ""
echo "Test 3: OPTIONS /ping preflight with denied origin"
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
  echo "✓ Got HTTP 403 (CORS blocked preflight)"
elif [ "$HTTP_CODE" -eq 405 ]; then
  echo "! Got HTTP 405 (OPTIONS not handled - CORS middleware issue)"
else
  echo "! Got HTTP $HTTP_CODE"
fi

rm -f "$HDR_FILE" "$BODY_FILE"

echo ""
echo "✅ CORS custom chain test completed"
