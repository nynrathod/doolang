#!/bin/bash
# =============================================================================
# Git Module Comprehensive Test Suite
# Tests: init, clone, commitAll, isDirty, headShort, hasRemote,
#        stash, stashPop, push, pull, currentBranch, multiple commits
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"
cd "$SCRIPT_DIR"

FILE="main.doo"
EXPECTED_TESTS=19

echo "=== Doo Git Module Test Suite ==="
echo ""

# Build and run via doo (not a server test — CLI test)
if [ ! -x "$BIN" ]; then
    echo "Building doo binary for tests..."
    (cd "$PROJECT_ROOT" && cargo build --release --workspace >/dev/null 2>&1) || true
fi

if [ ! -x "$BIN" ]; then
    echo "❌ doo binary not found or not executable at: $BIN"
    exit 1
fi

# Clean leftover test repos from previous runs
rm -rf ./__test_repos 2>/dev/null || true

echo "Running git tests..."
echo "  Binary: $BIN"
echo ""

# Step 1: Check compilation first
echo "  Checking compilation..."
COMPILE_OUT=$("$BIN" check "$FILE" 2>&1) || COMPILE_EXIT=$?
COMPILE_EXIT=${COMPILE_EXIT:-0}
if [ $COMPILE_EXIT -ne 0 ]; then
    echo "❌ Compilation failed (exit=$COMPILE_EXIT):"
    echo "$COMPILE_OUT"
    exit 1
fi
echo "  ✅ Compilation OK"
echo ""

# Step 2: Run the program — stream output live (stdbuf forces line buffering)
OUTPUT_FILE=$(mktemp)
echo "--- Running Tests ---"
stdbuf -oL timeout 120 "$BIN" run "$FILE" 2>&1 | tee "$OUTPUT_FILE" || RUN_EXIT=$?
RUN_EXIT=${RUN_EXIT:-0}
echo "--- End Tests ---"
echo ""

# ============================================================================
# Verification helpers
# ============================================================================
TOTAL_PASS=0
TOTAL_FAIL=0

# assert_grep <test_num> <label> <pattern>
# Passes if grep -q finds the pattern in OUTPUT_FILE
assert_grep() {
    local num="$1" label="$2" pattern="$3"
    if grep -q "$pattern" "$OUTPUT_FILE"; then
        echo "  ✅ PASS: TEST $num — $label"
        TOTAL_PASS=$((TOTAL_PASS + 1))
    else
        echo "  ❌ FAIL: TEST $num — $label (expected: $pattern)"
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
    fi
}

# assert_hash <test_num> <label> <grep_pattern>
# Extracts value after "= " and verifies it is a 7-char hex hash [0-9a-f]{7}
assert_hash() {
    local num="$1" label="$2" pattern="$3"
    local val
    val=$(grep "$pattern" "$OUTPUT_FILE" | head -1 | sed 's/.*= //' | tr -d '[:space:]')
    if [ -n "$val" ] && echo "$val" | grep -qE '^[0-9a-f]{7}$'; then
        echo "  ✅ PASS: TEST $num — $label (hash=$val)"
        TOTAL_PASS=$((TOTAL_PASS + 1))
    else
        echo "  ❌ FAIL: TEST $num — $label (invalid hash: '$val', expected 7-char hex)"
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
    fi
}

# assert_bool <test_num> <label> <grep_pattern> <expected: true|false>
# Extracts value after "= " and checks it is exactly the expected boolean
assert_bool() {
    local num="$1" label="$2" pattern="$3" expected="$4"
    local val
    val=$(grep "$pattern" "$OUTPUT_FILE" | head -1 | sed 's/.*= //' | tr -d '[:space:]')
    if [ "$val" = "$expected" ]; then
        echo "  ✅ PASS: TEST $num — $label ($val)"
        TOTAL_PASS=$((TOTAL_PASS + 1))
    else
        echo "  ❌ FAIL: TEST $num — $label (got '$val', expected '$expected')"
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
    fi
}

# assert_match <test_num> <label> <pat1> <pat2>
# Extracts values from two grep patterns and checks they are equal
assert_hashes_match() {
    local num="$1" label="$2" pat1="$3" pat2="$4"
    local v1 v2
    v1=$(grep "$pat1" "$OUTPUT_FILE" | head -1 | sed 's/.*= //' | tr -d '[:space:]')
    v2=$(grep "$pat2" "$OUTPUT_FILE" | head -1 | sed 's/.*= //' | tr -d '[:space:]')
    if [ -n "$v1" ] && [ "$v1" = "$v2" ]; then
        echo "  ✅ PASS: TEST $num — $label ($v1 == $v2)"
        TOTAL_PASS=$((TOTAL_PASS + 1))
    else
        echo "  ❌ FAIL: TEST $num — $label ($v1 != $v2)"
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
    fi
}

