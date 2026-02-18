#!/bin/bash
# =============================================================================
# Process Module Comprehensive Test Suite
# Tests: run, output, spawn, kill, status, waitForOutput, isRunning, shutdown
# =============================================================================
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

FILE="main.doo"

echo "=== Doo Process Module Test Suite ==="
echo ""

# Build and run via doo (not a server test — just a CLI test)
if [ ! -x "$BIN" ]; then
    echo "Building doo binary for tests..."
    (cd "$PROJECT_ROOT" && cargo build --release --workspace >/dev/null 2>&1) || true
fi

if [ ! -x "$BIN" ]; then
    echo "❌ doo binary not found or not executable at: $BIN"
    exit 1
fi

echo "Running process tests..."
echo ""

# Run the doo program and capture output
OUTPUT_FILE=$(mktemp)
"$BIN" run "$FILE" > "$OUTPUT_FILE" 2>&1 || true

cat "$OUTPUT_FILE"
echo ""

# Parse test results from output
TOTAL_PASS=0
TOTAL_FAIL=0

# Test 1: Process::run (echo)
echo "--- Verifying Test Results ---"

if grep -q '"exit_code":0' "$OUTPUT_FILE" && grep -q 'hello_doo' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: TEST 1 — Process::run echo"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 1 — Process::run echo"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 2: Process::output
if grep -q 'process_output_test' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: TEST 2 — Process::output"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 2 — Process::output"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 3: Process::run with args
if grep -q 'hello world' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: TEST 3 — Process::run with args"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 3 — Process::run with args"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 4: Process::run (false — exit code 1)
if grep -q '"exit_code":1' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: TEST 4 — Process::run exit code 1"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 4 — Process::run exit code 1"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 5: spawn + isRunning + kill
if grep -q 'isRunning = true' "$OUTPUT_FILE" || grep -q 'isRunning = 1' "$OUTPUT_FILE"; then
    if grep -q 'isRunning after kill = false' "$OUTPUT_FILE" || grep -q 'isRunning after kill = 0' "$OUTPUT_FILE"; then
        echo "  ✅ PASS: TEST 5 — spawn + isRunning + kill"
        TOTAL_PASS=$((TOTAL_PASS + 1))
    else
        echo "  ❌ FAIL: TEST 5 — isRunning after kill should be false"
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
    fi
else
    echo "  ❌ FAIL: TEST 5 — spawn + isRunning + kill"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 6: spawn + status
if grep -q '"status":"running"' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: TEST 6 — spawn + status (running)"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 6 — spawn + status"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 7: spawn + waitForOutput
if grep -q 'wait_test' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: TEST 7 — spawn + waitForOutput"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 7 — spawn + waitForOutput"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 8: activeCount
if grep -q 'active count' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: TEST 8 — activeCount"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 8 — activeCount"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 9: ls
if grep -q 'main.doo' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: TEST 9 — Process::run ls"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 9 — Process::run ls"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 10: pwd
if grep -q 'cwd = /' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: TEST 10 — Process::output pwd"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 10 — Process::output pwd"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 11: stderr output
if grep -q 'err_msg' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: TEST 11 — stderr output"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 11 — stderr output"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 12: Multiple spawn + shutdown
if grep -q 'after shutdown' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: TEST 12 — Multiple spawn + shutdown"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 12 — Multiple spawn + shutdown"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Check if all tests ran to completion
if grep -q 'ALL PROCESS TESTS COMPLETE' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: All tests ran to completion"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: Tests did not complete"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

rm -f "$OUTPUT_FILE"

TOTAL=$((TOTAL_PASS + TOTAL_FAIL))

echo ""
echo "==========================================="
echo "  Process Module Test Results"
echo "==========================================="
echo "  Total:  $TOTAL"
echo "  Passed: $TOTAL_PASS"
echo "  Failed: $TOTAL_FAIL"
echo "==========================================="

if [ "$TOTAL_FAIL" -gt 0 ]; then
    exit 1
fi

exit 0
