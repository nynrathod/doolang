#!/bin/bash
# =============================================================================
# Interface Feature Tests
# Runs interface .doo test files and verifies output
#
# Usage:
#   bash test_interface.sh          # Run all interface tests
#   bash test_interface.sh -v       # Verbose mode
# =============================================================================

set +e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

VERBOSE=0
for arg in "$@"; do
    case "$arg" in
        -v|--verbose) VERBOSE=1 ;;
    esac
done

PASS=0
FAIL=0
TOTAL=0

echo ""
echo -e "${BOLD}================================================================${NC}"
echo -e "${BOLD}  Interface Feature Tests${NC}"
echo -e "${BOLD}================================================================${NC}"
echo ""

run_test() {
    local file="$1"
    local name="$(basename "$file" .doo)"
    TOTAL=$((TOTAL + 1))

    if [ ! -f "$file" ]; then
        echo -e "  ${RED}✗ MISSING${NC}  $name — file not found"
        FAIL=$((FAIL + 1))
        return
    fi

    # Extract EXPECT lines
    local expect_file=$(mktemp)
    grep '// EXPECT:' "$file" | sed 's/.*\/\/ EXPECT: //' > "$expect_file"

    # Run the doo program
    local output
    output=$("$BIN" run "$file" 2>&1)
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "  ${RED}✗ FAIL${NC}    $name — exit code $exit_code"
        if [ $VERBOSE -eq 1 ]; then
            echo -e "    ${YELLOW}stderr:${NC}"
            echo "$output" | sed 's/^/      /'
        fi
        FAIL=$((FAIL + 1))
        rm -f "$expect_file"
        return
    fi

    # If there are EXPECT lines, verify output
    if [ -s "$expect_file" ]; then
        local expected_count=0
        local matched_count=0
        local mismatches=""

        while IFS= read -r expected_line; do
            expected_count=$((expected_count + 1))
            # Check if expected line appears in output
            if echo "$output" | grep -qF "$expected_line"; then
                matched_count=$((matched_count + 1))
            else
                mismatches="$mismatches\n        expected: $expected_line"
            fi
        done < "$expect_file"

        if [ "$matched_count" -eq "$expected_count" ]; then
            echo -e "  ${GREEN}✓ PASS${NC}    $name ($matched_count/$expected_count expects)"
            PASS=$((PASS + 1))
        else
            echo -e "  ${RED}✗ FAIL${NC}    $name ($matched_count/$expected_count expects matched)"
            if [ -n "$mismatches" ]; then
                echo -e "    ${YELLOW}Missing expected output:${NC}"
                echo -e "$mismatches"
            fi
            if [ $VERBOSE -eq 1 ]; then
                echo -e "    ${YELLOW}Actual output:${NC}"
                echo "$output" | sed 's/^/      /'
            fi
            FAIL=$((FAIL + 1))
        fi
    else
        # No EXPECT lines, just check exit code
        echo -e "  ${GREEN}✓ PASS${NC}    $name (exit 0)"
        PASS=$((PASS + 1))
    fi

    rm -f "$expect_file"
}

# Run all interface test files
for file in "$SCRIPT_DIR"/*.doo; do
    if [ -f "$file" ]; then
        run_test "$file"
    fi
done

echo ""
echo -e "${BOLD}----------------------------------------------------------------${NC}"
echo -e "  Results: ${GREEN}$PASS passed${NC}, ${RED}$FAIL failed${NC}, $TOTAL total"
echo -e "${BOLD}----------------------------------------------------------------${NC}"
echo ""

if [ $FAIL -gt 0 ]; then
    exit 1
fi
exit 0
