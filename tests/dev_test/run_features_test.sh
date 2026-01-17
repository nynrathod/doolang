#!/bin/bash

# =============================================================================
# Doo Feature Tests Runner
# Runs all feature test .doo files with full output
# =============================================================================

set +e  # Continue on errors, we track them

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

echo "════════════════════════════════════════════════════════════"
echo "  🧪 Running Feature Tests"
echo "════════════════════════════════════════════════════════════"
echo ""

TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0
TIMEOUT_TESTS=0

declare -a FAILED_TEST_LIST

run_test() {
    local file="$1"
    local rel_path="${file#$SCRIPT_DIR/}"

    TOTAL_TESTS=$((TOTAL_TESTS + 1))

    echo ""
    echo "──────────────────────────────────────────────────────────"
    echo "[$TOTAL_TESTS] $rel_path"
    echo "──────────────────────────────────────────────────────────"

    # Run with 30 second timeout, show full output
    timeout 30s "$BIN" run "$file" 2>&1
    exit_code=$?

    if [ "$exit_code" -eq 0 ]; then
        PASSED_TESTS=$((PASSED_TESTS + 1))
    elif [ "$exit_code" -eq 124 ]; then
        TIMEOUT_TESTS=$((TIMEOUT_TESTS + 1))
        FAILED_TESTS=$((FAILED_TESTS + 1))
        FAILED_TEST_LIST+=("$rel_path (TIMEOUT)")
        echo ""
        echo "⏱️  TIMEOUT - Test exceeded 30 seconds"
    else
        FAILED_TESTS=$((FAILED_TESTS + 1))
        FAILED_TEST_LIST+=("$rel_path (exit code: $exit_code)")
        echo ""
        echo "❌ Test failed with exit code: $exit_code"
    fi
}

# Find and run all .doo files (excluding database and http)
while IFS= read -r file; do
    [ -f "$file" ] || continue
    run_test "$file"
done < <(find "$SCRIPT_DIR" -type f -name "*.doo" \
    -not -path "*/database/*" \
    -not -path "*/http/*" \
    -not -path "*/target/*" \
    | sort)

# =============================================================================
# Summary
# =============================================================================
echo ""
echo "════════════════════════════════════════════════════════════"
echo "  📊 TEST SUMMARY"
echo "════════════════════════════════════════════════════════════"
echo ""
echo "  Total:    $TOTAL_TESTS"
echo "  Passed:   $PASSED_TESTS ✅"
echo "  Failed:   $FAILED_TESTS ❌"
if [ $TIMEOUT_TESTS -gt 0 ]; then
    echo "  Timeout:  $TIMEOUT_TESTS ⏱️"
fi
echo ""

if [ ${#FAILED_TEST_LIST[@]} -gt 0 ]; then
    echo "  Failed tests:"
    for test in "${FAILED_TEST_LIST[@]}"; do
        echo "    • $test"
    done
    echo ""
fi

echo "════════════════════════════════════════════════════════════"

if [ "$FAILED_TESTS" -eq 0 ]; then
    echo "  ✅ All feature tests passed!"
    exit 0
else
    echo "  ⚠️  Some tests failed or timed out"
    exit 1
fi
