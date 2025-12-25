#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3104
FILE="4_router_decorator_error.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo "Test: Valid User (Manual Handler)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/create \
  -H "Content-Type: application/json" \
  -d '{"Email":"ok@t.com","Age":20,"Score":50,"Username":"okt","Rank":2,"Role":"user","Name":"Ok"}' \
  | pretty_json
echo ""

# ... (Existing validation tests omitted for brevity, keeping only relevant logic) ...
# Actually better to keep them to ensure no regression.

echo "Test: Invalid Email (Expect 422)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/create \
  -H "Content-Type: application/json" \
  -d '{"Email":"bad","Age":20,"Score":50,"Username":"baduser","Rank":3,"Role":"user","Name":"Bad"}' \
  | pretty_json
echo ""

echo "Test: Invalid Role enum (Expect 422)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/create \
  -H "Content-Type: application/json" \
  -d '{"Email":"john@example.com","Age":20,"Score":50,"Username":"johnny","Rank":2,"Role":"guest","Name":"John"}' \
  | pretty_json
echo ""

# Auth Flow
echo "Test: Signup (Get Token)"
SIGNUP_RES=$(curl -s -X POST http://127.0.0.1:$PORT/signup \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","password":"password123"}')
echo "$SIGNUP_RES" | pretty_json
echo ""

echo "Test: Login (Get Token)"
LOGIN_RES=$(curl -s -X POST http://127.0.0.1:$PORT/login \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","password":"password123"}')
echo "$LOGIN_RES" | pretty_json
echo ""

# Extract Token - simple grep hack since jq might not be present or we want to be dependency-lite
# Assuming format {"data":{"token":"..."}}
TOKEN=$(echo "$LOGIN_RES" | grep -o '"token":"[^"]*"' | cut -d'"' -f4)

if [ -z "$TOKEN" ]; then
    echo "❌ Failed to extract token"
    exit 1
fi
echo "Extracted Token: $TOKEN"
echo ""

echo "Test: Invalid Task Status Enum (Expect 422) with Token"
curl -s -X POST http://127.0.0.1:$PORT/tasks \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"title":"Fix Bug","status":"Archived","priority":"High"}' \
  | pretty_json
echo ""

echo "Test: Invalid Task Priority Enum (Expect 422) with Token"
curl -s -X POST http://127.0.0.1:$PORT/tasks \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"title":"Fix Bug","status":"Todo","priority":"Vital"}' \
  | pretty_json
echo ""

echo "Test: Multiple Invalid Enums (Expect 422) with Token"
curl -s -X POST http://127.0.0.1:$PORT/tasks \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"title":"Fix Bug","status":"Archived","priority":"Vital"}' \
  | pretty_json
echo ""

echo "Test: Valid Enums (Expect 200) with Token"
curl -s -X POST http://127.0.0.1:$PORT/tasks \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"title":"Fix Bug","status":"Todo","priority":"Medium"}' \
  | pretty_json
echo ""
