#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3109
FILE="12_middlware_chaining.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

# --- PUBLIC ---
echo ""
echo "Test 1: Public route /status (200)"
RESPONSE=$(http_get "/status")
assert_status "$RESPONSE" 200 "GET /status"
assert_json "$RESPONSE" ".Message" "API is running" ".Message=API is running"
assert_json "$RESPONSE" ".Value" "100" ".Value=100"

# --- PROTECTED: no token → 401 ---
echo ""
echo "Test 2: Protected /protected without token (401)"
RESPONSE=$(http_get "/protected")
assert_rfc7807 "$RESPONSE" 401 "Unauthorized" "unauthorized"

# --- PROTECTED: invalid token → 401 ---
echo ""
echo "Test 3: Protected /protected invalid token (401)"
RESPONSE=$(http_get "/protected" "Authorization: Bearer wrong-token")
assert_rfc7807 "$RESPONSE" 401 "Unauthorized" "unauthorized"

# --- PROTECTED: valid token → 200 ---
echo ""
echo "Test 4: Protected /protected valid token (200)"
RESPONSE=$(http_get "/protected" "Authorization: Bearer valid-token")
assert_status "$RESPONSE" 200 "GET /protected valid"
assert_json "$RESPONSE" ".Message" "Protected resource" ".Message=Protected resource"
assert_json "$RESPONSE" ".Value" "200" ".Value=200"

print_http_summary
