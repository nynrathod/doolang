#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3101
FILE="1_router_not_found.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

# =========================================================
# Test 1: Valid route — should return 200 with body
# =========================================================
echo ""
echo "Test 1: Valid route (GET /users)"
RESPONSE=$(http_get "/users")
assert_status "$RESPONSE" 200 "GET /users"
assert_body "$RESPONSE" "User found!" "GET /users body"

# =========================================================
# Test 2: 404 Not Found — GET non-existent route
# =========================================================
echo ""
echo "Test 2: 404 Not Found (GET /notfound)"
RESPONSE=$(http_get "/notfound")
assert_rfc7807 "$RESPONSE" 404 "Not Found" "not_found" \
    "The requested route does not exist" "/notfound" "GET"

# =========================================================
# Test 3: 404 Not Found — POST non-existent route
# =========================================================
echo ""
echo "Test 3: 404 Not Found (POST /invalid)"
RESPONSE=$(http_post "/invalid")
assert_rfc7807 "$RESPONSE" 404 "Not Found" "not_found" \
    "The requested route does not exist" "/invalid" "POST"

# =========================================================
# Summary
# =========================================================
print_http_summary
