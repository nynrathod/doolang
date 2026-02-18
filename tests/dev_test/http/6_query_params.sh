#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3106
FILE="6_query_params.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo ""
echo "Test 1: Valid Int (200)"
RESPONSE=$(http_get "/api/users/int?id=1")
assert_status "$RESPONSE" 200 "GET /api/users/int?id=1"

echo ""
echo "Test 2: Invalid Int (400)"
RESPONSE=$(http_get "/api/users/int?id=abc")
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

echo ""
echo "Test 3: Missing params (400)"
RESPONSE=$(http_get "/api/users/int")
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

echo ""
echo "Test 4: Valid Bool (200)"
RESPONSE=$(http_get "/api/users/bool?id=true")
assert_status "$RESPONSE" 200 "GET /api/users/bool?id=true"

echo ""
echo "Test 5: Invalid Bool (400)"
RESPONSE=$(http_get "/api/users/bool?id=yes")
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

echo ""
echo "Test 6: Valid Float (200)"
RESPONSE=$(http_get "/api/users/float?id=1.5")
assert_status "$RESPONSE" 200 "GET /api/users/float?id=1.5"

echo ""
echo "Test 7: Invalid Float (400)"
RESPONSE=$(http_get "/api/users/float?id=abc")
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

echo ""
echo "Test 8: Valid Str (200)"
RESPONSE=$(http_get "/api/users/str?id=alice")
assert_status "$RESPONSE" 200 "GET /api/users/str?id=alice"

echo ""
echo "Test 9: Invalid Str empty (400)"
RESPONSE=$(http_get "/api/users/str?id=")
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

print_http_summary
