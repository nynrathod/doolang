#!/bin/bash
# =============================================================================
# Doo Full Test Runner
# Auto-discovers and runs all tests in database/ and http/ directories
# =============================================================================

# Get script directory and source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

echo "BIN: $BIN"
echo ""

TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# Files to exclude from test discovery
EXCLUDED_FILES="run_all_tests.sh|run_http_tests.sh|run_router_tests.sh|common.sh|create_scripts|fix_all|pretty.sh"

run_test() {
    local script="$1"
    local name
    name="$(basename "$script" .sh)"

    TOTAL_TESTS=$((TOTAL_TESTS + 1))

    echo ""
    echo "══════════════════════════════════════════════════════════"
    echo "  [$TOTAL_TESTS] $name"
    echo "══════════════════════════════════════════════════════════"
    echo ""

    # Run script from its own directory
    local script_dir="$(dirname "$script")"
    local script_name="$(basename "$script")"
    local exit_code=0

    # Run the test - simple direct execution for both platforms
    # Each test script sources common.sh independently
    if [[ "$(uname)" == "Darwin" ]]; then
        # macOS: Run directly (no timeout to avoid buffering)
        bash -c "cd \"$script_dir\" && bash \"$script_name\""
        exit_code=$?
    else
        # Linux/WSL - use timeout
        timeout 90 bash -c "cd \"$script_dir\" && bash \"$script_name\""
        exit_code=$?
    fi

    if [ "$exit_code" -eq 0 ]; then
        PASSED_TESTS=$((PASSED_TESTS + 1))
        echo "✅ $name passed"
    else
        FAILED_TESTS=$((FAILED_TESTS + 1))
        echo "❌ $name failed or timed out"
    fi

    # Pause for cleanup
    sleep 0.3
}

# =============================================================================
# Database tests
# =============================================================================
if [ -d "$SCRIPT_DIR/database" ]; then

    for script in "$SCRIPT_DIR"/database/*.sh; do
        [ -f "$script" ] || continue
        name="$(basename "$script")"
        if [[ "$name" =~ ^($EXCLUDED_FILES)$ ]]; then
            continue
        fi
        run_test "$script"
    done
fi

# =============================================================================
# HTTP tests
# =============================================================================
if [ -d "$SCRIPT_DIR/http" ]; then

    for script in "$SCRIPT_DIR"/http/*.sh; do
        [ -f "$script" ] || continue
        name="$(basename "$script")"
        if [[ "$name" =~ ^($EXCLUDED_FILES)$ ]]; then
            continue
        fi
        run_test "$script"
    done
fi

# =============================================================================
# Summary
# =============================================================================
echo ""
echo "╔═══════════════════════════════════════════════════════════╗"
echo "║                      TEST SUMMARY                         ║"
echo "╚═══════════════════════════════════════════════════════════╝"
echo ""
echo "  Total Tests:  $TOTAL_TESTS"
echo "  Passed:       $PASSED_TESTS"
echo "  Failed:       $FAILED_TESTS"
echo ""

if [ "$FAILED_TESTS" -eq 0 ]; then
    echo "  ✅ All tests passed!"
    exit 0
else
    echo "  ⚠️  Some tests failed"
    exit 1
fi
