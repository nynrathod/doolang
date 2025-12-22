#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3112
FILE="11_middleware_groups_auth.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

# --------------------------------------------------
# PUBLIC ROUTES
# --------------------------------------------------

echo "Test 1: GET /status (public)"
curl -s http://127.0.0.1:$PORT/status | pretty_json
echo ""

# --------------------------------------------------
# API GROUP — AUTH REQUIRED
# --------------------------------------------------

echo "Test 3: GET /api/profile (no auth → 401)"
curl -s http://127.0.0.1:$PORT/api/profile | pretty_json
echo ""

echo "Test 4: GET /api/profile (invalid token → 401)"
curl -s -H "Authorization: Bearer wrong-token" \
  http://127.0.0.1:$PORT/api/profile | pretty_json
echo ""

echo "Test 5: GET /api/profile (valid token → 200)"
curl -s -H "Authorization: Bearer valid-token" \
  http://127.0.0.1:$PORT/api/profile | pretty_json
echo ""

echo "Test 6: POST /api/users (valid token → 200)"
curl -s -X POST \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer valid-token" \
  http://127.0.0.1:$PORT/api/users | pretty_json
echo ""

echo "Test 7: GET /api/posts (valid token → 200)"
curl -s -H "Authorization: Bearer valid-token" \
  http://127.0.0.1:$PORT/api/posts | pretty_json
echo ""

# --------------------------------------------------
# ADMIN GROUP — AUTH + ROLE REQUIRED
# --------------------------------------------------

echo "Test 8: GET /admin/dashboard (no auth → 401)"
curl -s http://127.0.0.1:$PORT/admin/dashboard | pretty_json
echo ""

echo "Test 9: GET /admin/dashboard (auth, no role → 403)"
curl -s -H "Authorization: Bearer valid-token" \
  http://127.0.0.1:$PORT/admin/dashboard | pretty_json
echo ""

echo "Test 10: GET /admin/dashboard (auth + admin role → 200)"
curl -s \
  -H "Authorization: Bearer valid-token" \
  -H "X-Role: admin" \
  http://127.0.0.1:$PORT/admin/dashboard | pretty_json
echo ""

echo "Test 11: GET /admin/users (auth + admin role → 200)"
curl -s \
  -H "Authorization: Bearer valid-token" \
  -H "X-Role: admin" \
  http://127.0.0.1:$PORT/admin/users | pretty_json
echo ""

echo "Test 12: DELETE /admin/users/789 (auth + admin role → 200)"
curl -s -X DELETE \
  -H "Authorization: Bearer valid-token" \
  -H "X-Role: admin" \
  http://127.0.0.1:$PORT/admin/users/789 | pretty_json
echo ""

echo "✅ All middleware + auth + group tests completed"
