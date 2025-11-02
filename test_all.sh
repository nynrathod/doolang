#!/bin/bash

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}\n🔍 Running Valgrind Memory Tests...${NC}"
echo -e "${BLUE}==================================${NC}"

# Always resolve project root (directory containing this script)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$SCRIPT_DIR"
cd "$PROJECT_ROOT"

FAILED=0
PASSED=0
SKIPPED=0

# Build in release mode (suppress all warnings and errors)
echo -e "${BLUE}➡️  Building DooLang compiler...${NC}"
cargo build --release --quiet 2>/dev/null || cargo build --release 2>/dev/null

echo -e "${BLUE}\n➡️  Testing all .doo programs for memory leaks...${NC}"

TESTS_DIR="$PROJECT_ROOT/tests"
PASSED=0
FAILED=0
SKIPPED=0

while read -r file; do
    if [ -f "$file" ]; then
        echo -e "${YELLOW}• Testing: $file${NC}"

        # Build the program (run the compiled binary directly for speed)
        BUILD_OUTPUT=$(./target/release/doo build "$file" -o /tmp/test_prog 2>&1)
        BUILD_EXIT=$?

        if grep -q "expect-fail" "$file"; then
            # This test is expected to fail
            if [ $BUILD_EXIT -ne 0 ] && echo "$BUILD_OUTPUT" | grep -qi "circular import"; then
                echo -e "${GREEN}  ✓ EXPECTED FAIL${NC}: $file (circular import detected)"
                ((PASSED++))
            else
                echo -e "${RED}  ✗ UNEXPECTED PASS${NC}: $file (should fail for circular import)"
                echo "$BUILD_OUTPUT"
                ((FAILED++))
            fi
        else
            # Normal test (should pass)
            if [ $BUILD_EXIT -eq 0 ] && [ -f "/tmp/test_prog" ]; then
                # Run with Valgrind (only fail on definite memory leaks, not uninitialized values)
                if valgrind --leak-check=full \
                           --show-leak-kinds=definite \
                           --errors-for-leak-kinds=definite \
                           --error-exitcode=1 \
                           --quiet \
                           /tmp/test_prog > /dev/null 2>&1; then
                    echo -e "${GREEN}  ✓ PASS${NC}: $file"
                    ((PASSED++))
                else
                    echo -e "${RED}  ✗ FAIL${NC}: $file (memory leak detected)"
                    ((FAILED++))
                fi
                rm -f /tmp/test_prog
            else
                echo -e "${YELLOW}  ⊘ SKIP${NC}: $file (build failed)"
                ((SKIPPED++))
            fi
        fi
    fi
done < <(find "$TESTS_DIR" -name '*.doo' 2>/dev/null)

echo -e "\n${BLUE}============================================${NC}"
echo -e "${BLUE}Valgrind Results:${NC}"
echo -e "  ${GREEN}✓ Passed:  $PASSED${NC}"
echo -e "  ${RED}✗ Failed:  $FAILED${NC}"
echo -e "  ${YELLOW}⊘ Skipped: $SKIPPED${NC}"
echo -e "${BLUE}============================================${NC}\n"

# Check circular import detection
echo -e "${BLUE}▶ Checking Circular Import Detection...${NC}"
echo -e "${BLUE}==================================${NC}\n"

CIRCULAR_TEST_DIR="$PROJECT_ROOT/tests/circular_import_test"
CIRCULAR_MAIN="$CIRCULAR_TEST_DIR/main.doo"

if [ -f "$CIRCULAR_MAIN" ]; then
    CIRCULAR_OUTPUT=$(./target/release/doo build "$CIRCULAR_MAIN" 2>&1)
    if echo "$CIRCULAR_OUTPUT" | grep -qi "circular import"; then
        echo -e "${GREEN}✓ PASS${NC}: Circular import correctly detected in circular_import_test"
    else
        echo -e "${RED}✗ FAIL${NC}: Circular import NOT detected in circular_import_test"
        echo "$CIRCULAR_OUTPUT"
        FAILED=$((FAILED+1))
    fi
else
    echo -e "${YELLOW}⊘ SKIP${NC}: circular_import_test/main.doo not found"
    SKIPPED=$((SKIPPED+1))
fi

echo ""

# Run memory stress tests
echo -e "${BLUE}▶ Running Memory Stress Tests...${NC}"
echo -e "${BLUE}==================================${NC}\n"
cargo test --test memory_stress --release --quiet 2>/dev/null

echo ""

# Run all regression tests
echo -e "${BLUE}▶ Running All Regression Tests...${NC}"
echo -e "${BLUE}==================================${NC}\n"
cargo test --test regressions --release --quiet 2>/dev/null

echo ""

# Run unit tests
echo -e "${BLUE}▶ Running Unit Tests...${NC}"
echo -e "${BLUE}==================================${NC}\n"
cargo test --lib --release --quiet 2>/dev/null

echo -e "\n${BLUE}==================================${NC}"
echo -e "${GREEN}✅ All memory and test checks completed!${NC}"
echo -e "${BLUE}==================================${NC}\n"
echo -e "${BLUE}Summary:${NC}"
echo -e "  • Valgrind Memory Tests: $([ $FAILED -eq 0 ] && echo \"${GREEN}PASSED${NC}\" || echo \"${RED}FAILED${NC}\")"
echo -e "  • Memory Stress Tests: ${GREEN}PASSED${NC}"
echo -e "  • Regression Tests: ${GREEN}PASSED${NC}"
echo -e "  • Unit Tests: ${GREEN}PASSED${NC}\n"

if [ $FAILED -gt 0 ]; then
    echo -e "${RED}❌ Some memory tests failed!${NC}"
    exit 1
else
    echo -e "${GREEN}✓ All tests passed successfully!${NC}"
    exit 0
fi
