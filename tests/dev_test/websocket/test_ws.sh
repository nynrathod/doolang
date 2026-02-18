#!/bin/bash
# =============================================================================
# WebSocket Comprehensive Test Suite
# Tests: connect, emit/receive, echo, rooms, broadcast, lifecycle, multi-client
# =============================================================================
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3210
FILE="main.doo"

# =============================================================================
# Python WebSocket Client Helper
# We use Python + websockets lib for WS testing.
# Install if missing: pip install websockets
# =============================================================================
check_ws_deps() {
    # Need websockets >= 12.0 for Python 3.10+ compatibility (loop param removed)
    local need_install=0
    if ! python3 -c "import websockets" 2>/dev/null; then
        need_install=1
    else
        local ver
        ver=$(python3 -c "import websockets; print(websockets.__version__)" 2>/dev/null)
        local major
        major=$(echo "$ver" | cut -d. -f1)
        if [ "$major" -lt 12 ] 2>/dev/null; then
            need_install=1
        fi
    fi

    if [ "$need_install" -eq 1 ]; then
        echo "Installing/upgrading websockets Python package (need >= 12.0)..."
        python3 -m pip install "websockets>=12.0" --quiet --break-system-packages 2>/dev/null \
            || python3 -m pip install "websockets>=12.0" --quiet --user 2>/dev/null \
            || python3 -m pip install "websockets>=12.0" --quiet 2>/dev/null \
            || pip3 install "websockets>=12.0" --quiet 2>/dev/null \
            || {
            echo "FAIL: Cannot install 'websockets >= 12.0' Python package."
            echo "  Try: python3 -m pip install 'websockets>=12.0' --user"
            exit 1
        }
    fi
}

# =============================================================================
# Start server and check deps
# =============================================================================
echo "=== Doo WebSocket Test Suite ==="
echo ""

check_ws_deps

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

WS_URL="ws://127.0.0.1:$PORT"
HTTP_URL="http://127.0.0.1:$PORT"

# =============================================================================
# Test 1: HTTP health check (server is up)
# =============================================================================
echo ""
echo "--- Test 1: Server Health Check ---"
RESPONSE=$(http_get "/health")
assert_status "$RESPONSE" 200 "Health check"

# =============================================================================
# ALL WebSocket tests in ONE Python process via test_ws.py
# Output streams live (not buffered) — python3 -u for unbuffered.
# =============================================================================
SCRIPT_PY="$SCRIPT_DIR/test_ws.py"
WS_RESULT_FILE=$(mktemp)
python3 -u "$SCRIPT_PY" "$WS_URL" "$HTTP_URL" 2>&1 | tee "$WS_RESULT_FILE" || true

# Parse WS results from the output
WS_PASSED=$(grep -oP 'WS_RESULTS:\K[0-9]+' "$WS_RESULT_FILE" 2>/dev/null || echo 0)
WS_FAILED=$(sed -n 's/.*WS_RESULTS:[0-9]*:\([0-9]*\):.*/\1/p' "$WS_RESULT_FILE" 2>/dev/null || echo 0)
rm -f "$WS_RESULT_FILE"

# Lifecycle log check
echo ""
echo "--- Lifecycle Log Check ---"
LIFECYCLE_PASS=0
LIFECYCLE_FAIL=0

sleep 0.3
if [ -f server.log ]; then
    if grep -q "LIFECYCLE: connected" server.log; then
        echo "  ✅ PASS: onConnect fired"
        LIFECYCLE_PASS=$((LIFECYCLE_PASS + 1))
    else
        echo "  ❌ FAIL: onConnect fired (NOT_FOUND_IN_LOG)"
        LIFECYCLE_FAIL=$((LIFECYCLE_FAIL + 1))
    fi
    if grep -q "LIFECYCLE: disconnected" server.log; then
        echo "  ✅ PASS: onDisconnect fired"
        LIFECYCLE_PASS=$((LIFECYCLE_PASS + 1))
    else
        echo "  ❌ FAIL: onDisconnect fired (NOT_FOUND_IN_LOG)"
        LIFECYCLE_FAIL=$((LIFECYCLE_FAIL + 1))
    fi
else
    echo "  ❌ FAIL: onConnect fired (NO_SERVER_LOG)"
    echo "  ❌ FAIL: onDisconnect fired (NO_SERVER_LOG)"
    LIFECYCLE_FAIL=2
fi

# HTTP test count (health check)
HTTP_PASS_COUNT=$(get_http_pass_count 2>/dev/null || echo 1)
HTTP_FAIL_COUNT=$(get_http_fail_count 2>/dev/null || echo 0)

TOTAL_PASSED=$((WS_PASSED + LIFECYCLE_PASS + HTTP_PASS_COUNT))
TOTAL_FAILED=$((WS_FAILED + LIFECYCLE_FAIL + HTTP_FAIL_COUNT))
TOTAL=$((TOTAL_PASSED + TOTAL_FAILED))

echo ""
echo "==========================================="
echo "  WebSocket Test Results"
echo "==========================================="
echo "  Total:  $TOTAL"
echo "  Passed: $TOTAL_PASSED"
echo "  Failed: $TOTAL_FAILED"
echo "==========================================="

print_http_summary 2>/dev/null || true

if [ "$TOTAL_FAILED" -gt 0 ]; then
    exit 1
fi

exit 0
