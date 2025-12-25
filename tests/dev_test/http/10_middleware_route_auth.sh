#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3111
FILE="10_middleware_route_auth.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

# --------------------------------------------------
# PUBLIC ROUTE
# --------------------------------------------------

echo "Test 1: Public route (no auth → 200)"
curl -s http://127.0.0.1:$PORT/public | pretty_json
echo ""

# --------------------------------------------------
# PROTECTED ROUTE (/profile)
# --------------------------------------------------

echo "Test 2: Protected route (no auth → 401)"
curl -s http://127.0.0.1:$PORT/profile | pretty_json
echo ""

echo "Test 3: Protected route (invalid token → 401)"
curl -s \
  -H "Authorization: Bearer wrong-token" \
  http://127.0.0.1:$PORT/profile | pretty_json
echo ""

echo "Test 4: Protected route (valid token → 200)"
curl -s \
  -H "Authorization: Bearer valid-token" \
  http://127.0.0.1:$PORT/profile | pretty_json
echo ""

# --------------------------------------------------
# ADMIN ROUTE (/admin)
# --------------------------------------------------

echo "Test 5: Admin route (no auth → 401)"
curl -s http://127.0.0.1:$PORT/admin | pretty_json
echo ""

echo "Test 6: Admin route (invalid token → 401)"
curl -s \
  -H "Authorization: Bearer wrong-token" \
  http://127.0.0.1:$PORT/admin | pretty_json
echo ""

echo "Test 7: Admin route (valid token, no role → 403)"
curl -s \
  -H "Authorization: Bearer valid-token" \
  http://127.0.0.1:$PORT/admin | pretty_json
echo ""

echo "Test 8: Admin route (valid token + admin role → 200)"
curl -s \
  -H "Authorization: Bearer valid-token" \
  -H "X-Role: admin" \
  http://127.0.0.1:$PORT/admin | pretty_json
echo ""

echo "✅ All middleware route auth tests completed"
