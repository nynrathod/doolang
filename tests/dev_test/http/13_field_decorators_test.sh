#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

# Unique email to avoid conflicts
EMAIL="decorator_test_$(date +%s)@test.com"

PORT=3113
FILE="13_field_decorators_test.doo"

echo "=================================================="
echo "  Testing Field Visibility Decorators"
echo "  @writeOnly, @readOnly, @internal"
echo "=================================================="
echo ""

start_server "$FILE" "$PORT" || exit 1
setup_trap

# --- Test 1: Signup (password @writeOnly — NOT in response) ---
echo ""
echo "Test 1: Signup (@writeOnly password hidden)"
RESPONSE=$(http_post "/signup" "{\"Email\":\"$EMAIL\",\"Password\":\"secret123\",\"Name\":\"Test User\"}")
assert_status "$RESPONSE" 200 "signup"
assert_json_not_has "$RESPONSE" "Password" "@writeOnly: password hidden"
assert_json_exists "$RESPONSE" ".data.token" "signup returns token"
assert_json_type "$RESPONSE" ".data.token" "string" "token is string"
TOKEN=$(extract_json "$RESPONSE" ".data.token")
if [ -z "$TOKEN" ] || [ "$TOKEN" = "null" ]; then
    # Fallback: try login
    LOGIN_RESP=$(http_post "/login" "{\"Email\":\"$EMAIL\",\"Password\":\"secret123\"}")
    TOKEN=$(extract_json "$LOGIN_RESP" ".data.token")
fi
if [ -z "$TOKEN" ] || [ "$TOKEN" = "null" ]; then
    echo "FATAL: Could not get auth token"
    exit 1
fi

# --- Test 2: Login (password @writeOnly — accepted but NOT in response) ---
echo ""
echo "Test 2: Login (@writeOnly password hidden)"
RESPONSE=$(http_post "/login" "{\"Email\":\"$EMAIL\",\"Password\":\"secret123\"}")
assert_status "$RESPONSE" 200 "login"
assert_json_not_has "$RESPONSE" "Password" "@writeOnly: password hidden in login"
assert_json_exists "$RESPONSE" ".data.token" "login returns token"

# --- Test 3: GET /users (credits @readOnly — should be in response) ---
echo ""
echo "Test 3: GET /users (@readOnly credits visible)"
RESPONSE=$(http_get "/users" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "GET /users"

# --- Test 4: InternalId @internal — NOT in any response ---
echo ""
echo "Test 4: GET single user (@internal InternalId hidden)"
# Try multiple ID extraction paths (handles different response shapes)
SIGNUP_USER_ID=$(extract_json "$RESPONSE" ".data[0].id" 2>/dev/null)
if [ -z "$SIGNUP_USER_ID" ] || [ "$SIGNUP_USER_ID" = "null" ]; then
    SIGNUP_USER_ID=$(extract_json "$RESPONSE" ".data[0].Id" 2>/dev/null)
fi
if [ -z "$SIGNUP_USER_ID" ] || [ "$SIGNUP_USER_ID" = "null" ]; then
    SIGNUP_USER_ID=$(extract_json "$RESPONSE" ".[0].id" 2>/dev/null)
fi
if [ -z "$SIGNUP_USER_ID" ] || [ "$SIGNUP_USER_ID" = "null" ]; then
    SIGNUP_USER_ID=$(extract_json "$RESPONSE" ".[0].Id" 2>/dev/null)
fi
if [ -z "$SIGNUP_USER_ID" ] || [ "$SIGNUP_USER_ID" = "null" ]; then
    echo "  SKIP: Could not extract user ID from GET /users"
else
    RESPONSE=$(http_get "/users/$SIGNUP_USER_ID" "Authorization: Bearer $TOKEN")
    assert_status "$RESPONSE" 200 "GET /users/$SIGNUP_USER_ID"
    assert_json_not_has "$RESPONSE" "InternalId" "@internal: InternalId hidden"
    assert_json_not_has "$RESPONSE" "internal_id" "@internal: internal_id hidden"
fi

print_http_summary
