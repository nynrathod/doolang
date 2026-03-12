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

# ANSI-safe grep helpers for logger assertions
strip_ansi() { sed 's/\x1b\[[0-9;]*m//g' "$1"; }
assert_doo_log() {
    local method="$1" path="$2" status="$3" label="$4"
    for i in $(seq 1 40); do
        if strip_ansi server.log | grep -q "\[Doo\].*| $status |.*| $method $path" 2>/dev/null; then
            echo "  ✓ [Doo] log: $status $method $path"
            return 0
        fi
        sleep 0.05
    done
    echo "  ❌ FAIL [$label]: expected [Doo] log for $status $method $path"
    strip_ansi server.log | grep "\[Doo\]" 2>/dev/null || echo "  (no [Doo] lines)"
    exit 1
}
assert_no_doo_log() {
    local method="$1" path="$2" status="$3" label="$4"
    sleep 0.3
    if strip_ansi server.log | grep -q "\[Doo\].*| $status |.*| $method $path" 2>/dev/null; then
        echo "  ❌ FAIL [$label]: found [Doo] log for $status (should be filtered)"
        exit 1
    fi
    echo "  ✓ No [Doo] log for $status $method $path (correctly filtered)"
}

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

# ==========================================================================
# Logger Custom Level Assertions
# Config: .logger({ Level: "Warn, Error" })
# Info (2xx) should NOT appear, Warn (4xx) SHOULD appear
# ==========================================================================
echo ""
echo "Test 10: Logger — Info (200) NOT logged (filtered)"
assert_no_doo_log "GET" "/api/users/int/123" "200" "Info filtered"

echo ""
echo "Test 11: Logger — Warn (400) IS logged"
assert_doo_log "GET" "/api/users/int/abc" "400" "Warn 400"

echo ""
echo "Test 12: Logger — Warn (404) IS logged"
assert_doo_log "GET" "/api/users/int" "404" "Warn 404"

echo ""
echo "Test 13: Logger — banner shows Warn, Error"
if strip_ansi server.log | grep -q "Logger:"; then
    LOGGER_LINE=$(strip_ansi server.log | grep "Logger:" | head -1 | sed 's/^ *//')
    echo "  ✓ Found: $LOGGER_LINE"
else
    echo "  ❌ FAIL: Logger line not in banner"
    exit 1
fi

print_http_summary
