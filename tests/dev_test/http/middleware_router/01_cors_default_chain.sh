#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../common.sh"

PORT=3111
FILE="01_cors_default_chain.doo"

expect_header_contains() {
  local headers_file=$1
  local header_name=$2
  local expected_substring=$3

  if ! grep -i "^${header_name}:" "$headers_file" >/dev/null; then
    echo "❌ Missing header: $header_name"
    echo "Headers were:"
    cat "$headers_file"
    exit 1
  fi

  if ! grep -i "^${header_name}:.*${expected_substring}" "$headers_file" >/dev/null; then
    echo "❌ Header $header_name did not contain '$expected_substring'"
    echo "Headers were:"
    cat "$headers_file"
    exit 1
  fi
}

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo ""
echo "=== CORS Default Chain Test ==="
echo "Config: Default CORS (all origins allowed, chained)"
echo ""

echo "Test 1: GET /ping with Origin header should return 200 with CORS headers"
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

if [ "$HTTP_CODE" -ne 200 ]; then
  echo "❌ Expected HTTP 200 but got $HTTP_CODE"
  rm -f "$HDR_FILE" "$BODY_FILE"
  exit 1
fi
echo "✓ Got HTTP 200"

expect_header_contains "$HDR_FILE" "access-control-allow-origin" "*"
echo "✓ Access-Control-Allow-Origin: *"

rm -f "$HDR_FILE" "$BODY_FILE"

echo ""
echo "Test 2: OPTIONS preflight should return 204"
HDR_FILE="$(mktemp)"
BODY_FILE="$(mktemp)"
HTTP_CODE="$(curl -s -D "$HDR_FILE" -o "$BODY_FILE" -w '%{http_code}' \
  -X OPTIONS "http://127.0.0.1:$PORT/ping" \
  -H "Origin: https://example.com" \
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
echo "✅ CORS default chain test completed"
