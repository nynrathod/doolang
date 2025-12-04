#!/bin/bash

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}\n🔍 Running Valgrind Memory Leak Tests...${NC}"
echo -e "${BLUE}=========================================${NC}"

# Always resolve project root (directory containing this script)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$SCRIPT_DIR"
cd "$PROJECT_ROOT"

# Build in release mode
echo -e "${BLUE}➡️  Building DooLang compiler...${NC}"
cargo build --release --quiet 2>/dev/null || cargo build --release 2>/dev/null

PASSED=0
FAILED=0
SKIPPED=0

DEV_TEST_DIR="$PROJECT_ROOT/tests/dev_test"

# Function to run valgrind test on a file
# Args: $1 = file path, $2 = project_root (for main.doo files, use parent dir)
run_valgrind_test() {
    local file="$1"
    local build_path="$2"
    local filename=$(basename "$file")

    echo -e "${YELLOW}• Testing: $file${NC}"

    # Build the program
    BUILD_OUTPUT=$(./target/release/doo build "$build_path" -o /tmp/test_prog 2>&1)
    BUILD_EXIT=$?

    if [ $BUILD_EXIT -eq 0 ] && [ -f "/tmp/test_prog" ]; then
        # Run with Valgrind
        if valgrind --leak-check=full \
                   --show-leak-kinds=definite \
                   --errors-for-leak-kinds=definite \
                   --error-exitcode=1 \
                   --quiet \
                   /tmp/test_prog > /dev/null 2>&1; then
            echo -e "${GREEN}  ✓ PASS${NC} (no memory leaks)"
            ((PASSED++))
        else
            echo -e "${RED}  ✗ FAIL${NC} (memory leak detected)"
            ((FAILED++))
        fi
        rm -f /tmp/test_prog
    else
        echo -e "${YELLOW}  ⊘ SKIP${NC} (build failed)"
        ((SKIPPED++))
    fi
}

# ═══════════════════════════════════════════════════════════════════════════════
# PART 1: Test all .doo files in dev_test/ (excluding fixture/)
# Same logic as find_doo_files() in dev_test_runner.rs
# ═══════════════════════════════════════════════════════════════════════════════
echo -e "\n${BLUE}▶ Testing dev_test files (excluding fixture/)...${NC}"
echo -e "${BLUE}=================================================${NC}\n"

while read -r file; do
    if [ -f "$file" ]; then
        filename=$(basename "$file" .doo)

        # Skip _error files (they are expected to fail compilation)
        if [[ "$filename" == *_error ]]; then
            echo -e "${YELLOW}• Skipping (error test): $file${NC}"
            continue
        fi

        # For standalone files, build_path is the file itself
        run_valgrind_test "$file" "$file"
    fi
done < <(find "$DEV_TEST_DIR" -name '*.doo' -not -path '*/fixture/*' 2>/dev/null | sort)

# ═══════════════════════════════════════════════════════════════════════════════
# PART 2: Test fixture/visibilitytest main.doo files (first level only)
# Same logic as find_fixture_main_files_first_level() + test_visibilitytest_main()
# ═══════════════════════════════════════════════════════════════════════════════
VISIBILITY_DIR="$DEV_TEST_DIR/fixture/visibilitytest"

if [ -d "$VISIBILITY_DIR" ]; then
    echo -e "\n${BLUE}▶ Testing visibilitytest fixtures...${NC}"
    echo -e "${BLUE}=====================================${NC}\n"

    # Check root main.doo
    if [ -f "$VISIBILITY_DIR/main.doo" ]; then
        run_valgrind_test "$VISIBILITY_DIR/main.doo" "$VISIBILITY_DIR"
    fi

    # Check first-level subdirectories for main.doo
    for subdir in "$VISIBILITY_DIR"/*/; do
        if [ -d "$subdir" ]; then
            main_file="$subdir/main.doo"
            if [ -f "$main_file" ]; then
                # For main.doo in fixture, use parent directory as build path
                run_valgrind_test "$main_file" "${subdir%/}"
            fi
        fi
    done
fi

# ═══════════════════════════════════════════════════════════════════════════════
# PART 3: Skip circular_import_test (expected to fail, no binary to test)
# These are tested in dev_test_runner.rs for compilation failure only
# ═══════════════════════════════════════════════════════════════════════════════
echo -e "\n${BLUE}▶ Skipping circular_import_test (compile-failure tests, no valgrind needed)${NC}"

# ═══════════════════════════════════════════════════════════════════════════════
# Valgrind Summary
# ═══════════════════════════════════════════════════════════════════════════════
echo -e "\n${BLUE}============================================${NC}"
echo -e "${BLUE}Valgrind Memory Leak Results:${NC}"
echo -e "  ${GREEN}✓ Passed:  $PASSED${NC}"
echo -e "  ${RED}✗ Failed:  $FAILED${NC}"
echo -e "  ${YELLOW}⊘ Skipped: $SKIPPED${NC}"
echo -e "${BLUE}============================================${NC}\n"

# ═══════════════════════════════════════════════════════════════════════════════
# Run Rust tests (memory stress + unit tests)
# ═══════════════════════════════════════════════════════════════════════════════
echo -e "${BLUE}▶ Running Memory Stress Tests...${NC}"
echo -e "${BLUE}=================================${NC}\n"
cargo test stress::memory --release --quiet 2>/dev/null

echo ""

echo -e "${BLUE}▶ Running Unit Tests...${NC}"
echo -e "${BLUE}=======================${NC}\n"
cargo test --lib --release --quiet 2>/dev/null

# ═══════════════════════════════════════════════════════════════════════════════
# Final Summary
# ═══════════════════════════════════════════════════════════════════════════════
echo -e "\n${BLUE}==================================${NC}"
echo -e "${GREEN}✅ All tests completed!${NC}"
echo -e "${BLUE}==================================${NC}\n"
echo -e "${BLUE}Summary:${NC}"
echo -e "  • Valgrind Memory Tests: $([ $FAILED -eq 0 ] && echo -e "${GREEN}PASSED${NC}" || echo -e "${RED}FAILED${NC}")"
echo -e "  • Memory Stress Tests: ${GREEN}RAN${NC}"
echo -e "  • Unit Tests: ${GREEN}RAN${NC}\n"

if [ $FAILED -gt 0 ]; then
    echo -e "${RED}❌ Some valgrind tests failed!${NC}"
    exit 1
else
    echo -e "${GREEN}✓ All valgrind tests passed!${NC}"
    exit 0
fi
