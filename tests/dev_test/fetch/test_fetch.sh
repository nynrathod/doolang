#!/bin/bash
set -e

# =============================================================================
# Fetch API Tests — outbound HTTP requests from inside Doo route handlers
# Tests runtime safety (no nested-runtime panics) and full HTTP client features
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3190
FILE="main.doo"

echo "=== Fetch API Tests ==="
echo ""

start_server "$FILE" "$PORT" || exit 1
setup_trap

# ─────────────────────────────────────────────────────────────────────────────
# Test 1: Basic GET fetch (self-referential — handler fetches local /data)
# ─────────────────────────────────────────────────────────────────────────────
echo "Test 1: fetch GET to local endpoint"
RESPONSE=$(http_get "/test-fetch-get")
assert_status "$RESPONSE" 200 "GET /test-fetch-get"
# The response is the fetch result JSON: { status, body, ok, headers }
assert_json "$RESPONSE" ".status" "200" "fetch response status is 200"
assert_json "$RESPONSE" ".ok" "true" "fetch response ok is true"
# The body field contains the /data endpoint's response
assert_json "$RESPONSE" ".body" '{"name":"Doo","version":"1.0","lang":"compiled"}' "fetch body matches data"

# ─────────────────────────────────────────────────────────────────────────────
# Test 2: POST fetch with body
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "Test 2: fetch POST with JSON body"
RESPONSE=$(http_get "/test-fetch-post")
assert_status "$RESPONSE" 200 "GET /test-fetch-post"
assert_json "$RESPONSE" ".status" "200" "fetch POST status is 200"
assert_json "$RESPONSE" ".ok" "true" "fetch POST ok is true"

# ─────────────────────────────────────────────────────────────────────────────
# Test 3: fetch with custom headers
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "Test 3: fetch with custom headers"
RESPONSE=$(http_get "/test-fetch-headers")
assert_status "$RESPONSE" 200 "GET /test-fetch-headers"
assert_json "$RESPONSE" ".status" "200" "fetch headers status is 200"

# ─────────────────────────────────────────────────────────────────────────────
# Test 4: fetch with timeout (should succeed quickly on localhost)
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "Test 4: fetch with timeout parameter"
RESPONSE=$(http_get "/test-fetch-timeout")
assert_status "$RESPONSE" 200 "GET /test-fetch-timeout"
assert_json "$RESPONSE" ".status" "200" "fetch timeout status is 200"
assert_json "$RESPONSE" ".ok" "true" "fetch timeout ok is true"

# ─────────────────────────────────────────────────────────────────────────────
# Test 5: fetch to non-existent host (connection error → 500 error response)
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "Test 5: fetch to unreachable host"
RESPONSE=$(http_get "/test-fetch-error")
# Fetch returns a JSON string with ok:false when the request fails
assert_status "$RESPONSE" 200 "GET /test-fetch-error"
assert_json "$RESPONSE" ".ok" "false" "fetch error ok is false"

# ─────────────────────────────────────────────────────────────────────────────
# Test 6: fetch PUT method
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "Test 6: fetch PUT method"
RESPONSE=$(http_get "/test-fetch-put")
assert_status "$RESPONSE" 200 "GET /test-fetch-put"
assert_json "$RESPONSE" ".status" "200" "fetch PUT status is 200"

# ─────────────────────────────────────────────────────────────────────────────
# Test 7: fetch DELETE method
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "Test 7: fetch DELETE method"
RESPONSE=$(http_get "/test-fetch-delete")
assert_status "$RESPONSE" 200 "GET /test-fetch-delete"
assert_json "$RESPONSE" ".status" "405" "fetch DELETE status is 405"

# ─────────────────────────────────────────────────────────────────────────────
# Test 8: fetch with empty options (defaults to GET)
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "Test 8: fetch with empty options (default GET)"
RESPONSE=$(http_get "/test-fetch-defaults")
assert_status "$RESPONSE" 200 "GET /test-fetch-defaults"
assert_json "$RESPONSE" ".status" "200" "fetch default GET status is 200"
assert_json "$RESPONSE" ".ok" "true" "fetch default ok is true"
assert_json "$RESPONSE" ".body" '{"name":"Doo","version":"1.0","lang":"compiled"}' "fetch default body matches data"

# ─────────────────────────────────────────────────────────────────────────────
print_http_summary
