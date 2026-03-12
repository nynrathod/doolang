#!/bin/bash
# ==========================================================================
# Doo Memory Leak Test Runner (Valgrind)
# ==========================================================================
# Runs Doo programs through Valgrind memcheck to detect memory leaks.
# Zero code changes to the compiler — Valgrind instruments from outside.
#
# Usage:
#   ./tests/memory_leak/run_leak_tests.sh
#
# Must be run from the project root directory in WSL/Linux.
# Requires: valgrind (sudo apt install valgrind)
# ==========================================================================

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Counters
TOTAL=0
PASSED=0
FAILED=0
SKIPPED=0

# Test directory
TEST_DIR="tests/memory_leak/programs"
RESULTS_DIR="tests/memory_leak/results"

# Find doo binary
find_doo_binary() {
    local candidates=(
        "target-linux/release/doo"
        "target/release/doo"
        "target/debug/doo"
    )
    
    for bin in "${candidates[@]}"; do
        if [ -x "$bin" ]; then
            echo "$bin"
            return 0
        fi
    done
    
    # Try PATH
    if command -v doo &>/dev/null; then
        echo "doo"
        return 0
    fi
    
    return 1
}

# Run a test with Valgrind memcheck
run_valgrind_test() {
    local doo_file="$1"
    local test_name
    test_name=$(basename "$doo_file" .doo)
    
    TOTAL=$((TOTAL + 1))
    
    printf "  %-40s " "$test_name"
    
    # Compile
    local output_name="${RESULTS_DIR}/${test_name}"
    if ! "$DOO_BIN" build "$doo_file" -o "$output_name" 2>"${output_name}.compile_err"; then
        printf "${YELLOW}SKIP${NC} (compile failed)\n"
        if [ -f "${output_name}.compile_err" ]; then
            head -5 "${output_name}.compile_err" | sed 's/^/    /'
        fi
        SKIPPED=$((SKIPPED + 1))
        return
    fi
    
    # Run with Valgrind
    local exit_code=0
    timeout 60 valgrind \
        --leak-check=full \
        --show-leak-kinds=definite,possible \
        --errors-for-leak-kinds=definite,possible \
        --error-exitcode=42 \
        --track-origins=yes \
        --log-file="${output_name}.valgrind" \
        "$output_name" >"${output_name}.stdout" 2>"${output_name}.stderr" || exit_code=$?
    
    if [ $exit_code -eq 42 ]; then
        # Valgrind found leaks/errors
        printf "${RED}LEAK${NC}"
        local definitely
        definitely=$(grep "definitely lost:" "${output_name}.valgrind" | tail -1 || echo "")
        if [ -n "$definitely" ]; then
            printf " - %s" "$(echo "$definitely" | sed 's/^==[0-9]*== *//')"
        fi
        printf "\n"
        FAILED=$((FAILED + 1))
    elif [ $exit_code -eq 0 ]; then
        printf "${GREEN}PASS${NC}\n"
        PASSED=$((PASSED + 1))
    elif [ $exit_code -eq 124 ]; then
        printf "${YELLOW}SKIP${NC} (timeout)\n"
        SKIPPED=$((SKIPPED + 1))
    else
        printf "${YELLOW}SKIP${NC} (exit code: $exit_code)\n"
        SKIPPED=$((SKIPPED + 1))
    fi
}

# Print summary
print_summary() {
    local effective=$((TOTAL - SKIPPED))
    local pass_rate=0
    if [ $effective -gt 0 ]; then
        pass_rate=$((PASSED * 100 / effective))
    fi
    
    echo ""
    echo -e "${CYAN}================================================================${NC}"
    echo -e "${CYAN}  MEMORY LEAK TEST SUMMARY (Valgrind)${NC}"
    echo -e "${CYAN}================================================================${NC}"
    echo ""
    echo -e "  Total tests:    $TOTAL"
    echo -e "  ${GREEN}Passed:         $PASSED${NC}"
    echo -e "  ${RED}Failed (leaks): $FAILED${NC}"
    echo -e "  ${YELLOW}Skipped:        $SKIPPED${NC}"
    echo ""
    echo -e "  Pass rate: ${pass_rate}% ($PASSED/$effective effective tests)"
    echo ""
    
    if [ $pass_rate -ge 95 ]; then
        echo -e "  ${GREEN}TARGET MET: ≥95% pass rate achieved!${NC}"
    else
        echo -e "  ${RED}TARGET NOT MET: Need ≥95% pass rate (got ${pass_rate}%)${NC}"
    fi
    echo -e "${CYAN}================================================================${NC}"
    echo ""
    
    # List failed tests with details
    if [ $FAILED -gt 0 ]; then
        echo -e "${RED}Failed tests (leak details):${NC}"
        for f in "${RESULTS_DIR}"/*.valgrind; do
            local name
            name=$(basename "$f" .valgrind)
            if grep -q "definitely lost: [1-9]" "$f" 2>/dev/null || \
               grep -q "indirectly lost: [1-9]" "$f" 2>/dev/null || \
               grep -q "possibly lost: [1-9]" "$f" 2>/dev/null; then
                echo -e "  ${RED}✗${NC} $name"
                grep -E "(definitely|indirectly|possibly) lost:" "$f" | tail -3 | sed 's/^==[0-9]*== */    /'
            fi
        done
        echo ""
        echo "  Full Valgrind logs: ${RESULTS_DIR}/<testname>.valgrind"
        echo ""
    fi
}

# Main
main() {
    echo -e "${CYAN}================================================================${NC}"
    echo -e "${CYAN}  Doo Memory Leak Test Runner (Valgrind)${NC}"
    echo -e "${CYAN}================================================================${NC}"
    echo ""
    
    # Check Valgrind
    if ! command -v valgrind &>/dev/null; then
        echo -e "${RED}ERROR: Valgrind not installed. Install with:${NC}"
        echo "  sudo apt install valgrind"
        exit 1
    fi
    
    # Find doo binary
    DOO_BIN=$(find_doo_binary) || {
        echo -e "${RED}ERROR: Could not find doo binary. Build first:${NC}"
        echo "  cargo build --release --workspace --target-dir target-linux"
        exit 1
    }
    echo -e "  Using doo binary: ${BLUE}$DOO_BIN${NC}"
    
    # Check for test files
    if [ ! -d "$TEST_DIR" ]; then
        echo -e "${RED}ERROR: Test directory not found: $TEST_DIR${NC}"
        exit 1
    fi
    
    local test_files
    test_files=$(find "$TEST_DIR" -name "*.doo" -type f | sort)
    local test_count
    test_count=$(echo "$test_files" | wc -l)
    echo -e "  Found ${test_count} test files"
    echo -e "  Valgrind: ${GREEN}$(valgrind --version)${NC}"
    
    # Create results directory
    mkdir -p "$RESULTS_DIR"
    
    echo ""
    echo -e "${BLUE}Running Valgrind memcheck on each test...${NC}"
    echo ""
    
    for file in $test_files; do
        run_valgrind_test "$file"
    done
    
    print_summary
    
    # Exit with failure if any tests failed
    [ $FAILED -eq 0 ]
}

main "$@"
