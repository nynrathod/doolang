#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3111
FILE="10_middleware_route_auth.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

# --- PUBLIC ---
echo ""
echo "Test 1: Public route (no auth -> 200)"
RESPONSE=$(http_get "/public")
assert_status "$RESPONSE" 200 "GET /public"
assert_json "$RESPONSE" ".Message" "Public data" ".Message=Public data"
assert_json "$RESPONSE" ".UserId" "0" ".UserId=0"

# --- PROTECTED: no auth → 401 ---
echo ""
echo "Test 2: Protected route (no auth -> 401)"
RESPONSE=$(http_get "/profile")
assert_rfc7807 "$RESPONSE" 401 "Unauthorized" "unauthorized"

# --- PROTECTED: invalid token → 401 ---
echo ""
echo "Test 3: Protected route (invalid token -> 401)"
RESPONSE=$(http_get "/profile" "Authorization: Bearer wrong-token")
assert_rfc7807 "$RESPONSE" 401 "Unauthorized" "unauthorized"

# --- PROTECTED: valid token → 200 ---
echo ""
echo "Test 4: Protected route (valid token -> 200)"
RESPONSE=$(http_get "/profile" "Authorization: Bearer valid-token")
assert_status "$RESPONSE" 200 "GET /profile valid"
assert_json "$RESPONSE" ".Message" "User profile" ".Message=User profile"
assert_json "$RESPONSE" ".UserId" "123" ".UserId=123"

# --- ADMIN: no auth → 401 ---
echo ""
echo "Test 5: Admin route (no auth -> 401)"
RESPONSE=$(http_get "/admin")
assert_rfc7807 "$RESPONSE" 401 "Unauthorized" "unauthorized"

# --- ADMIN: invalid token → 401 ---
echo ""
echo "Test 6: Admin route (invalid token -> 401)"
RESPONSE=$(http_get "/admin" "Authorization: Bearer wrong-token")
assert_rfc7807 "$RESPONSE" 401 "Unauthorized" "unauthorized"

# --- ADMIN: valid token, no role → 403 ---
echo ""
echo "Test 7: Admin route (valid token, no role -> 403)"
RESPONSE=$(http_get "/admin" "Authorization: Bearer valid-token")
assert_rfc7807 "$RESPONSE" 403 "Forbidden" "forbidden"

# --- ADMIN: valid token + admin role → 200 ---
echo ""
echo "Test 8: Admin route (valid token + admin role -> 200)"
RESPONSE=$(http_get "/admin" "Authorization: Bearer valid-token" "X-Role: admin")
assert_status "$RESPONSE" 200 "GET /admin valid"
assert_json "$RESPONSE" ".Message" "Admin data" ".Message=Admin data"
assert_json "$RESPONSE" ".UserId" "1" ".UserId=1"

print_http_summary
