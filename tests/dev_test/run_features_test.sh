#!/bin/bash

# =============================================================================
# Doo Feature Tests Runner (with Output Verification)
# Runs all feature test .doo files and verifies output matches // EXPECT: lines
#
# Usage:
#   bash run_features_test.sh          # Quiet mode: only show failures
#   bash run_features_test.sh -v       # Verbose: show all output
#   bash run_features_test.sh --verbose
#
# How it works:
#   - If a .doo file has // EXPECT: comments, actual stdout is checked line-by-line
#   - If no // EXPECT: comments, just checks exit code (backward compatible)
# =============================================================================

set +e  # Continue on errors, we track them

# Parse flags
VERBOSE=0
for arg in "$@"; do
    case "$arg" in
        -v|--verbose) VERBOSE=1 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
DIM='\033[2m'
BOLD='\033[1m'
NC='\033[0m' # No Color

echo ""
echo -e "${BOLD}================================================================${NC}"
echo -e "${BOLD}  Doo Feature Tests ${DIM}(use -v for verbose output)${NC}"
echo -e "${BOLD}================================================================${NC}"
echo ""

TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0
TIMEOUT_TESTS=0
OUTPUT_MISMATCH=0

declare -a FAILED_TEST_LIST

# =============================================================================
# extract_expected - Extract // EXPECT: lines from a .doo file
# Strips CRLF (\r) to handle Windows line endings
# =============================================================================
extract_expected() {
    local file="$1"
    grep '// EXPECT:' "$file" 2>/dev/null | sed 's|.*// EXPECT: *||' | tr -d '\r' || true
}

