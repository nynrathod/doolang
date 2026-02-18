#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3110
FILE="9_route_group.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

# --- Public Routes ---
echo ""
echo "Test 1: GET /status"
RESPONSE=$(http_get "/status")
assert_status "$RESPONSE" 200 "GET /status"
assert_json "$RESPONSE" ".Status" "ok" ".Status=ok"
assert_json "$RESPONSE" ".Version" "1.0.0" ".Version=1.0.0"

# --- API Group: Users ---
echo ""
echo "Test 2: GET /api/profile"
RESPONSE=$(http_get "/api/profile")
assert_status "$RESPONSE" 200 "GET /api/profile"
assert_json "$RESPONSE" ".Id" "1" ".Id=1"
assert_json "$RESPONSE" ".Name" "John Doe" ".Name=John Doe"

echo ""
echo "Test 3: POST /api/users"
RESPONSE=$(http_post "/api/users")
assert_status "$RESPONSE" 200 "POST /api/users"
assert_json "$RESPONSE" ".Id" "2" ".Id=2"
assert_json "$RESPONSE" ".Name" "New User" ".Name=New User"

echo ""
echo "Test 4: GET /api/users/123"
RESPONSE=$(http_get "/api/users/123")
assert_status "$RESPONSE" 200 "GET /api/users/123"
assert_json "$RESPONSE" ".Id" "123" ".Id=123"
assert_json "$RESPONSE" ".Name" "User 123" ".Name=User 123"

echo ""
echo "Test 5: DELETE /api/users/123"
RESPONSE=$(http_delete "/api/users/123")
assert_status "$RESPONSE" 200 "DELETE /api/users/123"
assert_json "$RESPONSE" ".Status" "deleted" ".Status=deleted"

# --- API Group: Posts ---
echo ""
echo "Test 6: GET /api/posts"
RESPONSE=$(http_get "/api/posts")
assert_status "$RESPONSE" 200 "GET /api/posts"
assert_json "$RESPONSE" ".Title" "All Posts" ".Title=All Posts"

echo ""
echo "Test 7: POST /api/posts"
RESPONSE=$(http_post "/api/posts")
assert_status "$RESPONSE" 200 "POST /api/posts"
assert_json "$RESPONSE" ".Id" "1" ".Id=1"
assert_json "$RESPONSE" ".Title" "New Post" ".Title=New Post"

echo ""
echo "Test 8: GET /api/posts/456"
RESPONSE=$(http_get "/api/posts/456")
assert_status "$RESPONSE" 200 "GET /api/posts/456"
assert_json "$RESPONSE" ".Id" "456" ".Id=456"
assert_json "$RESPONSE" ".Title" "Post 456" ".Title=Post 456"

echo ""
echo "Test 9: DELETE /api/posts/456"
RESPONSE=$(http_delete "/api/posts/456")
assert_status "$RESPONSE" 200 "DELETE /api/posts/456"
assert_json "$RESPONSE" ".Status" "deleted" ".Status=deleted"

# --- Admin Group ---
echo ""
echo "Test 10: GET /admin/dashboard"
RESPONSE=$(http_get "/admin/dashboard")
assert_status "$RESPONSE" 200 "GET /admin/dashboard"
assert_json "$RESPONSE" ".Status" "admin dashboard" ".Status=admin dashboard"

echo ""
echo "Test 11: GET /admin/users"
RESPONSE=$(http_get "/admin/users")
assert_status "$RESPONSE" 200 "GET /admin/users"
assert_json "$RESPONSE" ".Name" "All Admin Users" ".Name=All Admin Users"

print_http_summary
