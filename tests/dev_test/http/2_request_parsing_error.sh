#!/bin/bash
set -euo pipefail

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3102
FILE="2_request_parsing_error.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

# Helper: expect a single HTTP status code
# Usage: expect_status <expected_code> -- <curl-args...>
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
  # Capture body to tmpfile, and http code to variable
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

# Helper: expect one of multiple acceptable codes (regex)
# Usage: expect_status_one_of '<regex>' -- <curl-args...>
expect_status_one_of() {
  local regex="$1"
  shift
  if [ "$1" != "--" ]; then
    echo "expect_status_one_of usage: expect_status_one_of <regex> -- <curl args...>"
    exit 2
  fi
  shift

  local tmpfile
  tmpfile="$(mktemp)"
  local http_code
  http_code="$(curl -s -w '%{http_code}' -o "$tmpfile" "$@")" || true

  if ! echo "$http_code" | grep -Eq "$regex"; then
    echo ""
    echo "❌ Expected HTTP matching /$regex/ but got $http_code for: curl $*"
    echo "Response body:"
    cat "$tmpfile" | pretty_json || cat "$tmpfile"
    rm -f "$tmpfile"
    exit 1
  else
    echo ""
    echo "✅ Got expected HTTP matching /$regex/ for: curl $*"
    cat "$tmpfile" | pretty_json || cat "$tmpfile"
    rm -f "$tmpfile"
  fi
}

echo "Test: Signup (Valid)"
expect_status 200 -- -X POST "http://127.0.0.1:$PORT/api/users/signup" \
  -H "Content-Type: application/json" \
  -d '{"email":"test@test.com","password":"pass"}'

echo "Test: Update (Valid)"
expect_status 200 -- -X PUT "http://127.0.0.1:$PORT/api/users/update" \
  -H "Content-Type: application/json" \
  -d '{"name":"User","age":25}'

echo "Test: Primitives (valid)"
expect_status 200 -- -X POST "http://127.0.0.1:$PORT/api/test/primitives" \
  -H "Content-Type: application/json" \
  -d '{"s":"hi","i":123,"f":1.5,"b":true}'


echo "Test: Arrays (valid)"
expect_status 200 -- -X POST "http://127.0.0.1:$PORT/api/test/arrays" \
  -H "Content-Type: application/json" \
  -d '{"tags":["a","b"],"nums":[1,2,3]}'

echo "Test: Arrays (element type mismatch -> expect 400)"
expect_status 400 -- -X POST "http://127.0.0.1:$PORT/api/test/arrays" \
  -H "Content-Type: application/json" \
  -d '{"tags":["a",123],"nums":[1,2]}'

echo "Test: Enum (valid)"
expect_status 200 -- -X POST "http://127.0.0.1:$PORT/api/test/enum" \
  -H "Content-Type: application/json" \
  -d '{"role":"User","id":10}'

echo "Test: Enum (invalid value -> expect 400)"
expect_status 400 -- -X POST "http://127.0.0.1:$PORT/api/test/enum" \
  -H "Content-Type: application/json" \
  -d '{"role":"Super","id":10}'

echo "Test: Map (valid)"
expect_status 200 -- -X POST "http://127.0.0.1:$PORT/api/test/map" \
  -H "Content-Type: application/json" \
  -d '{"meta":{"k":"v"},"flags":{"f1":true}}'

echo "Test: PUT Nested (valid) - cover non-POST handler"
expect_status 200 -- -X PUT "http://127.0.0.1:$PORT/api/test/nested" \
  -H "Content-Type: application/json" \
  -d '{"inner":{"x":7,"y":"put"},"list":[]}'

echo ""
echo "All request-parsing tests completed successfully."
