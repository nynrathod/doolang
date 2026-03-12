#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3112
FILE="11_middleware_groups_auth.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

# --- PUBLIC ---
echo ""
echo "Test 1: GET /status (public -> 200)"
RESPONSE=$(http_get "/status")
assert_status "$RESPONSE" 200 "GET /status"
assert_json "$RESPONSE" ".Status" "ok" ".Status=ok"

# --- API GROUP: no auth → 401 ---
echo ""
echo "Test 2: GET /api/profile (no auth -> 401)"
RESPONSE=$(http_get "/api/profile")
assert_rfc7807 "$RESPONSE" 401 "Unauthorized" "unauthorized"

# --- API GROUP: invalid token → 401 ---
echo ""
echo "Test 3: GET /api/profile (invalid token -> 401)"
RESPONSE=$(http_get "/api/profile" "Authorization: Bearer wrong-token")
assert_rfc7807 "$RESPONSE" 401 "Unauthorized" "unauthorized"

# --- API GROUP: valid token → 200 ---
echo ""
echo "Test 4: GET /api/profile (valid token -> 200)"
RESPONSE=$(http_get "/api/profile" "Authorization: Bearer valid-token")
assert_status "$RESPONSE" 200 "GET /api/profile valid"
assert_json "$RESPONSE" ".Message" "User profile" ".Message=User profile"
assert_json "$RESPONSE" ".UserId" "123" ".UserId=123"

echo ""
echo "Test 5: POST /api/users (valid token -> 200)"
RESPONSE=$(http_post "/api/users" "" "Authorization: Bearer valid-token")
assert_status "$RESPONSE" 200 "POST /api/users valid"
assert_json "$RESPONSE" ".Message" "User created" ".Message=User created"
assert_json "$RESPONSE" ".UserId" "456" ".UserId=456"

echo ""
echo "Test 6: GET /api/posts (valid token -> 200)"
RESPONSE=$(http_get "/api/posts" "Authorization: Bearer valid-token")
assert_status "$RESPONSE" 200 "GET /api/posts valid"
assert_json "$RESPONSE" ".Message" "All posts" ".Message=All posts"

# --- ADMIN GROUP: no auth → 401 ---
echo ""
echo "Test 7: GET /admin/dashboard (no auth -> 401)"
RESPONSE=$(http_get "/admin/dashboard")
assert_rfc7807 "$RESPONSE" 401 "Unauthorized" "unauthorized"

# --- ADMIN GROUP: auth, no role → 403 ---
echo ""
echo "Test 8: GET /admin/dashboard (auth, no role -> 403)"
RESPONSE=$(http_get "/admin/dashboard" "Authorization: Bearer valid-token")
assert_rfc7807 "$RESPONSE" 403 "Forbidden" "forbidden"

# --- ADMIN GROUP: auth + admin role → 200 ---
echo ""
echo "Test 9: GET /admin/dashboard (auth + admin -> 200)"
RESPONSE=$(http_get "/admin/dashboard" "Authorization: Bearer valid-token" "X-Role: admin")
assert_status "$RESPONSE" 200 "GET /admin/dashboard valid"
assert_json "$RESPONSE" ".Message" "Admin dashboard" ".Message=Admin dashboard"

echo ""
echo "Test 10: GET /admin/users (auth + admin -> 200)"
RESPONSE=$(http_get "/admin/users" "Authorization: Bearer valid-token" "X-Role: admin")
assert_status "$RESPONSE" 200 "GET /admin/users valid"
assert_json "$RESPONSE" ".Message" "All users" ".Message=All users"

echo ""
echo "Test 11: DELETE /admin/users/789 (auth + admin -> 200)"
RESPONSE=$(http_delete "/admin/users/789" "Authorization: Bearer valid-token" "X-Role: admin")
assert_status "$RESPONSE" 200 "DELETE /admin/users/789 valid"
assert_json "$RESPONSE" ".Status" "deleted" ".Status=deleted"

print_http_summary
