#!/bin/bash
# =============================================================================
# Doo Database Performance Audit Tests (WSL/Linux Only)
#
# Verifies ALL fixes from the database performance audit:
# - Pool configuration (bounded size, timeouts, recycling, FIFO)
# - Shared runtime (no per-query thread spawn)
# - Semaphore backpressure
# - Per-query timeouts
# - Row count safety limits
# - Unified DooResult type
# - Correct param type handling
# - Direct JSON serialization
# - Error propagation (no silent [])
# - Transaction support
# - PG error code mapping
# - Statement type detection
# - Missing free functions
#
# Usage:
#   bash test_db_performance_audit.sh          # Run with default DATABASE_URL
#   DATABASE_URL="postgres://..." bash test_db_performance_audit.sh
#
# Requirements:
#   - PostgreSQL running and accessible
#   - DATABASE_URL env var set (or default localhost)
#   - Built doo compiler (cargo build --release --workspace)
# =============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

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
echo -e "${BOLD}  Doo Database Performance Audit Tests${NC}"
echo -e "${BOLD}================================================================${NC}"
echo ""

# ---------------------------------------------------------------------------
# 1. Check DATABASE_URL
# ---------------------------------------------------------------------------
if [ -z "${DATABASE_URL:-}" ]; then
    # Try to load from .env files
    for env_file in "$SCRIPT_DIR/.env" "$SCRIPT_DIR/../../../.env" "$PROJECT_ROOT/.env"; do
        if [ -f "$env_file" ]; then
            db_line=$(grep -E '^DATABASE_URL=' "$env_file" | head -n 1 || true)
            if [ -n "$db_line" ]; then
                export DATABASE_URL="${db_line#DATABASE_URL=}"
                DATABASE_URL="${DATABASE_URL%\"}"
                DATABASE_URL="${DATABASE_URL#\"}"
                echo -e "  ${DIM}Loaded DATABASE_URL from $env_file${NC}"
                break
            fi
        fi
    done
fi

if [ -z "${DATABASE_URL:-}" ]; then
    # Default to localhost
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
    # Extract host and port from DATABASE_URL
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

# ---------------------------------------------------------------------------
# 3. Ensure test database exists
# ---------------------------------------------------------------------------
DB_NAME=$(echo "$DATABASE_URL" | sed -E 's|.*/([^?]+).*|\1|')
if command -v psql &>/dev/null; then
    DB_USER=$(echo "$DATABASE_URL" | sed -E 's|.*://([^:@]+).*|\1|')
    DB_HOST=$(echo "$DATABASE_URL" | sed -E 's|.*@([^:/]+).*|\1|')
    DB_PORT=$(echo "$DATABASE_URL" | sed -E 's|.*:([0-9]+)/.*|\1|')

    # Try to create the database if it doesn't exist
    psql -h "${DB_HOST:-localhost}" -p "${DB_PORT:-5432}" -U "${DB_USER:-postgres}" -tc \
        "SELECT 1 FROM pg_database WHERE datname = '${DB_NAME}'" 2>/dev/null | grep -q 1 || \
        psql -h "${DB_HOST:-localhost}" -p "${DB_PORT:-5432}" -U "${DB_USER:-postgres}" -c \
        "CREATE DATABASE ${DB_NAME}" 2>/dev/null || true
    echo -e "  ${DIM}Database '${DB_NAME}' ready${NC}"
fi

echo ""

# ---------------------------------------------------------------------------
# 4. Verify doo binary exists
# ---------------------------------------------------------------------------
if [ ! -f "$BIN" ]; then
    echo -e "  ${RED}ERROR: doo binary not found at: $BIN${NC}"
    echo -e "  ${DIM}Run: cargo build --release --workspace${NC}"
    exit 1
fi
echo -e "  ${DIM}Using binary: $BIN${NC}"

# ---------------------------------------------------------------------------
# 5. Run the test
# ---------------------------------------------------------------------------
TEST_FILE="$SCRIPT_DIR/test_db_performance_audit.doo"

if [ ! -f "$TEST_FILE" ]; then
    echo -e "  ${RED}ERROR: Test file not found: $TEST_FILE${NC}"
    exit 1
fi

echo ""
echo -e "${BOLD}--- Running Database Tests ---${NC}"
echo ""

PASS_COUNT=0
FAIL_COUNT=0
TOTAL_COUNT=0

# Run the test with a timeout
OUTPUT=$(timeout 120s "$BIN" run "$TEST_FILE" 2>&1 | tr -d '\r') || true
EXIT_CODE=${PIPESTATUS[0]:-$?}

if [ "$EXIT_CODE" -eq 124 ]; then
    echo -e "  ${RED}TIMEOUT: Test exceeded 120 seconds${NC}"
    FAIL_COUNT=1
elif [ "$EXIT_CODE" -ne 0 ]; then
    echo -e "  ${RED}CRASH: Exit code $EXIT_CODE${NC}"
    echo "$OUTPUT" | head -30
    FAIL_COUNT=1
fi

# Count PASS/FAIL lines
while IFS= read -r line; do
    if [[ "$line" == *"PASS:"* ]]; then
        PASS_COUNT=$((PASS_COUNT + 1))
        TOTAL_COUNT=$((TOTAL_COUNT + 1))
        echo -e "  ${GREEN}$line${NC}"
    elif [[ "$line" == *"FAIL:"* ]] || [[ "$line" == *"ERROR:"* ]]; then
        FAIL_COUNT=$((FAIL_COUNT + 1))
        TOTAL_COUNT=$((TOTAL_COUNT + 1))
        echo -e "  ${RED}$line${NC}"
    elif [[ "$line" == *"=== TEST"* ]] || [[ "$line" == *"=== ALL"* ]] || [[ "$line" == *"=== CLEANUP"* ]]; then
        echo -e "  ${CYAN}$line${NC}"
    elif [[ "$line" == *"Error"* ]] || [[ "$line" == *"error"* ]] || [[ "$line" == *"panic"* ]]; then
        echo -e "  ${RED}$line${NC}"
    fi
done <<< "$OUTPUT"

# ---------------------------------------------------------------------------
# 6. Summary
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}================================================================${NC}"
echo -e "${BOLD}  DATABASE TEST SUMMARY${NC}"
echo -e "${BOLD}================================================================${NC}"
echo ""
echo -e "  Total:   ${BOLD}$TOTAL_COUNT${NC}"
echo -e "  Passed:  ${GREEN}$PASS_COUNT${NC}"
if [ "$FAIL_COUNT" -gt 0 ]; then
    echo -e "  Failed:  ${RED}$FAIL_COUNT${NC}"
else
    echo -e "  Failed:  $FAIL_COUNT"
fi
echo ""
echo -e "${BOLD}================================================================${NC}"

if [ "$FAIL_COUNT" -eq 0 ] && [ "$PASS_COUNT" -gt 0 ]; then
    echo -e "  ${GREEN}All database performance audit tests passed!${NC}"
    echo ""
    exit 0
else
    echo -e "  ${RED}Some tests failed${NC}"
    echo ""
    echo -e "  ${DIM}Full output:${NC}"
    echo "$OUTPUT"
    exit 1
fi
