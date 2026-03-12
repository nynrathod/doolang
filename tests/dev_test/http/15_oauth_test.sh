#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3120
FILE="15_oauth_test.doo"

# ---- Set OAuth env vars (test/dummy values) ----
export OAUTH_GOOGLE_CLIENT_ID="test-google-client-id.apps.googleusercontent.com"
export OAUTH_GOOGLE_CLIENT_SECRET="test-google-client-secret"
export OAUTH_GOOGLE_REDIRECT_URI="http://localhost:${PORT}/auth/google/callback"

export OAUTH_GITHUB_CLIENT_ID="test-github-client-id"
export OAUTH_GITHUB_CLIENT_SECRET="test-github-client-secret"
export OAUTH_GITHUB_REDIRECT_URI="http://localhost:${PORT}/auth/github/callback"

export JWT_SECRET="${JWT_SECRET:-test-oauth-jwt-secret}"

echo "=== OAuth Test Suite ==="
echo "Port: $PORT"
echo ""

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

# =====================================================================
# Test 1: Public route works (200)
# =====================================================================
echo ""
echo "Test 1: Public route GET /public (200)"
RESPONSE=$(http_get "/public")
assert_status "$RESPONSE" 200 "GET /public"

# =====================================================================
# Test 2: Public route /status works (200)
# =====================================================================
echo ""
echo "Test 2: Public route GET /status (200)"
RESPONSE=$(http_get "/status")
assert_status "$RESPONSE" 200 "GET /status"

# =====================================================================
# Test 3: JWT protected route without token (401)
# =====================================================================
echo ""
echo "Test 3: Protected GET /profile without JWT (401)"
RESPONSE=$(http_get "/profile")
assert_status "$RESPONSE" 401 "GET /profile no JWT"

# =====================================================================
# Test 4: Google OAuth redirect (302 → accounts.google.com)
# =====================================================================
echo ""
echo "Test 4: GET /auth/google → 302 redirect"
REDIRECT_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:$PORT/auth/google")
HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))
if [ "$REDIRECT_STATUS" = "302" ]; then
    HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
    echo -e "  ${_AGREEN}PASS${_ANC} status=302 GET /auth/google"
else
    HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
    echo -e "  ${_ARED}FAIL${_ANC} status: expected=302 actual=$REDIRECT_STATUS GET /auth/google"
fi

# Verify Location header points to Google
LOCATION=$(curl -s -D - -o /dev/null "http://127.0.0.1:$PORT/auth/google" | grep -i "^location:" | tr -d '\r')
HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))
if echo "$LOCATION" | grep -q "accounts.google.com"; then
    HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
    echo -e "  ${_AGREEN}PASS${_ANC} Location → accounts.google.com"
else
    HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
    echo -e "  ${_ARED}FAIL${_ANC} Location header missing or wrong: $LOCATION"
fi

# =====================================================================
# Test 5: GitHub OAuth redirect (302 → github.com)
# =====================================================================
echo ""
echo "Test 5: GET /auth/github → 302 redirect"
REDIRECT_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:$PORT/auth/github")
HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))
if [ "$REDIRECT_STATUS" = "302" ]; then
    HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
    echo -e "  ${_AGREEN}PASS${_ANC} status=302 GET /auth/github"
else
    HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
    echo -e "  ${_ARED}FAIL${_ANC} status: expected=302 actual=$REDIRECT_STATUS GET /auth/github"
fi

# Verify Location header points to GitHub
LOCATION=$(curl -s -D - -o /dev/null "http://127.0.0.1:$PORT/auth/github" | grep -i "^location:" | tr -d '\r')
HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))
if echo "$LOCATION" | grep -q "github.com"; then
    HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
    echo -e "  ${_AGREEN}PASS${_ANC} Location → github.com"
else
    HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
    echo -e "  ${_ARED}FAIL${_ANC} Location header missing or wrong: $LOCATION"
fi

# =====================================================================
# Test 6: Google callback without params (400 — missing code/state)
# =====================================================================
echo ""
echo "Test 6: GET /auth/google/callback without params (400)"
RESPONSE=$(http_get "/auth/google/callback")
assert_status "$RESPONSE" 400 "callback no params"

# =====================================================================
# Test 7: Google callback with error param (401 — access denied)
# =====================================================================
echo ""
echo "Test 7: GET /auth/google/callback?error=access_denied (401)"
RESPONSE=$(http_get "/auth/google/callback?error=access_denied&error_description=User+denied+access")
assert_status "$RESPONSE" 401 "callback error=access_denied"

# =====================================================================
# Test 8: Google callback with fake code+state (403 — invalid state)
# =====================================================================
echo ""
echo "Test 8: GET /auth/google/callback?code=fake&state=fake (403 invalid state)"
RESPONSE=$(http_get "/auth/google/callback?code=fakecode123&state=fakestate456")
# Should be 403 (invalid CSRF state) or 500 (exchange failure)
ACTUAL_STATUS=$(_get_status "$RESPONSE")
HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))
if [ "$ACTUAL_STATUS" = "403" ] || [ "$ACTUAL_STATUS" = "500" ]; then
    HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
    echo -e "  ${_AGREEN}PASS${_ANC} status=$ACTUAL_STATUS (expected 403 or 500) callback fake code+state"
else
    HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
    echo -e "  ${_ARED}FAIL${_ANC} status: expected=403|500 actual=$ACTUAL_STATUS callback fake code+state"
fi

# =====================================================================
# Test 9: GitHub callback without params (400)
# =====================================================================
echo ""
echo "Test 9: GET /auth/github/callback without params (400)"
RESPONSE=$(http_get "/auth/github/callback")
assert_status "$RESPONSE" 400 "github callback no params"

# =====================================================================
# Test 10: Unknown provider route (404)
# =====================================================================
echo ""
echo "Test 10: GET /auth/linkedin (404 — not registered)"
RESPONSE=$(http_get "/auth/linkedin")
assert_status "$RESPONSE" 404 "unknown provider"

# =====================================================================
# Test 11: Redirect includes PKCE + state in URL
# =====================================================================
echo ""
echo "Test 11: Google redirect URL includes PKCE code_challenge and state"
LOCATION_FULL=$(curl -s -D - -o /dev/null "http://127.0.0.1:$PORT/auth/google" | grep -i "^location:" | tr -d '\r')
HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))
if echo "$LOCATION_FULL" | grep -q "code_challenge"; then
    HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
    echo -e "  ${_AGREEN}PASS${_ANC} PKCE code_challenge in redirect URL"
else
    HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
    echo -e "  ${_ARED}FAIL${_ANC} Missing code_challenge in redirect URL"
fi

HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))
if echo "$LOCATION_FULL" | grep -q "state="; then
    HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
    echo -e "  ${_AGREEN}PASS${_ANC} CSRF state parameter in redirect URL"
else
    HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
    echo -e "  ${_ARED}FAIL${_ANC} Missing state parameter in redirect URL"
fi

print_http_summary
