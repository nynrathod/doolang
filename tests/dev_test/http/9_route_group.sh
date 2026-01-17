#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3110
FILE="9_route_group.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

# --------------------------------------------------
# PUBLIC ROUTES
# --------------------------------------------------

echo "Test 1: GET /status"
curl -s http://127.0.0.1:$PORT/status | pretty_json
echo ""

# --------------------------------------------------
# API GROUP — USERS
# --------------------------------------------------

echo "Test 3: GET /api/profile"
curl -s http://127.0.0.1:$PORT/api/profile | pretty_json
echo ""

echo "Test 4: POST /api/users"
curl -s -X POST -H "Content-Type: application/json" http://127.0.0.1:$PORT/api/users | pretty_json
echo ""

echo "Test 5: GET /api/users/123"
curl -s http://127.0.0.1:$PORT/api/users/123 | pretty_json
echo ""

echo "Test 6: DELETE /api/users/123"
curl -s -X DELETE http://127.0.0.1:$PORT/api/users/123 | pretty_json
echo ""

# --------------------------------------------------
# API GROUP — POSTS
# --------------------------------------------------

echo "Test 7: GET /api/posts"
curl -s http://127.0.0.1:$PORT/api/posts | pretty_json
echo ""

echo "Test 8: POST /api/posts"
curl -s -X POST -H "Content-Type: application/json" http://127.0.0.1:$PORT/api/posts | pretty_json
echo ""

echo "Test 9: GET /api/posts/456"
curl -s http://127.0.0.1:$PORT/api/posts/456 | pretty_json
echo ""

echo "Test 10: DELETE /api/posts/456"
curl -s -X DELETE http://127.0.0.1:$PORT/api/posts/456 | pretty_json
echo ""

# --------------------------------------------------
# ADMIN GROUP
# --------------------------------------------------

echo "Test 11: GET /admin/dashboard"
curl -s http://127.0.0.1:$PORT/admin/dashboard | pretty_json
echo ""

echo "Test 12: GET /admin/users"
curl -s http://127.0.0.1:$PORT/admin/users | pretty_json
echo ""

echo "✅ All route group tests completed"

# --------------------------------------------------
# Log location (like blog/test_blog.sh)
# --------------------------------------------------
echo ""
echo "📝 Server logs saved in: $(cd "$SCRIPT_DIR" && pwd)/server9.log"
echo "   To inspect: tail -200 \"$SCRIPT_DIR/server9.log\""
if command -v wslpath >/dev/null 2>&1; then
  WIN_LOG_PATH="$(wslpath -w "$SCRIPT_DIR/server9.log")"
  echo "   Windows path: $WIN_LOG_PATH"
fi
