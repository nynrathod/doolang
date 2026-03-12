#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3107
FILE="7_auto_json.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo ""
echo "Test 1: Query Params (200)"
RESPONSE=$(http_get "/search?q=test&page=1")
assert_status "$RESPONSE" 200 "GET /search?q=test&page=1"

echo ""
echo "Test 2: Path Params (200)"
RESPONSE=$(http_get "/users/42")
assert_status "$RESPONSE" 200 "GET /users/42"

echo ""
echo "Test 3: Body Params (200)"
RESPONSE=$(http_post "/users/42/profile" '{"name":"Alice","email":"alice@example.com"}')
assert_status "$RESPONSE" 200 "POST /users/42/profile"

print_http_summary
