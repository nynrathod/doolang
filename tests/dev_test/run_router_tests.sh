#!/bin/bash
# =============================================================================
# Doo Router/HTTP Test Runner
# Usage:
#   bash run_router_tests.sh          # Quiet: only show failures + summary
#   bash run_router_tests.sh -v       # Verbose: show full API response output
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

# Parse verbose flag
VERBOSE=0
VERBOSE_ARG=""
for arg in "$@"; do
    case "$arg" in -v|--verbose) VERBOSE=1; VERBOSE_ARG="-v" ;; esac
done

echo "BIN: $BIN"
echo ""

TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

EXCLUDED_FILES="run_all_tests.sh|run_http_tests.sh|run_router_tests.sh|run_features_test.sh|common.sh|create_scripts|fix_all|pretty.sh|run_middleware_tests.sh|_write_tests.py"

run_test() {
    local script="$1"
    local name
    name="$(basename "$script" .sh)"

    TOTAL_TESTS=$((TOTAL_TESTS + 1))

    echo ""
    echo "============================================================"
    echo "  [$TOTAL_TESTS] $name"
    echo "============================================================"
    echo ""

    local script_dir="$(dirname "$script")"
    local script_name="$(basename "$script")"
    local exit_code=0

    # Capture output — pass verbose flag to individual test scripts
    local actual
    if [[ "$(uname)" == "Darwin" ]]; then
        actual=$(bash -c "cd \"$script_dir\" && bash \"$script_name\" $VERBOSE_ARG" 2>&1)
        exit_code=$?
    else
        actual=$(timeout 90 bash -c "cd \"$script_dir\" && bash \"$script_name\" $VERBOSE_ARG" 2>&1)
        exit_code=$?
    fi

    if [ "$exit_code" -ne 0 ]; then
        FAILED_TESTS=$((FAILED_TESTS + 1))
        # Show ONLY failed assertions + which test case they belong to
        echo "$actual" | grep -E '(^Test [0-9]+:|FAIL)' | head -20
        echo ""
        echo "  FAILED: $name ($exit_code)"
        sleep 0.3
        return
    fi

    PASSED_TESTS=$((PASSED_TESTS + 1))
    if [ "$VERBOSE" -eq 1 ]; then
        # Show full output in verbose mode
        echo "$actual"
    else
        # Extract pass/total from summary
        local total passed
        total=$(echo "$actual" | grep -oP 'Total:\s+\K[0-9]+' | tail -1)
        passed=$(echo "$actual" | grep -oP 'Passed:\s+\K[0-9]+' | tail -1)
        echo "  OK: $name ($passed/$total assertions)"
    fi
    sleep 0.3
}

# =============================================================================
# Database tests
# =============================================================================
if [ -d "$SCRIPT_DIR/database" ]; then
    for script in "$SCRIPT_DIR"/database/*.sh; do
        [ -f "$script" ] || continue
        name="$(basename "$script")"
        if [[ "$name" =~ ^($EXCLUDED_FILES)$ ]]; then continue; fi
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
        if [[ "$name" =~ ^($EXCLUDED_FILES)$ ]]; then continue; fi
        run_test "$script"
    done
fi

# =============================================================================
# Summary
# =============================================================================
echo ""
echo "============================================================"
echo "  TEST SUMMARY"
echo "============================================================"
echo ""
echo "  Total Tests:     $TOTAL_TESTS"
echo "  Passed:          $PASSED_TESTS"
echo "  Failed:          $FAILED_TESTS"
echo ""

if [ "$FAILED_TESTS" -eq 0 ]; then
    echo "  All tests passed!"
    exit 0
else
    echo "  Some tests failed"
    exit 1
fi
