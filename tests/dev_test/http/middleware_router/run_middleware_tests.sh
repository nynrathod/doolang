#!/bin/bash
# =============================================================================
# Middleware Router Tests - Master Runner
# Runs all CORS and Rate Limit middleware tests (01-08)
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../common.sh"

echo "BIN: $BIN"
echo ""

TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# Files to exclude from test discovery
EXCLUDED_FILES="run_all.sh|run_middleware_tests.sh|server.log"

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

    local exit_code=0

    if [[ "$(uname)" == "Darwin" ]]; then
        bash -c "cd \"$SCRIPT_DIR\" && bash \"$(basename "$script")\""
        exit_code=$?
    else
        timeout 120 bash -c "cd \"$SCRIPT_DIR\" && bash \"$(basename "$script")\""
        exit_code=$?
    fi

    if [ "$exit_code" -eq 0 ]; then
        PASSED_TESTS=$((PASSED_TESTS + 1))
        echo "✅ $name passed"
    else
        FAILED_TESTS=$((FAILED_TESTS + 1))
        echo "❌ $name failed or timed out"
    fi

    sleep 0.5
}

echo "╔═══════════════════════════════════════════════════════════╗"
echo "║          MIDDLEWARE ROUTER TESTS                          ║"
echo "╚═══════════════════════════════════════════════════════════╝"
echo ""

# Run all numbered test scripts in order
for script in "$SCRIPT_DIR"/[0-9]*.sh; do
    [ -f "$script" ] || continue
    name="$(basename "$script")"
    if [[ "$name" =~ ^($EXCLUDED_FILES)$ ]]; then
        continue
    fi
    run_test "$script"
done

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
    echo "  ✅ All middleware tests passed!"
    exit 0
else
    echo "  ⚠️  Some tests failed"
    exit 1
fi
