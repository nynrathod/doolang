#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../common.sh"

PORT=3118
FILE="08_ratelimit_custom_nonchain.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

sleep 0.5

echo ""
echo "=== Rate Limit Custom Non-Chain Test ==="
echo "Config: 2 requests per 30 seconds per IP (non-chained)"
echo ""

echo "Test 1: First request should pass (200)"
http_code=$(curl -s -w '%{http_code}' -o /dev/null "http://127.0.0.1:$PORT/ping") || true
if [ "$http_code" -eq 200 ]; then
  echo "✓ Request 1: 200 OK"
else
  echo "❌ Request 1: Expected 200, got $http_code"
  exit 1
fi

sleep 0.1

echo ""
echo "Test 2: Second request should pass (200)"
http_code=$(curl -s -w '%{http_code}' -o /dev/null "http://127.0.0.1:$PORT/ping") || true
if [ "$http_code" -eq 200 ]; then
  echo "✓ Request 2: 200 OK"
else
  echo "❌ Request 2: Expected 200, got $http_code"
  exit 1
fi

sleep 0.1

echo ""
echo "Test 3: Third request should be rate limited (429)"
tmpfile="$(mktemp)"
http_code=$(curl -s -w '%{http_code}' -o "$tmpfile" "http://127.0.0.1:$PORT/ping") || true
if [ "$http_code" -eq 429 ]; then
  echo "✓ Request 3: 429 Too Many Requests"
  echo "Response body:"
  cat "$tmpfile" | pretty_json || cat "$tmpfile"
  
  if ! grep -q '"status"' "$tmpfile"; then
    echo "❌ Expected RFC7807 JSON body to contain status"
    rm -f "$tmpfile"
    exit 1
  fi
  echo "✓ RFC7807 format verified"
else
  echo "❌ Request 3: Expected 429, got $http_code"
  rm -f "$tmpfile"
  exit 1
fi

rm -f "$tmpfile"

echo ""
echo "✅ Rate limit custom non-chain test completed"
