#!/bin/bash
# =============================================================================
# Doo Query Builder Integration Tests
#
# Tests all query builder features for a YC-level SaaS startup scenario:
#
# CORE (qb_core.doo):
#   toSql(), INSERT, find(), where() with all operators (Gt/Gte/Lt/Lte/Ne/
#   Like/ILike/Between/In/NotIn), findOne(), count(), orderBy(), limit(),
#   offset(), update().set(), delete(), table name derivation
#
# ADVANCED (qb_advanced.doo):
#   @table decorator, select(), distinct(), IsNull/IsNotNull, orWhere(),
#   whereNot(), whereNull(), whereNotNull(), whereBetween(), whereIn(),
#   complex chains, insertMany(), increment(), decrement(), returning(),
#   groupBy(), edge cases
#
# Usage:
#   bash test_query_builder.sh                       # Use .env or default
#   DATABASE_URL="postgres://..." bash test_query_builder.sh
#
# Requirements:
#   - PostgreSQL running and accessible
#   - DATABASE_URL env var set (or default localhost)
#   - Built doo compiler (cargo build --release --workspace)
# =============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../common.sh"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

echo ""
echo -e "${BOLD}================================================================${NC}"
echo -e "${BOLD}  Doo Query Builder Integration Tests${NC}"
echo -e "${BOLD}================================================================${NC}"
echo ""

# ---------------------------------------------------------------------------
# 1. Check DATABASE_URL
# ---------------------------------------------------------------------------
if [ -z "${DATABASE_URL:-}" ]; then
    for env_file in "$SCRIPT_DIR/.env" "$SCRIPT_DIR/../.env" "$SCRIPT_DIR/../../.env" "$PROJECT_ROOT/.env"; do
        if [ -f "$env_file" ]; then
            db_line=$(grep -E '^DATABASE_URL=' "$env_file" | head -n 1 || true)
            if [ -n "$db_line" ]; then
                export DATABASE_URL="${db_line#DATABASE_URL=}"
                DATABASE_URL="${DATABASE_URL%\"}"
                DATABASE_URL="${DATABASE_URL#\"}"
                DATABASE_URL="${DATABASE_URL%$'\r'}"
                echo -e "  ${DIM}Loaded DATABASE_URL from $env_file${NC}"
                break
            fi
        fi
    done
fi

if [ -z "${DATABASE_URL:-}" ]; then
    export DATABASE_URL="postgresql://postgres:postgres@localhost:5432/doo_test"
    echo -e "  ${YELLOW}WARNING: DATABASE_URL not set, using default: $DATABASE_URL${NC}"
    echo -e "  ${DIM}Set DATABASE_URL or create .env file to customize${NC}"
fi

echo -e "  ${DIM}DATABASE_URL: ${DATABASE_URL%%@*}@***${NC}"
echo ""

# ---------------------------------------------------------------------------
# 2. Verify PostgreSQL is reachable
# ---------------------------------------------------------------------------
echo -e "  ${DIM}Checking PostgreSQL connectivity...${NC}"
if command -v pg_isready &>/dev/null; then
    DB_HOST=$(echo "$DATABASE_URL" | sed -E 's|.*@([^:/]+).*|\1|')
    DB_PORT=$(echo "$DATABASE_URL" | sed -E 's|.*:([0-9]+)/.*|\1|')
    if pg_isready -h "${DB_HOST:-localhost}" -p "${DB_PORT:-5432}" &>/dev/null; then
        echo -e "  ${GREEN}PostgreSQL is running${NC}"
    else
        echo -e "  ${RED}ERROR: PostgreSQL not reachable at ${DB_HOST:-localhost}:${DB_PORT:-5432}${NC}"
        echo -e "  ${DIM}Start PostgreSQL: sudo service postgresql start${NC}"
        exit 1
    fi
else
    echo -e "  ${YELLOW}pg_isready not found, skipping connectivity check${NC}"
fi

echo ""

# ---------------------------------------------------------------------------
# 4. Verify doo binary exists
# ---------------------------------------------------------------------------
RUNNER_BIN=""
if command -v doo >/dev/null 2>&1; then
    RUNNER_BIN="$(command -v doo)"
elif [ -f "$BIN" ]; then
    RUNNER_BIN="$BIN"
fi

if [ -z "$RUNNER_BIN" ]; then
    echo -e "  ${RED}ERROR: doo binary not found in PATH and fallback missing at: $BIN${NC}"
    echo -e "  ${DIM}Run: cargo build --release --workspace${NC}"
    exit 1