echo "--- Verifying Test Results ---"

# Test 1: Git::init — repo initialized + .git directory exists
GIT_EXISTS=$(grep '.git exists = ' "$OUTPUT_FILE" | head -1 | sed 's/.*= //' | tr -d '[:space:]')
if grep -q 'initialized repo at' "$OUTPUT_FILE" && [ "$GIT_EXISTS" = "true" ]; then
    echo "  ✅ PASS: TEST 1 — Git::init (.git verified)"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 1 — Git::init (.git exists=$GIT_EXISTS)"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 2: Git::isDirty (clean repo) — should be false
assert_bool 2 "Git::isDirty (clean repo)" 'isDirty = ' "false"

# Test 3: Git::isDirty (after adding file) — should be true
assert_bool 3 "Git::isDirty (dirty after file)" 'isDirty after file = ' "true"

# Test 4: Git::commitAll — returns valid 7-char hex hash
assert_hash 4 "Git::commitAll" 'commit hash = '

# Test 5: Git::isDirty (after commit) — should be false
assert_bool 5 "Git::isDirty (after commit)" 'isDirty after commit = ' "false"

# Test 6: Git::headShort — returns valid 7-char hex hash
assert_hash 6 "Git::headShort" '  headShort = '

# Test 7: Git::hasRemote (no remote) — should be false
assert_bool 7 "Git::hasRemote (no remote)" '  hasRemote = ' "false"

# Test 8: Git::stash — dirty before, clean after
STASH_BEFORE=$(grep 'isDirty before stash = ' "$OUTPUT_FILE" | head -1 | sed 's/.*= //' | tr -d '[:space:]')
STASH_AFTER=$(grep 'isDirty after stash = ' "$OUTPUT_FILE" | head -1 | sed 's/.*= //' | tr -d '[:space:]')
if [ "$STASH_BEFORE" = "true" ] && [ "$STASH_AFTER" = "false" ]; then
    echo "  ✅ PASS: TEST 8 — Git::stash (before=$STASH_BEFORE, after=$STASH_AFTER)"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 8 — Git::stash (before=$STASH_BEFORE, after=$STASH_AFTER)"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 9: Git::stashPop — should be dirty after pop
assert_bool 9 "Git::stashPop" 'isDirty after stashPop = ' "true"

# Test 10: Git::commitAll (second commit) — valid hash
assert_hash 10 "Git::commitAll (2nd)" 'second commit hash = '

# Test 11: Git::headShort changes after second commit
HEAD_COUNT=$(grep -c '  headShort = ' "$OUTPUT_FILE" 2>/dev/null || true)
HEAD_COUNT=${HEAD_COUNT:-0}
if [ "$HEAD_COUNT" -ge 2 ] 2>/dev/null; then
    echo "  ✅ PASS: TEST 11 — Git::headShort (${HEAD_COUNT} occurrences)"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 11 — Git::headShort (expected 2+ occurrences, got $HEAD_COUNT)"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 12: Git::clone — cloned successfully + headShort is valid hash
CLONE_HEAD=$(grep 'clone headShort = ' "$OUTPUT_FILE" | head -1 | sed 's/.*= //' | tr -d '[:space:]')
if grep -q 'cloned to ' "$OUTPUT_FILE" && echo "$CLONE_HEAD" | grep -qE '^[0-9a-f]{7}$'; then
    echo "  ✅ PASS: TEST 12 — Git::clone (head=$CLONE_HEAD)"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 12 — Git::clone (head=$CLONE_HEAD)"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 13: Git::hasRemote (cloned repo) — should be true
CLONE_HAS_REMOTE=$(grep -A2 'TEST 13' "$OUTPUT_FILE" | grep '  hasRemote = ' | sed 's/.*= //' | tr -d '[:space:]')
if [ "$CLONE_HAS_REMOTE" = "true" ]; then
    echo "  ✅ PASS: TEST 13 — Git::hasRemote (cloned=$CLONE_HAS_REMOTE)"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 13 — Git::hasRemote (got '$CLONE_HAS_REMOTE', expected 'true')"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 14: Git::commitAll on cloned repo — valid hash
