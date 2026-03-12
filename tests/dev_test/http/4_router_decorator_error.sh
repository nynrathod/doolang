#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3104
FILE="4_router_decorator_error.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo ""
echo "Test 1: Valid User (200)"
RESPONSE=$(http_post "/api/users/create" '{"Email":"ok@t.com","Age":20,"Score":50,"Username":"okt","Rank":2,"Role":"user","Name":"Ok"}')
assert_status "$RESPONSE" 200 "valid user create"

echo ""
echo "Test 2: Invalid Email (422)"
RESPONSE=$(http_post "/api/users/create" '{"Email":"bad","Age":20,"Score":50,"Username":"baduser","Rank":3,"Role":"user","Name":"Bad"}')
assert_rfc7807 "$RESPONSE" 422 "Unprocessable Entity" "validation_error"

echo ""
echo "Test 3: Invalid Role enum (422)"
RESPONSE=$(http_post "/api/users/create" '{"Email":"john@example.com","Age":20,"Score":50,"Username":"johnny","Rank":2,"Role":"guest","Name":"John"}')
assert_rfc7807 "$RESPONSE" 422 "Unprocessable Entity" "validation_error"

# --- Auth Flow (dynamic tokens) ---
echo ""
echo "Test 4: Signup (get token)"
RESPONSE=$(http_post "/signup" '{"email":"test@example.com","password":"password123"}')
assert_status "$RESPONSE" 200 "signup"
assert_json_exists "$RESPONSE" ".data.token" "signup returns token"
assert_json_type "$RESPONSE" ".data.token" "string" "token is string"
TOKEN=$(extract_json "$RESPONSE" ".data.token")
if [ -z "$TOKEN" ] || [ "$TOKEN" = "null" ]; then
    echo "FATAL: Failed to extract token from signup"
    exit 1
fi

echo ""
echo "Test 5: Login (get token)"
RESPONSE=$(http_post "/login" '{"email":"test@example.com","password":"password123"}')
assert_status "$RESPONSE" 200 "login"
assert_json_exists "$RESPONSE" ".data.token" "login returns token"
TOKEN=$(extract_json "$RESPONSE" ".data.token")

echo ""
echo "Test 6: Invalid Task Status Enum (422)"
RESPONSE=$(http_post "/tasks" '{"title":"Fix Bug","status":"Archived","priority":"High"}' "Authorization: Bearer $TOKEN")
assert_rfc7807 "$RESPONSE" 422 "Unprocessable Entity" "validation_error"

echo ""
echo "Test 7: Invalid Task Priority Enum (422)"
RESPONSE=$(http_post "/tasks" '{"title":"Fix Bug","status":"Todo","priority":"Vital"}' "Authorization: Bearer $TOKEN")
assert_rfc7807 "$RESPONSE" 422 "Unprocessable Entity" "validation_error"

echo ""
echo "Test 8: Multiple Invalid Enums (422)"
RESPONSE=$(http_post "/tasks" '{"title":"Fix Bug","status":"Archived","priority":"Vital"}' "Authorization: Bearer $TOKEN")
assert_rfc7807 "$RESPONSE" 422 "Unprocessable Entity" "validation_error"

echo ""
echo "Test 9: Valid Enums (200)"
RESPONSE=$(http_post "/tasks" '{"title":"Fix Bug","status":"Todo","priority":"Medium"}' "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "valid task create"

print_http_summary
