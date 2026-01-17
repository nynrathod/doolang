#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../common.sh"

PORT=3117
FILE="07_ratelimit_default_chain.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

sleep 0.5

echo ""
echo "=== Rate Limit Default Chain Test ==="
echo "Config: Default 100 requests per 60 seconds (chained)"
echo "Test: Sending 101 requests"
echo ""

success_count=0
fail_count=0
first_fail_at=0

for i in $(seq 1 101); do
  http_code=$(curl -s -w '%{http_code}' -o /dev/null "http://127.0.0.1:$PORT/ping") || true
  
  if [ "$http_code" -eq 200 ]; then
    success_count=$((success_count + 1))
    echo "Request $i: 200 OK"
  elif [ "$http_code" -eq 429 ]; then
    if [ "$first_fail_at" -eq 0 ]; then
      first_fail_at=$i
    fi
    fail_count=$((fail_count + 1))
    echo "Request $i: 429 Too Many Requests"
  else
    echo "Request $i: Unexpected HTTP $http_code"
    fail_count=$((fail_count + 1))
  fi
done

echo ""
echo "=== Test Results ==="
echo "Total Success (200): $success_count"
echo "Total Rate Limited (429): $fail_count"
echo "First rate limit hit at request: $first_fail_at"
echo ""

if [ "$success_count" -eq 100 ] && [ "$fail_count" -eq 1 ] && [ "$first_fail_at" -eq 101 ]; then
  echo "✅ Rate limit default chain test PASSED"
elif [ "$first_fail_at" -ge 100 ] && [ "$first_fail_at" -le 102 ]; then
  echo "✅ Rate limit default chain test PASSED (within range)"
else
  echo "❌ Rate limit default chain test FAILED: Expected limit at 101, got $first_fail_at"
  exit 1
fi