fi
echo -e "  ${DIM}Using binary: $RUNNER_BIN${NC}"

# ---------------------------------------------------------------------------
# 5. DDL helpers — create/drop tables via psql (DDL is outside QB scope)
# ---------------------------------------------------------------------------
psql_exec() {
    psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c "SET client_min_messages TO warning; $1" >/dev/null
}

create_core_tables() {
    echo -e "  ${DIM}Setting up core tables (tasks, projects)...${NC}"
    psql_exec "DROP TABLE IF EXISTS tasks CASCADE;"
    psql_exec "DROP TABLE IF EXISTS projects CASCADE;"
    psql_exec "CREATE TABLE tasks (id SERIAL PRIMARY KEY, title TEXT NOT NULL, status TEXT DEFAULT 'todo', priority INT DEFAULT 1, user_id INT DEFAULT 0, description TEXT DEFAULT '');"
    psql_exec "CREATE TABLE projects (id SERIAL PRIMARY KEY, name TEXT NOT NULL, status TEXT DEFAULT 'active', owner_id INT DEFAULT 0);"
    echo -e "  ${GREEN}Core tables ensured${NC}"
}

drop_core_tables() {
    psql_exec "DROP TABLE IF EXISTS tasks CASCADE;"
    psql_exec "DROP TABLE IF EXISTS projects CASCADE;"
}

create_advanced_tables() {
    echo -e "  ${DIM}Setting up advanced tables (blog_posts, metrics)...${NC}"
    psql_exec "DROP TABLE IF EXISTS blog_posts CASCADE;"
    psql_exec "DROP TABLE IF EXISTS metrics CASCADE;"
    psql_exec "CREATE TABLE blog_posts (id SERIAL PRIMARY KEY, title TEXT NOT NULL, category TEXT DEFAULT 'general', view_count INT DEFAULT 0, like_count INT DEFAULT 0, author_id INT DEFAULT 0, published BOOLEAN DEFAULT false, summary TEXT DEFAULT '');"
    psql_exec "CREATE TABLE metrics (id SERIAL PRIMARY KEY, name TEXT NOT NULL, value INT DEFAULT 0, category TEXT DEFAULT 'general', user_id INT DEFAULT 0, recorded_at TEXT DEFAULT '');"
    echo -e "  ${GREEN}Advanced tables ensured${NC}"
}

drop_advanced_tables() {
    psql_exec "DROP TABLE IF EXISTS blog_posts CASCADE;"
    psql_exec "DROP TABLE IF EXISTS metrics CASCADE;"
}

create_matrix_tables() {
    echo -e "  ${DIM}Setting up matrix table (matrix_items)...${NC}"
    psql_exec "DROP TABLE IF EXISTS matrix_items CASCADE;"
    psql_exec "CREATE TABLE matrix_items (id SERIAL PRIMARY KEY, name TEXT NOT NULL, score INT NOT NULL, ratio DOUBLE PRECISION NOT NULL, active BOOLEAN NOT NULL, tag TEXT NOT NULL);"
    echo -e "  ${GREEN}Matrix table ensured${NC}"
}

drop_matrix_tables() {
    psql_exec "DROP TABLE IF EXISTS matrix_items CASCADE;"
}

create_edge_tables() {
    echo -e "  ${DIM}Setting up edge table (edge_items)...${NC}"
    psql_exec "DROP TABLE IF EXISTS edge_items CASCADE;"
    psql_exec "CREATE TABLE edge_items (id SERIAL PRIMARY KEY, name TEXT NOT NULL, score INT NOT NULL, active BOOLEAN NOT NULL, category TEXT NOT NULL);"
    echo -e "  ${GREEN}Edge table ensured${NC}"
}

drop_edge_tables() {
    psql_exec "DROP TABLE IF EXISTS edge_items CASCADE;"
}

# ---------------------------------------------------------------------------
# 6. run_suite <name> <file>
# ---------------------------------------------------------------------------
TOTAL_PASS=0
TOTAL_FAIL=0
TOTAL_COUNT=0
SUITES_FAILED=0