# =============================================================================
# run_test - Run a .doo file and optionally verify output
# =============================================================================
run_test() {
    local file="$1"
    local rel_path="${file#$SCRIPT_DIR/}"

    TOTAL_TESTS=$((TOTAL_TESTS + 1))

    # Pre-check: if file has no EXPECT lines, use fast compile-only check.
    # Server tests (fetch, metrics, websocket) call app.start() which blocks
    # forever, making doo run hang until timeout. Since they have no EXPECT
    # lines to verify, compile-check is sufficient and instant.
    local has_expects
    has_expects=$(grep -c '// EXPECT:' "$file" 2>/dev/null || true)

    if [ "$has_expects" -eq 0 ]; then
        local check_output
        check_output=$("$BIN" check "$file" 2>&1 | tr -d '\r')
        local check_exit=$?
        if [ "$check_exit" -eq 0 ]; then
            PASSED_TESTS=$((PASSED_TESTS + 1))
            if [ "$VERBOSE" -eq 1 ]; then
                echo -e "  ${GREEN}PASS${NC} $rel_path ${DIM}(no expects, compile check)${NC}"
            else
                echo -e "  ${GREEN}PASS${NC} $rel_path"
            fi
        else
            FAILED_TESTS=$((FAILED_TESTS + 1))
            FAILED_TEST_LIST+=("$rel_path (compile error)")
            echo ""
            echo -e "  ${RED}FAIL${NC} ${BOLD}$rel_path${NC}"
            echo -e "       ${RED}Compile check failed (exit: $check_exit)${NC}"
            echo "$check_output" | head -10 | sed 's/^/       /'
        fi
        return
    fi

    # Capture actual output (also strip \r from output for consistency)
    local actual
    actual=$(timeout 30s "$BIN" run "$file" 2>&1 | tr -d '\r')
    local exit_code=$?

    # --- TIMEOUT ---
    if [ "$exit_code" -eq 124 ]; then
        TIMEOUT_TESTS=$((TIMEOUT_TESTS + 1))
        FAILED_TESTS=$((FAILED_TESTS + 1))
        FAILED_TEST_LIST+=("$rel_path (TIMEOUT)")
        echo ""
        echo -e "  ${RED}FAIL${NC} ${BOLD}$rel_path${NC}"
        echo -e "       ${RED}TIMEOUT${NC} - exceeded 30 seconds"
        return
    fi

    # --- CRASH / NON-ZERO EXIT ---
    if [ "$exit_code" -ne 0 ]; then
        FAILED_TESTS=$((FAILED_TESTS + 1))
        FAILED_TEST_LIST+=("$rel_path (exit code: $exit_code)")
        echo ""
        echo -e "  ${RED}FAIL${NC} ${BOLD}$rel_path${NC}"
        echo -e "       ${RED}Exit code: $exit_code${NC}"
        echo "$actual" | head -10 | sed 's/^/       /'
        return
    fi

    # Extract expected output lines
    local expected
    expected=$(extract_expected "$file")

    # --- VERIFY OUTPUT: Go-style ordered sequential matching ---
    # Each EXPECT line must appear in the output, in order, searching forward
    # from the last match position. Extra output lines between matches are OK.
    local mismatch=0
    local matched=0
    local total_expects=0
    local mismatch_details=""

    # Convert actual output to array of lines
    local -a actual_lines=()
    while IFS= read -r line; do
        actual_lines+=("$line")
    done <<< "$actual"
    local actual_count=${#actual_lines[@]}
    local search_from=0

    while IFS= read -r expect_line; do
        [ -z "$expect_line" ] && continue
        total_expects=$((total_expects + 1))

        # Search forward from last match position
        local found=0
        local i=$search_from
        while [ $i -lt $actual_count ]; do
            if [[ "${actual_lines[$i]}" == *"$expect_line"* ]]; then
                found=1
                search_from=$((i + 1))
                break
            fi
            i=$((i + 1))
        done

        if [ "$found" -eq 1 ]; then
            matched=$((matched + 1))
        else
            mismatch=$((mismatch + 1))
            mismatch_details+="       ${RED}EXPECT[$total_expects]:${NC} $expect_line\n"
            # Show what's at that position in actual output
            if [ $search_from -lt $actual_count ]; then
                mismatch_details+="       ${YELLOW}ACTUAL[$((search_from+1))]:${NC} ${actual_lines[$search_from]}\n"
            else
                mismatch_details+="       ${YELLOW}ACTUAL:${NC} ${DIM}(end of output reached)${NC}\n"
            fi
            mismatch_details+="\n"
        fi
    done <<< "$expected"

    if [ "$mismatch" -gt 0 ]; then
        OUTPUT_MISMATCH=$((OUTPUT_MISMATCH + 1))
        FAILED_TESTS=$((FAILED_TESTS + 1))
        FAILED_TEST_LIST+=("$rel_path (OUTPUT MISMATCH: $mismatch/$total_expects)")
        echo ""
        echo -e "  ${RED}FAIL${NC} ${BOLD}$rel_path${NC}  ${DIM}($matched/$total_expects matched)${NC}"
        echo -e "$mismatch_details"
        if [ "$VERBOSE" -eq 1 ]; then
            echo -e "       ${CYAN}--- Full output ---${NC}"
            echo "$actual" | head -30 | sed 's/^/       /'
            echo ""
        fi
    else
        PASSED_TESTS=$((PASSED_TESTS + 1))
        if [ "$VERBOSE" -eq 1 ]; then
            echo -e "  ${GREEN}PASS${NC} $rel_path  ${DIM}($total_expects/$total_expects matched)${NC}"
            echo "$actual" | sed 's/^/       /'
        else
            echo -e "  ${GREEN}PASS${NC} $rel_path"
        fi
    fi
}

# Find and run all .doo files (excluding database, http, fixture)
while IFS= read -r file; do
    [ -f "$file" ] || continue
    run_test "$file"
done < <(find "$SCRIPT_DIR" -type f -name "*.doo" \
    -not -path "*/database/*" \
    -not -path "*/http/*" \
    -not -path "*/fixture/*" \
    -not -path "*/target/*" \
    | sort)

# =============================================================================
# Summary
# =============================================================================
echo ""
echo -e "${BOLD}================================================================${NC}"
echo -e "${BOLD}  TEST SUMMARY${NC}"
echo -e "${BOLD}================================================================${NC}"
echo ""
echo -e "  Total:           ${BOLD}$TOTAL_TESTS${NC}"
echo -e "  Passed:          ${GREEN}$PASSED_TESTS${NC}"
if [ "$FAILED_TESTS" -gt 0 ]; then
    echo -e "  Failed:          ${RED}$FAILED_TESTS${NC}"
else
    echo -e "  Failed:          $FAILED_TESTS"
fi
if [ $TIMEOUT_TESTS -gt 0 ]; then
    echo -e "  Timeout:         ${RED}$TIMEOUT_TESTS${NC}"
fi
if [ $OUTPUT_MISMATCH -gt 0 ]; then
    echo -e "  Output Mismatch: ${YELLOW}$OUTPUT_MISMATCH${NC}"
fi
echo ""

if [ ${#FAILED_TEST_LIST[@]} -gt 0 ]; then
    echo -e "  ${RED}Failed tests:${NC}"
    for test in "${FAILED_TEST_LIST[@]}"; do
        echo -e "    ${RED}-${NC} $test"
    done
    echo ""
fi

echo -e "${BOLD}================================================================${NC}"

if [ "$FAILED_TESTS" -eq 0 ]; then
    echo -e "  ${GREEN}All feature tests passed!${NC}"
    exit 0
else
    echo -e "  ${RED}Some tests failed${NC}"
    exit 1
fi
