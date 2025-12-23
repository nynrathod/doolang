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
SIGNUP_RESP=$(curl -s -X POST http://127.0.0.1:$PORT/signup \
  -H "Content-Type: application/json" \
  -d "{\"Email\":\"$EMAIL\",\"Password\":\"testpass123\",\"Name\":\"Jwt\",\"Role\":\"user\"}")

echo "$SIGNUP_RESP" | pretty_json
echo ""

# Extract token from signup response (more reliable)
TOKEN=$(echo "$SIGNUP_RESP" | grep -o '"token":"[^"]*"' | head -1 | sed 's/"token":"//;s/"$//')

echo "Login"
LOGIN_RESP=$(curl -s -X POST http://127.0.0.1:$PORT/login \
  -H "Content-Type: application/json" \
  -d "{\"Email\":\"$EMAIL\",\"Password\":\"testpass123\"}")

echo "$LOGIN_RESP" | pretty_json

# If signup token extraction failed, try from login
if [ -z "$TOKEN" ]; then
  TOKEN=$(echo "$LOGIN_RESP" | grep -o '"token":"[^"]*"' | head -1 | sed 's/"token":"//;s/"$//')
fi
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
