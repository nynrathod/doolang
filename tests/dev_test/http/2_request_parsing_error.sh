#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3102
FILE="2_request_parsing_error.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo ""
echo "Test 1: Signup (Valid)"
RESPONSE=$(http_post "/api/users/signup" '{"email":"test@test.com","password":"pass"}')
assert_status "$RESPONSE" 200 "POST /api/users/signup"

echo ""
echo "Test 2: Update (Valid)"
RESPONSE=$(http_put "/api/users/update" '{"name":"User","age":25}')
assert_status "$RESPONSE" 200 "PUT /api/users/update"

echo ""
echo "Test 3: Primitives (Valid)"
RESPONSE=$(http_post "/api/test/primitives" '{"s":"hi","i":123,"f":1.5,"b":true}')
assert_status "$RESPONSE" 200 "POST /api/test/primitives"

echo ""
echo "Test 4: Arrays (Valid)"
RESPONSE=$(http_post "/api/test/arrays" '{"tags":["a","b"],"nums":[1,2,3]}')
assert_status "$RESPONSE" 200 "POST /api/test/arrays"

echo ""
echo "Test 5: Arrays (element type mismatch -> 400)"
RESPONSE=$(http_post "/api/test/arrays" '{"tags":["a",123],"nums":[1,2]}')
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

echo ""
echo "Test 6: Enum (Valid)"
RESPONSE=$(http_post "/api/test/enum" '{"role":"User","id":10}')
assert_status "$RESPONSE" 200 "POST /api/test/enum"

echo ""
echo "Test 7: Enum (invalid value -> 400)"
RESPONSE=$(http_post "/api/test/enum" '{"role":"Super","id":10}')
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

echo ""
echo "Test 8: Map (Valid)"
RESPONSE=$(http_post "/api/test/map" '{"meta":{"k":"v"},"flags":{"f1":true}}')
assert_status "$RESPONSE" 200 "POST /api/test/map"

echo ""
echo "Test 9: PUT Nested (Valid)"
RESPONSE=$(http_put "/api/test/nested" '{"inner":{"x":7,"y":"put"},"list":[]}')
assert_status "$RESPONSE" 200 "PUT /api/test/nested"

print_http_summary
