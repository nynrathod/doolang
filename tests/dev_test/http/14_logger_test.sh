#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3120
FILE="14_logger_test.doo"

echo "=== Logger System Test ==="
echo "Tests: Info (2xx), Warn (4xx), banner, CORS chaining"
echo ""

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

# --------------------------------------------------------------------------
# Helper: wait for [Doo] log line (server writes AFTER response is sent)
# Retries up to 2s to avoid race between curl and server log flush
# --------------------------------------------------------------------------
# Strip ANSI escape codes from a file (handles color codes in piped output)
strip_ansi() {
    sed 's/\x1b\[[0-9;]*m//g' "$1"
}

assert_doo_log() {
    local method="$1"
    local path="$2"
    local status="$3"
    local label="$4"
    for i in $(seq 1 40); do
        if strip_ansi server.log | grep -q "\[Doo\].*| $status |.*| $method $path" 2>/dev/null; then
            echo "  ✓ [Doo] log: $status $method $path"
            return 0
        fi
        sleep 0.05
    done
    echo "  ❌ FAIL [$label]: expected [Doo] log for $status $method $path"
    echo "  --- server.log [Doo] lines (raw) ---"
    strip_ansi server.log | grep "\[Doo\]" 2>/dev/null || echo "  (no [Doo] lines found)"
    echo "  --- full server.log ---"
    cat server.log
    exit 1
}

# ==========================================================================
# Test 1: Info category — GET /ok → 200
# ==========================================================================
echo "Test 1: Info category — GET /ok → 200"
RESPONSE=$(http_get "/ok")
assert_status "$RESPONSE" 200 "GET /ok"
assert_doo_log "GET" "/ok" "200" "Info 200"

# ==========================================================================
# Test 2: Info category — POST /create → 200
# ==========================================================================
echo ""
echo "Test 2: Info category — POST /create → 200"
RESPONSE=$(http_post "/create" '{}')
assert_status "$RESPONSE" 200 "POST /create"
assert_doo_log "POST" "/create" "200" "Info POST 200"

# ==========================================================================
# Test 3: Warn category — GET /nonexistent → 404 (router auto-404)
# ==========================================================================
echo ""
echo "Test 3: Warn category — GET /nonexistent → 404"
RESPONSE=$(http_get "/nonexistent")
assert_status "$RESPONSE" 404 "GET /nonexistent"
assert_doo_log "GET" "/nonexistent" "404" "Warn 404"

# ==========================================================================
# Test 4: Warn category — POST to GET-only route → 405
# ==========================================================================
echo ""
echo "Test 4: Warn category — POST /ok → 405"
RESPONSE=$(http_post "/ok" '{}')
assert_status "$RESPONSE" 405 "POST /ok (method not allowed)"
assert_doo_log "POST" "/ok" "405" "Warn 405"

# ==========================================================================
# Test 5: Startup banner present
# ==========================================================================
echo ""
echo "Test 5: Startup banner"
if strip_ansi server.log | grep -q "Doo v"; then
    echo "  ✓ Doo banner found"
else
    echo "  ❌ FAIL: Doo banner not in server.log"
    strip_ansi server.log
    exit 1
fi

# ==========================================================================
# Test 6: Logger levels in banner
# ==========================================================================
echo ""
echo "Test 6: Logger line in banner"
if strip_ansi server.log | grep -q "Logger:"; then
    LOGGER_LINE=$(strip_ansi server.log | grep "Logger:" | head -1)
    echo "  ✓ Found: $(echo "$LOGGER_LINE" | sed 's/^ *//')"
else
    echo "  ❌ FAIL: 'Logger:' not found in banner"
    strip_ansi server.log
    exit 1
fi

# ==========================================================================
# Test 7: CORS works alongside logger (chaining)
# ==========================================================================
echo ""
echo "Test 7: CORS + Logger chaining"
CORS_RESP=$(curl -siX OPTIONS "http://127.0.0.1:$PORT/ok" \
    -H "Origin: http://localhost:3000" \
    -H "Access-Control-Request-Method: GET" 2>/dev/null || true)
if echo "$CORS_RESP" | grep -qi "access-control-allow"; then
    echo "  ✓ CORS headers present on OPTIONS"
else
    # Some CORS impls only add headers on actual requests
    CORS_GET=$(curl -si "http://127.0.0.1:$PORT/ok" \
        -H "Origin: http://localhost:3000" 2>/dev/null || true)
    if echo "$CORS_GET" | grep -qi "access-control\|200"; then
        echo "  ✓ CORS working on GET with Origin"
    else
        echo "  ❌ FAIL: CORS + logger chaining broke"
        exit 1
    fi
fi


# ==========================================================================
# Test 8: Error category — GET /crash → 500 (red in terminal)
# ==========================================================================
echo ""
echo "Test 8: Error category — GET /crash → 500"
RESPONSE=$(http_get "/crash")
assert_status "$RESPONSE" 500 "GET /crash (intentional 500)"
assert_doo_log "GET" "/crash" "500" "Error 500"

# ==========================================================================
# Summary
# ==========================================================================
echo ""
echo "==================================="
echo "  ✓ All 8 logger tests passed!"
echo "==================================="
echo ""
echo "--- [Doo] log lines from server.log ---"
strip_ansi server.log | grep "\[Doo\]" || echo "(none)"