run_suite() {
    local SUITE_NAME="$1"
    local TEST_FILE="$2"

    echo ""
    echo -e "${BOLD}--- $SUITE_NAME ---${NC}"
    echo ""

    if [ ! -f "$TEST_FILE" ]; then
        echo -e "  ${RED}ERROR: Test file not found: $TEST_FILE${NC}"
        SUITES_FAILED=$((SUITES_FAILED + 1))
        return
    fi

    local PASS_COUNT=0
    local FAIL_COUNT=0

    local RAW_OUTPUT
    local OUTPUT
    local EXIT_CODE
    local attempt=1
    local max_attempts=2
    while true; do
        set +e
        RAW_OUTPUT=$(timeout 120s "$RUNNER_BIN" run "$TEST_FILE" 2>&1)
        EXIT_CODE=$?
        set -e
        OUTPUT=$(printf '%s' "$RAW_OUTPUT" | tr -d '\r')

        if [ "$EXIT_CODE" -eq 0 ]; then
            break
        fi

        if [[ "$OUTPUT" == *"LNK1104"*"temp_doo_"*".exe"* ]] && [ "$attempt" -lt "$max_attempts" ]; then
            echo -e "  ${YELLOW}WARN: transient linker lock detected, retrying suite...${NC}"
            attempt=$((attempt + 1))
            continue
        fi

        break
    done

    if [ "$EXIT_CODE" -eq 124 ]; then
        echo -e "  ${RED}TIMEOUT: Test exceeded 120 seconds${NC}"
        SUITES_FAILED=$((SUITES_FAILED + 1))
        return
    elif [ "$EXIT_CODE" -ne 0 ]; then
        echo -e "  ${RED}CRASH: Exit code $EXIT_CODE${NC}"
        echo "$OUTPUT" | head -30
        SUITES_FAILED=$((SUITES_FAILED + 1))
        return
    fi

    while IFS= read -r line; do
        if [[ "$line" == *"PASS:"* ]]; then
            PASS_COUNT=$((PASS_COUNT + 1))
            echo -e "  ${GREEN}$line${NC}"
        elif [[ "$line" == *"FAIL:"* ]] || [[ "$line" == *"ERROR:"* ]]; then
            FAIL_COUNT=$((FAIL_COUNT + 1))
            echo -e "  ${RED}$line${NC}"
        elif [[ "$line" == *"=== TEST"* ]] || [[ "$line" == *"=== QB"* ]] || [[ "$line" == *"=== SEED"* ]] || [[ "$line" == *"=== ALL"* ]]; then
            echo -e "  ${CYAN}$line${NC}"
        elif [[ "$line" == *"Error"* ]] || [[ "$line" == *"error"* ]] || [[ "$line" == *"panic"* ]]; then
            echo -e "  ${RED}$line${NC}"
        fi
    done <<< "$OUTPUT"

    TOTAL_PASS=$((TOTAL_PASS + PASS_COUNT))
    TOTAL_FAIL=$((TOTAL_FAIL + FAIL_COUNT))
    TOTAL_COUNT=$((TOTAL_COUNT + PASS_COUNT + FAIL_COUNT))

    if [ "$FAIL_COUNT" -gt 0 ]; then
        SUITES_FAILED=$((SUITES_FAILED + 1))
    fi
}

run_compile_fail_suite() {
    local SUITE_NAME="$1"
    local TEST_FILE="$2"
    local EXPECTED_TOKEN="$3"
    local EXPECTED_MSG="${4:-}"

    echo ""
    echo -e "${BOLD}--- $SUITE_NAME ---${NC}"
    echo ""

    if [ ! -f "$TEST_FILE" ]; then
        echo -e "  ${RED}ERROR: Test file not found: $TEST_FILE${NC}"
        SUITES_FAILED=$((SUITES_FAILED + 1))
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
        TOTAL_COUNT=$((TOTAL_COUNT + 1))
        return
    fi

    local RAW_OUTPUT
    local OUTPUT
    local EXIT_CODE
    set +e
    RAW_OUTPUT=$(timeout 120s "$RUNNER_BIN" run "$TEST_FILE" 2>&1)
    EXIT_CODE=$?
    set -e
    OUTPUT=$(printf '%s' "$RAW_OUTPUT" | tr -d '\r')

    if [ "$EXIT_CODE" -eq 124 ]; then
        echo -e "  ${RED}TIMEOUT: Test exceeded 120 seconds${NC}"
        SUITES_FAILED=$((SUITES_FAILED + 1))
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
        TOTAL_COUNT=$((TOTAL_COUNT + 1))
        return
    fi

    if [ "$EXIT_CODE" -eq 0 ]; then
        echo -e "  ${RED}FAIL: expected compile failure but got success${NC}"
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
        TOTAL_COUNT=$((TOTAL_COUNT + 1))
        SUITES_FAILED=$((SUITES_FAILED + 1))
        return
    fi

    if ! echo "$OUTPUT" | grep -q "$EXPECTED_TOKEN"; then
        echo -e "  ${RED}FAIL: compile failed, but missing expected token '$EXPECTED_TOKEN'${NC}"
        echo "$OUTPUT" | head -30
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
        TOTAL_COUNT=$((TOTAL_COUNT + 1))
        SUITES_FAILED=$((SUITES_FAILED + 1))
        return
    fi

    if [ -n "$EXPECTED_MSG" ] && ! echo "$OUTPUT" | grep -q "$EXPECTED_MSG"; then
        echo -e "  ${RED}FAIL: compile failed, but missing expected message '$EXPECTED_MSG'${NC}"
        echo "$OUTPUT" | head -30
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
        TOTAL_COUNT=$((TOTAL_COUNT + 1))
        SUITES_FAILED=$((SUITES_FAILED + 1))
        return
    fi

    echo -e "  ${GREEN}PASS: compile failed with expected token '$EXPECTED_TOKEN'${NC}"
    if [ -n "$EXPECTED_MSG" ]; then
        echo -e "  ${GREEN}PASS: compile failed with expected message '$EXPECTED_MSG'${NC}"
    fi
    TOTAL_PASS=$((TOTAL_PASS + 1))
    TOTAL_COUNT=$((TOTAL_COUNT + 1))
}

