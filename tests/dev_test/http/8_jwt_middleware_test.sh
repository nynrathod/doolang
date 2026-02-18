#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

EMAIL="u$(date +%s)@t.com"
PORT=3108
FILE="8_jwt_middleware_test.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

# --- Public route (no auth needed) ---
echo ""
echo "Test 1: Public route (200)"
RESPONSE=$(http_get "/public")
assert_status "$RESPONSE" 200 "GET /public"

# --- Protected route WITHOUT JWT → 401 ---
echo ""
echo "Test 2: Protected no JWT (401)"
RESPONSE=$(http_get "/profile")
assert_rfc7807 "$RESPONSE" 401 "Unauthorized" "unauthorized"

# --- Signup (dynamic token) ---
echo ""
echo "Test 3: Signup"
RESPONSE=$(http_post "/signup" "{\"Email\":\"$EMAIL\",\"Password\":\"testpass123\",\"Name\":\"Jwt\",\"Role\":\"user\"}")
assert_status "$RESPONSE" 200 "signup"
assert_json_exists "$RESPONSE" ".data.token" "signup returns token"
assert_json_type "$RESPONSE" ".data.token" "string" "token is string"
assert_json_not_has "$RESPONSE" "Password" "password not exposed"
TOKEN=$(extract_json "$RESPONSE" ".data.token")

# --- Login (dynamic token) ---
echo ""
echo "Test 4: Login"
RESPONSE=$(http_post "/login" "{\"Email\":\"$EMAIL\",\"Password\":\"testpass123\"}")
assert_status "$RESPONSE" 200 "login"
assert_json_exists "$RESPONSE" ".data.token" "login returns token"
# Use login token if signup had issues
NEW_TOKEN=$(extract_json "$RESPONSE" ".data.token")
if [ -n "$NEW_TOKEN" ] && [ "$NEW_TOKEN" != "null" ]; then
    TOKEN="$NEW_TOKEN"
fi

# --- Protected route WITH JWT → 200 ---
echo ""
echo "Test 5: Protected GET /profile with JWT (200)"
RESPONSE=$(http_get "/profile" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "GET /profile with JWT"

echo ""
echo "Test 6: Protected POST /profile with JWT (200)"
RESPONSE=$(http_post "/profile" "" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "POST /profile with JWT"

# --- API group middleware ---
echo ""
echo "Test 7: API group without JWT (401)"
RESPONSE=$(http_get "/api/profile")
assert_rfc7807 "$RESPONSE" 401 "Unauthorized" "unauthorized"

echo ""
echo "Test 8: API group GET with JWT (200)"
RESPONSE=$(http_get "/api/profile" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "GET /api/profile with JWT"

echo ""
echo "Test 9: API group POST with JWT (200)"
RESPONSE=$(http_post "/api/create" "" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "POST /api/create with JWT"

echo ""
echo "Test 10: API group LIST with JWT (200)"
RESPONSE=$(http_get "/api/list" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "GET /api/list with JWT"

print_http_summary
