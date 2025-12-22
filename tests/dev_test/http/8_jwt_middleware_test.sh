#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"
EMAIL="u$(date +%s)@t.com"

PORT=3108
FILE="8_jwt_middleware_test.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

# --------------------------------------------------
# Public route (NO JWT)
# --------------------------------------------------

echo "Public route (expect 200)"
curl -s http://127.0.0.1:$PORT/public | pretty_json
echo ""

# --------------------------------------------------
# Protected route WITHOUT JWT
# --------------------------------------------------

echo "Protected GET /profile without JWT (expect 401)"
curl -s http://127.0.0.1:$PORT/profile | pretty_json
echo ""

# --------------------------------------------------
# Signup + Login
# --------------------------------------------------

echo "Signup"
curl -s -X POST http://127.0.0.1:$PORT/signup \
  -H "Content-Type: application/json" \
  -d "{\"Email\":\"$EMAIL\",\"Password\":\"p\",\"Name\":\"Jwt\",\"Role\":\"user\"}" \
  | pretty_json
echo ""

echo "Login"
RESP=$(curl -s -X POST http://127.0.0.1:$PORT/login \
  -H "Content-Type: application/json" \
  -d '{"Email":"jwt@t.com","Password":"p"}')

echo "$RESP" | pretty_json
TOKEN=$(echo "$RESP" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')
echo ""

# --------------------------------------------------
# Protected routes WITH JWT
# --------------------------------------------------

echo "Protected GET /profile with JWT (expect 200)"
curl -s http://127.0.0.1:$PORT/profile \
  -H "Authorization: Bearer $TOKEN" \
  | pretty_json
echo ""

echo "Protected POST /profile with JWT (expect 200)"
curl -s -X POST http://127.0.0.1:$PORT/profile \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  | pretty_json
echo ""

# --------------------------------------------------
# API group middleware
# --------------------------------------------------

echo "API group without JWT (expect 401)"
curl -s http://127.0.0.1:$PORT/api/profile | pretty_json
echo ""

echo "API group GET with JWT"
curl -s http://127.0.0.1:$PORT/api/profile \
  -H "Authorization: Bearer $TOKEN" \
  | pretty_json
echo ""

echo "API group POST with JWT"
curl -s -X POST http://127.0.0.1:$PORT/api/create \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  | pretty_json
echo ""

echo "API group LIST with JWT"
curl -s http://127.0.0.1:$PORT/api/list \
  -H "Authorization: Bearer $TOKEN" \
  | pretty_json
echo ""

echo "✅ JWT middleware tests completed"