# ---------------------------------------------------------------------------
# 7. Run Core QB Tests
# ---------------------------------------------------------------------------
create_core_tables
run_suite "QB Core — SELECT/COUNT/INSERT/UPDATE/DELETE + Operators" \
    "$SCRIPT_DIR/qb_core.doo"
drop_core_tables

# ---------------------------------------------------------------------------
# 8. Run Advanced QB Tests
# ---------------------------------------------------------------------------
create_advanced_tables
run_suite "QB Advanced — Distinct/Aggregate/Increment/Bulk/Chain" \
    "$SCRIPT_DIR/qb_advanced.doo"
drop_advanced_tables

# ---------------------------------------------------------------------------
# 9. Run Matrix QB Tests (Cross-Type Combination Coverage)
# ---------------------------------------------------------------------------
create_matrix_tables
run_suite "QB Matrix — Cross-Type/Operator/Chain Combinations" \
    "$SCRIPT_DIR/qb_matrix.doo"
drop_matrix_tables

# ---------------------------------------------------------------------------
# 9b. Run Edge Case QB Tests (boundaries, empty results, chain order)
# ---------------------------------------------------------------------------
create_edge_tables
run_suite "QB Edge — Empty Results/Boundaries/Chain Order/Special Chars" \
    "$SCRIPT_DIR/qb_edge.doo"
drop_edge_tables

# ---------------------------------------------------------------------------
# 10. Run Compile-Fail QB Tests (3 files, one per error code)
# ---------------------------------------------------------------------------
run_compile_fail_suite "QB Compile-Fail — Unknown Field (E0706)" \
    "$SCRIPT_DIR/qb_compile_fail_E0706.doo" \
    "E0706" \
    "unknown field"

run_compile_fail_suite "QB Compile-Fail — Unsafe Mutation (E0707)" \
    "$SCRIPT_DIR/qb_compile_fail_E0707.doo" \
    "E0707" \
    "requires at least one filter clause"

run_compile_fail_suite "QB Compile-Fail — Type/Operator Mismatch (E0708)" \
    "$SCRIPT_DIR/qb_compile_fail_E0708.doo" \
    "E0708" \
    "Between requires numeric field"

# ---------------------------------------------------------------------------
# 11. Summary
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}================================================================${NC}"
echo -e "${BOLD}  QUERY BUILDER TEST SUMMARY${NC}"
echo -e "${BOLD}================================================================${NC}"
echo ""
echo -e "  Total:   ${BOLD}$TOTAL_COUNT${NC}"
echo -e "  Passed:  ${GREEN}$TOTAL_PASS${NC}"
if [ "$TOTAL_FAIL" -gt 0 ]; then
    echo -e "  Failed:  ${RED}$TOTAL_FAIL${NC}"
else
    echo -e "  Failed:  $TOTAL_FAIL"
fi
echo ""
echo -e "${BOLD}================================================================${NC}"

if [ "$TOTAL_FAIL" -eq 0 ] && [ "$SUITES_FAILED" -eq 0 ] && [ "$TOTAL_PASS" -gt 0 ]; then
    echo -e "  ${GREEN}All query builder tests passed!${NC}"
    echo ""
    exit 0
else
    echo -e "  ${RED}Some tests failed${NC}"
    echo ""
    exit 1
fi
