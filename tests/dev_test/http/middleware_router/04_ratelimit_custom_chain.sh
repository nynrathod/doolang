#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../common.sh"

PORT=3114
FILE="04_ratelimit_custom_chain.doo"

# Helper: expect a single HTTP status code with detailed output
expect_status() {
  local expected=$1
  shift
  if [ "$1" != "--" ]; then
    echo "expect_status usage: expect_status <code> -- <curl args...>"
    exit 2
  fi
  shift

  local tmpfile
  tmpfile="$(mktemp)"
  local http_code
  http_code="$(curl -s -w '%{http_code}' -o "$tmpfile" "$@")" || true

  if [ "$http_code" -ne "$expected" ]; then
    echo ""
    echo "❌ Expected HTTP $expected but got $http_code for: curl $*"
    echo "Response body:"
    cat "$tmpfile" | pretty_json || cat "$tmpfile"
    rm -f "$tmpfile"
    exit 1
  else
    echo ""
    echo "✅ Got expected HTTP $expected for: curl $*"
    cat "$tmpfile" | pretty_json || cat "$tmpfile"
    rm -f "$tmpfile"
  fi
}

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

# Give the server a moment to fully initialize rate limit state
sleep 0.5

echo ""
echo "=== Rate Limit Custom Chain Test ==="
echo "Config: 2 requests per 30 seconds per IP"
echo ""

echo "Test 1: First request should pass (200)"
expect_status 200 -- "http://127.0.0.1:$PORT/ping"
sleep 0.1

echo ""
echo "Test 2: Second request should pass (200)"
expect_status 200 -- "http://127.0.0.1:$PORT/ping"
sleep 0.1

echo ""
echo "Test 3: Third request should be rate limited (429 with RFC 7807 JSON)"
expect_status 429 -- "http://127.0.0.1:$PORT/ping"

echo ""
echo "✅ Rate limit custom chain test completed successfully!"
