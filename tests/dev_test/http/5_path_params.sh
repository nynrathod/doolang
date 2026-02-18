#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3105
FILE="5_path_params.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo ""
echo "Test 1: Valid Int (200)"
RESPONSE=$(http_get "/api/users/int/123")
assert_status "$RESPONSE" 200 "GET /api/users/int/123"

echo ""
echo "Test 2: Invalid Int (400)"
RESPONSE=$(http_get "/api/users/int/abc")
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

echo ""
echo "Test 3: Valid Str (200)"
RESPONSE=$(http_get "/api/users/str/hello")
assert_status "$RESPONSE" 200 "GET /api/users/str/hello"

echo ""
echo "Test 4: Valid Str 'true' (200)"
RESPONSE=$(http_get "/api/users/str/true")
assert_status "$RESPONSE" 200 "GET /api/users/str/true"

echo ""
echo "Test 5: Valid Bool (200)"
RESPONSE=$(http_get "/api/users/bool/true")
assert_status "$RESPONSE" 200 "GET /api/users/bool/true"

echo ""
echo "Test 6: Invalid Bool (400)"
RESPONSE=$(http_get "/api/users/bool/yes")
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

echo ""
echo "Test 7: Valid Float (200)"
RESPONSE=$(http_get "/api/users/float/12.34")
assert_status "$RESPONSE" 200 "GET /api/users/float/12.34"

echo ""
echo "Test 8: Invalid Float (400)"
RESPONSE=$(http_get "/api/users/float/notnum")
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

echo ""
echo "Test 9: Missing ID (404)"
RESPONSE=$(http_get "/api/users/int")
assert_rfc7807 "$RESPONSE" 404 "Not Found" "not_found"

print_http_summary