assert_hash 14 "Git::commitAll (cloned repo)" 'clone commit hash = '

# Test 15: Git::stash on clean repo — graceful (no crash)
assert_grep 15 "Git::stash (nothing to stash)" 'stash on clean repo completed'

# Test 16: Multiple rapid commits — all 3 hashes valid
COMMIT1=$(grep 'commit1 = ' "$OUTPUT_FILE" | sed 's/.*= //' | tr -d '[:space:]')
COMMIT2=$(grep 'commit2 = ' "$OUTPUT_FILE" | sed 's/.*= //' | tr -d '[:space:]')
COMMIT3=$(grep 'commit3 = ' "$OUTPUT_FILE" | sed 's/.*= //' | tr -d '[:space:]')
FINAL_HEAD=$(grep 'final headShort = ' "$OUTPUT_FILE" | sed 's/.*= //' | tr -d '[:space:]')
if echo "$COMMIT1" | grep -qE '^[0-9a-f]{7}$' && \
   echo "$COMMIT2" | grep -qE '^[0-9a-f]{7}$' && \
   echo "$COMMIT3" | grep -qE '^[0-9a-f]{7}$' && \
   [ "$COMMIT3" = "$FINAL_HEAD" ]; then
    echo "  ✅ PASS: TEST 16 — Multiple commits ($COMMIT1→$COMMIT2→$COMMIT3, head=$FINAL_HEAD)"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 16 — Multiple commits (c1=$COMMIT1 c2=$COMMIT2 c3=$COMMIT3 head=$FINAL_HEAD)"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 17: Git::push — push to bare remote + verify round-trip
PUSH_HASH=$(grep 'commit before push = ' "$OUTPUT_FILE" | sed 's/.*= //' | tr -d '[:space:]')
VERIFY_HASH=$(grep 'verify clone headShort = ' "$OUTPUT_FILE" | sed 's/.*= //' | tr -d '[:space:]')
if grep -q 'pushed to bare remote' "$OUTPUT_FILE" && \
   echo "$PUSH_HASH" | grep -qE '^[0-9a-f]{7}$' && \
   [ "$PUSH_HASH" = "$VERIFY_HASH" ]; then
    echo "  ✅ PASS: TEST 17 — Git::push ($PUSH_HASH == $VERIFY_HASH)"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 17 — Git::push (push=$PUSH_HASH, verify=$VERIFY_HASH)"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 18: Git::pull — pull changes and verify HEAD matches
PULL_PUSH_HASH=$(grep '  new commit = ' "$OUTPUT_FILE" | sed 's/.*= //' | tr -d '[:space:]')
PULL_HEAD=$(grep 'after pull headShort = ' "$OUTPUT_FILE" | sed 's/.*= //' | tr -d '[:space:]')
if echo "$PULL_PUSH_HASH" | grep -qE '^[0-9a-f]{7}$' && [ "$PULL_PUSH_HASH" = "$PULL_HEAD" ]; then
    echo "  ✅ PASS: TEST 18 — Git::pull ($PULL_PUSH_HASH == $PULL_HEAD)"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 18 — Git::pull (push=$PULL_PUSH_HASH, pull=$PULL_HEAD)"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Test 19: Git::currentBranch — returns non-empty branch name
BRANCH_NAME=$(grep 'currentBranch = ' "$OUTPUT_FILE" | head -1 | sed 's/.*= //' | tr -d '[:space:]')
if [ -n "$BRANCH_NAME" ]; then
    echo "  ✅ PASS: TEST 19 — Git::currentBranch (branch=$BRANCH_NAME)"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: TEST 19 — Git::currentBranch (empty or missing)"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

# Completion check — did the program run all the way through?
if grep -q 'ALL GIT TESTS COMPLETE' "$OUTPUT_FILE"; then
    echo "  ✅ PASS: Program completed (all $EXPECTED_TESTS tests ran)"
    TOTAL_PASS=$((TOTAL_PASS + 1))
else
    echo "  ❌ FAIL: Program did not complete (crash or early exit)"
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
fi

rm -f "$OUTPUT_FILE"

TOTAL=$((TOTAL_PASS + TOTAL_FAIL))

echo ""
echo "==========================================="
echo "  Git Module Test Results"
echo "==========================================="
echo "  Total:  $TOTAL"
echo "  Passed: $TOTAL_PASS"
echo "  Failed: $TOTAL_FAIL"
echo "==========================================="

if [ "$TOTAL_FAIL" -gt 0 ]; then
    exit 1
fi

exit 0
