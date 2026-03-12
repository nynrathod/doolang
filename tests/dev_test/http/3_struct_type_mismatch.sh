#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3103
FILE="3_struct_type_mismatch.doo"

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo ""
echo "Test 1: Valid Struct (200)"
RESPONSE=$(http_post "/api/users/signup" '{"email":"alice@test.com","age":30,"active":true,"score":99.5,"address":{"city":"Ahmedabad","zip":380015},"tagsStr":["dev","api"],"tagsInt":[1,2,3],"tagsBool":[true,false],"tagsFloat":[1.1,2.2]}')
assert_status "$RESPONSE" 200 "valid struct"
assert_body_contains "$RESPONSE" "alice@test.com" "response has email"

echo ""
echo "Test 2: Primitive Type Mismatch (400)"
RESPONSE=$(http_post "/api/users/signup" '{"email":"bob@test.com","age":"not-int","active":true,"score":10.5,"address":{"city":"A","zip":1},"tagsStr":[],"tagsInt":[],"tagsBool":[],"tagsFloat":[]}')
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

echo ""
echo "Test 3: Nested Struct Type Mismatch (400)"
RESPONSE=$(http_post "/api/users/signup" '{"email":"carol@test.com","age":22,"active":true,"score":12.3,"address":{"city":123,"zip":"bad"},"tagsStr":[],"tagsInt":[],"tagsBool":[],"tagsFloat":[]}')
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

echo ""
echo "Test 4: Array Element Type Mismatch (400)"
RESPONSE=$(http_post "/api/users/signup" '{"email":"dan@test.com","age":40,"active":false,"score":1.2,"address":{"city":"X","zip":9},"tagsStr":["ok",123],"tagsInt":[1,"bad"],"tagsBool":[true,"false"],"tagsFloat":[1.1,"2.2"]}')
assert_rfc7807 "$RESPONSE" 400 "Bad Request" "validation_error"

echo ""
echo "Test 5: Get struct return (200)"
RESPONSE=$(http_get "/api/users/info")
assert_status "$RESPONSE" 200 "GET /api/users/info"

print_http_summary
