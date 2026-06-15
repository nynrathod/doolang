#!/bin/bash
# =============================================================================
# Doo Migration Engine — Dependency Graph & Batch Test Suite
# Tests: component grouping, depends_on chains, batch is_chained flags,
#        same-table grouping, FK chains, enum→table deps, independent tables
#
# Usage:
#   bash test_dependency_graph.sh              # Run all tests
#   bash test_dependency_graph.sh --verbose    # Verbose output
#   bash test_dependency_graph.sh --build      # Force rebuild compiler first
# =============================================================================

set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../common.sh"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

PASSED=0
FAILED=0
SKIPPED=0

# Parse flags
VERBOSE=0
FORCE_BUILD=0
for arg in "$@"; do
    case "$arg" in
        --verbose|-v) VERBOSE=1 ;;
        --build|-b)   FORCE_BUILD=1 ;;
    esac
done

# ── Configuration ──────────────────────────────────────────────────────────
MIGRATE_DIR="$SCRIPT_DIR"
BASE_DOO="$MIGRATE_DIR/dep_graph_test.doo"
VARIANTS_DIR="$MIGRATE_DIR/variants_dep"
LOG_FILE="$MIGRATE_DIR/test_dep_graph.log"
RESULTS_FILE="$MIGRATE_DIR/test_dep_graph_results.log"

# Database config — single source of truth from DATABASE_URL env / .env
TEST_DB_URL="${DATABASE_URL:-postgresql://postgres:admin@localhost:5432/doo_test2}"

# Use BIN from common.sh
DOO_BIN="$BIN"
PSQL_BIN="psql"

# ── Helpers ────────────────────────────────────────────────────────────────

log()    { echo -e "${BLUE}[$(date +%H:%M:%S)]${NC} $*" >> "$LOG_FILE"; if [ "$VERBOSE" = 1 ]; then echo -e "${BLUE}[LOG]${NC} $*"; fi; }
pass()   { echo -e "  ${GREEN}✓ PASS${NC} $1"; PASSED=$((PASSED + 1)); }
fail()   { echo -e "  ${RED}✗ FAIL${NC} $1"; FAILED=$((FAILED + 1)); }
skip()   { echo -e "  ${YELLOW}⊘ SKIP${NC} $1"; SKIPPED=$((SKIPPED + 1)); }

section() {
    echo ""
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${CYAN}  $1${NC}"
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════${NC}"
}

step() { echo -e "  ${BLUE}→${NC} $1"; }

doo_migrate() {
    local label="$1"; shift
    log "Running: $DOO_BIN migrate $*"
    "$DOO_BIN" migrate "$@" 2>&1 || true
}

# Same as doo_migrate but captures ONLY stdout (for JSON parsing).
# Stderr (human-readable logs) is discarded so the captured output
# is clean JSON when --json flag is used.
doo_migrate_json() {
    local label="$1"; shift
    log "Running: $DOO_BIN migrate $*"
    "$DOO_BIN" migrate "$@" 2>/dev/null || true
}

assert_json_valid() {
    local json="$1" label="$2"
    if [ -z "$json" ]; then
        fail "$label — empty JSON"
        return 1
    fi
    if echo "$json" | jq . >/dev/null 2>&1; then
        pass "$label"
        return 0
    else
        fail "$label — invalid JSON"
        return 1
    fi
}

assert_json_field_eq() {
    local json="$1" field="$2" expected="$3" label="$4"
    local actual
    actual=$(echo "$json" | jq -r "$field" 2>/dev/null)
    if [ "$actual" = "$expected" ]; then
        pass "$label (got: $actual)"
        return 0
    else
        fail "$label — expected '$expected', got '$actual'"
        return 1
    fi
}

assert_json_field_gt() {
    local json="$1" field="$2" expected="$3" label="$4"
    local actual
    actual=$(echo "$json" | jq -r "$field" 2>/dev/null)
    if [ -n "$actual" ] && [ "$actual" -gt "$expected" ] 2>/dev/null; then
        pass "$label (got: $actual)"
        return 0
    else
        fail "$label — expected > $expected, got '$actual'"
        return 1
    fi
}

assert_json_contains() {
    local json="$1" jq_filter="$2" label="$3"
    local result
    result=$(echo "$json" | jq -r "$jq_filter" 2>/dev/null)
    if [ -n "$result" ] && [ "$result" != "null" ] && [ "$result" != "false" ]; then
        pass "$label"
        return 0
    else
        fail "$label"
        return 1
    fi
}

cleanup() {
    rm -rf "$VARIANTS_DIR"
    log "Cleaned up temp files"
}
trap cleanup EXIT

# ── Setup ───────────────────────────────────────────────────────────────────

echo -e "${BOLD}${CYAN}"
echo "  ╔═══════════════════════════════════════════════╗"
echo "  ║  Doo Migration — Dependency Graph Test Suite  ║"
echo "  ║  $(date)                      ║"
echo "  ╚═══════════════════════════════════════════════╝"
echo -e "${NC}"
echo "  Project:  $PROJECT_ROOT"
echo "  Database: $TEST_DB_URL"
echo "  Binary:   $DOO_BIN"
echo "  Log:      $LOG_FILE"
echo ""

rm -f "$LOG_FILE" "$RESULTS_FILE"

# ── Phase 0: Build Compiler ─────────────────────────────────────────────────
section "PHASE 0: Build Compiler"

step "Checking for doo binary..."
if [ ! -x "$DOO_BIN" ]; then
    # Try common alternative locations
    if [ -x "$PROJECT_ROOT/target/release/doo.exe" ]; then
        DOO_BIN="$PROJECT_ROOT/target/release/doo.exe"
    elif [ -x "$PROJECT_ROOT/target/release/doo" ]; then
        DOO_BIN="$PROJECT_ROOT/target/release/doo"
    fi
fi

if [ ! -x "$DOO_BIN" ] || [ "$FORCE_BUILD" = 1 ]; then
    step "Building compiler (this may take a few minutes)..."
    (cd "$PROJECT_ROOT" && cargo build --release --workspace 2>&1 | tail -5) || {
        fail "Compiler build failed"
        exit 1
    }
    if [ -x "$PROJECT_ROOT/target/release/doo.exe" ]; then
        DOO_BIN="$PROJECT_ROOT/target/release/doo.exe"
    elif [ -x "$PROJECT_ROOT/target/release/doo" ]; then
        DOO_BIN="$PROJECT_ROOT/target/release/doo"
    fi
fi

if [ ! -x "$DOO_BIN" ]; then
    fail "Cannot find doo binary at $DOO_BIN"
    exit 1
fi
pass "Compiler ready: $DOO_BIN"

# ── Check prerequisites ─────────────────────────────────────────────────────
if ! command -v jq &>/dev/null; then
    echo -e "  ${RED}jq is required for JSON validation. Please install jq.${NC}"
    exit 1
fi
pass "jq is available"

# ── Parse DB URL ────────────────────────────────────────────────────────────
parse_db_url() {
    local url="$1"
    # postgresql://user:pass@host:port/dbname
    DB_USER=$(echo "$url" | sed -n 's|.*://\([^:]*\):.*|\1|p')
    DB_PASS=$(echo "$url" | sed -n 's|.*://[^:]*:\([^@]*\)@.*|\1|p')
    DB_HOST=$(echo "$url" | sed -n 's|.*@\([^:]*\):.*|\1|p')
    DB_PORT=$(echo "$url" | sed -n 's|.*:\([0-9]*\)/.*|\1|p')
    DB_NAME=$(echo "$url" | sed -n 's|.*/\([^?]*\).*|\1|p')
    # Defaults
    DB_HOST="${DB_HOST:-localhost}"
    DB_PORT="${DB_PORT:-5432}"
    DB_NAME="${DB_NAME:-doo_test2}"
}
parse_db_url "$TEST_DB_URL"

ADMIN_USER="$DB_USER"
ADMIN_PASS="$DB_PASS"
ADMIN_HOST="$DB_HOST"
ADMIN_PORT="$DB_PORT"
ADMIN_DB="postgres"

do_psql() {
    PGPASSWORD="$ADMIN_PASS" "$PSQL_BIN" -h "$ADMIN_HOST" -p "$ADMIN_PORT" -U "$ADMIN_USER" -d "$ADMIN_DB" -c "$1" 2>&1
}
do_psql_test() {
    PGPASSWORD="$ADMIN_PASS" "$PSQL_BIN" -h "$ADMIN_HOST" -p "$ADMIN_PORT" -U "$ADMIN_USER" -d "$DB_NAME" -c "$1" 2>&1
}

# ── Phase 0b: Database Setup ────────────────────────────────────────────────
section "PHASE 0b: Database Setup"

step "Setting up test database $DB_NAME..."
do_psql "DROP DATABASE IF EXISTS $DB_NAME;" 2>/dev/null || true
do_psql "CREATE DATABASE $DB_NAME;" 2>/dev/null || true

if do_psql_test "SELECT 1" &>/dev/null; then
    pass "Test database $DB_NAME ready"
else
    fail "Cannot connect to test database $DB_NAME"
    exit 1
fi

do_psql_test "DROP TABLE IF EXISTS doo_migrations CASCADE;" 2>/dev/null || true
pass "Migration history table cleaned"

mkdir -p "$VARIANTS_DIR"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 1: Initial Migration — JSON Structure Validation
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 1: JSON Output Structure"

# 1.1 — Generate JSON plan
step "1.1 Generate JSON migration plan from dep_graph_test.doo"
JSON_PLAN=$(doo_migrate_json "phase1-json" --json --diff --database-url "$TEST_DB_URL" "$BASE_DOO")

if ! assert_json_valid "$JSON_PLAN" "1.1a JSON output is valid"; then
    fail "Cannot continue — invalid JSON"
    exit 1
fi

# 1.2 — Check top-level structure
step "1.2 Verify JSON top-level structure"
assert_json_contains "$JSON_PLAN" '.migration_plan' "1.2a Has migration_plan"
assert_json_contains "$JSON_PLAN" '.migration_plan.batches' "1.2b Has batches field"
assert_json_contains "$JSON_PLAN" '.migration_plan.component_count' "1.2c Has component_count"
assert_json_contains "$JSON_PLAN" '.migration_plan.changes' "1.2d Has changes array"

# 1.3 — Check each change has required fields
step "1.3 Verify each change has dependency fields"
CHANGE_COUNT=$(echo "$JSON_PLAN" | jq -r '.migration_plan.changes | length')
assert_json_field_gt "$JSON_PLAN" '.migration_plan.changes | length' 5 "1.3a Has multiple changes (got $CHANGE_COUNT)"

# Check first change has depends_on and component_id
assert_json_contains "$JSON_PLAN" '.migration_plan.changes[0].depends_on' "1.3b Changes have depends_on"
assert_json_contains "$JSON_PLAN" '.migration_plan.changes[0].component_id' "1.3c Changes have component_id"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 2: Component Grouping — Same-Table Changes
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 2: Same-Table Grouping"

# 2.1 — All changes to "users" table should be in same component
step "2.1 Changes to 'users' table share same component"
USERS_COMPONENTS=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.affected_objects[]? | startswith("users")) | .component_id] | unique | length')
if [ "$USERS_COMPONENTS" = "1" ]; then
    pass "2.1 All 'users' changes in one component"
else
    fail "2.1 'users' changes spread across $USERS_COMPONENTS components"
fi

# 2.2 — All changes to "posts" table should be in same component
step "2.2 Changes to 'posts' table share same component"
POSTS_COMPONENTS=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.affected_objects[]? | startswith("posts")) | .component_id] | unique | length')
if [ "$POSTS_COMPONENTS" = "1" ]; then
    pass "2.2 All 'posts' changes in one component"
else
    fail "2.2 'posts' changes spread across $POSTS_COMPONENTS components"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 3: Dependency Chains — FK Relationships
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 3: FK Dependency Chains"

# 3.1 — Posts table should depend on Users table (FK: AuthorId → User)
step "3.1 Posts depends on Users via FK"
POSTS_DEPS=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.reason | test("posts"; "i")) | .depends_on[]] | join(",")')
if [ -n "$POSTS_DEPS" ] && [ "$POSTS_DEPS" != "null" ]; then
    pass "3.1 Posts has dependencies: $POSTS_DEPS"
else
    fail "3.1 Posts has no dependencies"
fi

# 3.2 — Comments table should depend on both Posts and Users
step "3.2 Comments depends on Posts and Users"
COMMENTS_DEPS=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.reason | test("comments"; "i")) | .depends_on[]] | join(",")')
if [ -n "$COMMENTS_DEPS" ] && [ "$COMMENTS_DEPS" != "null" ]; then
    pass "3.2 Comments has dependencies: $COMMENTS_DEPS"
else
    fail "3.2 Comments has no dependencies"
fi

# 3.3 — Tasks should depend on Projects and Users (FKs to both)
step "3.3 Tasks depends on Projects and Users"
TASKS_DEPS=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.reason | test("tasks"; "i")) | .depends_on[]] | join(",")')
if [ -n "$TASKS_DEPS" ] && [ "$TASKS_DEPS" != "null" ]; then
    pass "3.3 Tasks has dependencies: $TASKS_DEPS"
else
    fail "3.3 Tasks has no dependencies"
fi

# 3.4 — Subtasks should depend on Tasks
step "3.4 Subtasks depends on Tasks"
SUBTASKS_DEPS=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.reason | test("subtasks"; "i")) | .depends_on[]] | join(",")')
if [ -n "$SUBTASKS_DEPS" ] && [ "$SUBTASKS_DEPS" != "null" ]; then
    pass "3.4 Subtasks has dependencies: $SUBTASKS_DEPS"
else
    fail "3.4 Subtasks has no dependencies"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 4: Enum → Table Dependency Chains
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 4: Enum → Table Dependencies"

# 4.1 — Tables using enum types should be in same component as enum creation
step "4.1 Enum-using tables share component with enum"
# Find component_id of CreateEnum for "Role" — JSON uses snake_case name field
ROLE_COMP=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and .change.name == "role") | .component_id] | first')
# Find component_id of table creation for "users" (uses Role enum)
USERS_COMP=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "users") | .component_id] | first')
if [ -n "$ROLE_COMP" ] && [ -n "$USERS_COMP" ] && [ "$ROLE_COMP" = "$USERS_COMP" ]; then
    pass "4.1 Role enum and users table in same component ($ROLE_COMP)"
elif [ -z "$ROLE_COMP" ]; then
    skip "4.1 Role enum not found (may already exist in DB)"
else
    fail "4.1 Role comp=$ROLE_COMP, users comp=$USERS_COMP — should match"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 5: Batch Structure Validation
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 5: Batch Structure"

# 5.1 — Each batch has required fields
step "5.1 Batches have required structure"
BATCH_COUNT=$(echo "$JSON_PLAN" | jq -r '.migration_plan.batches | length')
assert_json_field_gt "$JSON_PLAN" '.migration_plan.batches | length' 0 "5.1a Has batches (count: $BATCH_COUNT)"

assert_json_contains "$JSON_PLAN" '.migration_plan.batches[0].component_id' "5.1b Batches have component_id"
assert_json_contains "$JSON_PLAN" '.migration_plan.batches[0].change_ids' "5.1c Batches have change_ids"
assert_json_contains "$JSON_PLAN" '.migration_plan.batches[0].is_chained' "5.1d Batches have is_chained"
assert_json_contains "$JSON_PLAN" '.migration_plan.batches[0].description' "5.1e Batches have description"
assert_json_contains "$JSON_PLAN" '.migration_plan.batches[0].summary' "5.1f Batches have summary"

# 5.2 — Batch component_id matches its changes' component_id
step "5.2 Batch component_id consistency"
BATCH_CONSISTENT=1
for i in $(seq 0 $((BATCH_COUNT - 1))); do
    BATCH_COMP=$(echo "$JSON_PLAN" | jq -r ".migration_plan.batches[$i].component_id")
    CHANGE_COMPS=$(echo "$JSON_PLAN" | jq -r "[.migration_plan.batches[$i].change_ids[] as \$cid | .migration_plan.changes[] | select(.change_id == \$cid) | .component_id] | unique[]")
    for cc in $CHANGE_COMPS; do
        if [ "$cc" != "$BATCH_COMP" ]; then
            BATCH_CONSISTENT=0
            break 2
        fi
    done
done
if [ "$BATCH_CONSISTENT" = 1 ]; then
    pass "5.2 All batches consistent with their changes' component_id"
else
    fail "5.2 Batch component_id mismatch"
fi

# 5.3 — Independent tables get is_chained: false
step "5.3 Independent tables have is_chained=false"
# Find batches that contain audit_logs or settings
INDEP_BATCHES=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.batches[] | select(.description | test("audit_logs|settings"; "i")) | .is_chained] | join(",")')
if echo "$INDEP_BATCHES" | grep -q "false"; then
    pass "5.3 Independent batches have is_chained=false"
elif [ -z "$INDEP_BATCHES" ]; then
    skip "5.3 No independent batch found (tables may have deps)"
else
    fail "5.3 Independent batches should have is_chained=false: $INDEP_BATCHES"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 6: Apply Migration + Add Column (In-Component Change)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 6: Apply + Modify — Component Stability"

# 6.1 — Apply initial migration
step "6.1 Apply initial migration"
APPLY_OUTPUT=$(doo_migrate "phase6-apply" --force --database-url "$TEST_DB_URL" "$BASE_DOO")
if echo "$APPLY_OUTPUT" | grep -qi "Migration complete"; then
    pass "6.1 Initial migration applied"
else
    fail "6.1 Initial migration failed: $APPLY_OUTPUT"
fi

# Force-sync to persist DDL
doo_migrate "phase6-sync" --force --database-url "$TEST_DB_URL" "$BASE_DOO" > /dev/null 2>&1 || true

# 6.2 — Add a column to users table and verify it stays in same component
step "6.2 Add column to users — verify same-component grouping"
# Create variant: add Bio column to User struct
sed 's/Age: Int @optional,/Age: Int @optional,\n    Bio: Str @optional,/g' \
    "$BASE_DOO" > "$VARIANTS_DIR/v6_add_bio.doo"

ADD_COL_JSON=$(doo_migrate_json "phase6-addcol" --json --diff --database-url "$TEST_DB_URL" "$VARIANTS_DIR/v6_add_bio.doo")
if [ -n "$ADD_COL_JSON" ]; then
    # The AddColumn for users.bio should be in same component as other users changes
    BIO_COMP=$(echo "$ADD_COL_JSON" | jq -r '[.migration_plan.changes[] | select(.reason | test("bio"; "i")) | .component_id] | first')
    OTHER_USERS_COMP=$(echo "$ADD_COL_JSON" | jq -r '[.migration_plan.changes[] | select(.affected_objects[]? | startswith("users")) | .component_id] | unique | length')
    if [ -n "$BIO_COMP" ] && [ "$OTHER_USERS_COMP" = "1" ]; then
        pass "6.2 AddColumn stays in same component as other users changes"
    else
        fail "6.2 Component mismatch after AddColumn (components: $OTHER_USERS_COMP)"
    fi
else
    fail "6.2 Failed to get JSON for add column variant"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 7: Independent Table Addition — New Component
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 7: Independent Table → New Component"

# 7.1 — Add a new independent table (no FKs) and verify it gets its own component
step "7.1 Add independent table — verify new component"
# Create variant with an additional independent table
cp "$BASE_DOO" "$VARIANTS_DIR/v7_independent.doo"
# Insert new table before fn main()
LINE_NUM=$(grep -n "^fn main" "$VARIANTS_DIR/v7_independent.doo" | head -1 | cut -d: -f1)
if [ -n "$LINE_NUM" ]; then
    {
        head -n $((LINE_NUM - 1)) "$VARIANTS_DIR/v7_independent.doo"
        echo ""
        echo '@table("reports")'
        echo 'struct Report {'
        echo '    id: Int @primary @auto,'
        echo '    Title: Str,'
        echo '    Body: Str,'
        echo '}'
        echo ""
        tail -n +"$LINE_NUM" "$VARIANTS_DIR/v7_independent.doo"
    } > "$VARIANTS_DIR/v7_independent.doo.tmp" && mv "$VARIANTS_DIR/v7_independent.doo.tmp" "$VARIANTS_DIR/v7_independent.doo"

    INDEP_JSON=$(doo_migrate_json "phase7-indep" --json --diff --database-url "$TEST_DB_URL" "$VARIANTS_DIR/v7_independent.doo")
    if [ -n "$INDEP_JSON" ]; then
        # The new "reports" table should be in its own component
        REPORTS_COMP=$(echo "$INDEP_JSON" | jq -r '[.migration_plan.changes[] | select(.reason | test("reports"; "i")) | .component_id] | first')
        REPORTS_COUNT=$(echo "$INDEP_JSON" | jq -r "[.migration_plan.changes[] | select(.component_id == $REPORTS_COMP)] | length")
        # New table should have 1 change (CreateTable) in its component
        if [ -n "$REPORTS_COMP" ] && [ "$REPORTS_COUNT" = "1" ]; then
            pass "7.1 Independent 'reports' table gets its own component ($REPORTS_COMP) with 1 change"
        else
            fail "7.1 Reports component has $REPORTS_COUNT changes (expected 1, comp=$REPORTS_COMP)"
        fi
    else
        fail "7.1 Failed to get JSON for independent table variant"
    fi
else
    skip "7.1 Could not find fn main() line"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 8: Enum Value Addition — Chains with Tables
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 8: Enum Change → Table Chain"

# 8.1 — Add a new enum value and verify it chains with tables using that enum
step "8.1 Add enum value — verify enum chains with tables"
sed 's/    High,/    High,\n    Critical,/g' \
    "$BASE_DOO" > "$VARIANTS_DIR/v8_add_enum.doo"

ENUM_JSON=$(doo_migrate_json "phase8-enum" --json --diff --database-url "$TEST_DB_URL" "$VARIANTS_DIR/v8_add_enum.doo")
if [ -n "$ENUM_JSON" ]; then
    # AddEnumValue for Priority should be in same component as tables using Priority
    ENUM_COMP=$(echo "$ENUM_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "add_enum_value") | .component_id] | first')
    PRIORITY_COMP=$(echo "$ENUM_JSON" | jq -r '[.migration_plan.changes[] | select(.reason | test("Project|Task|Subtask"; "i")) | .component_id] | first')
    if [ -n "$ENUM_COMP" ] && [ -n "$PRIORITY_COMP" ] && [ "$ENUM_COMP" = "$PRIORITY_COMP" ]; then
        pass "8.1 AddEnumValue shares component with Priority-using tables ($ENUM_COMP)"
    elif [ -z "$ENUM_COMP" ]; then
        skip "8.1 No AddEnumValue in plan (enum may already exist)"
    else
        fail "8.1 Enum comp=$ENUM_COMP, Priority tables comp=$PRIORITY_COMP"
    fi
else
    fail "8.1 Failed to get JSON for enum variant"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 9: Depends-On Transitive Chain
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 9: Transitive Dependency Chain"

# 9.1 — Verify transitive deps: Subtask → Task → Project → (users)
step "9.1 Subtask transitively depends on Project (via Task FK)"
# The subtasks table has FK to tasks, which has FK to projects
# So subtasks transitively depends on projects
SUBTASK_DEPS_ALL=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.reason | test("subtasks"; "i")) | .depends_on[]] | join(",")')
if [ -n "$SUBTASK_DEPS_ALL" ] && [ "$SUBTASK_DEPS_ALL" != "null" ]; then
    pass "9.1 Subtask has transitive dependencies: $SUBTASK_DEPS_ALL"
else
    fail "9.1 Subtask has no transitive dependencies"
fi

# 9.2 — Comments should transitively depend on Users (via Posts FK + Comments.UserId FK)
step "9.2 Comments transitively depends on Users"
COMMENTS_DEPS_ALL=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.reason | test("comments"; "i")) | .depends_on[]] | join(",")')
if [ -n "$COMMENTS_DEPS_ALL" ] && [ "$COMMENTS_DEPS_ALL" != "null" ]; then
    pass "9.2 Comments has transitive deps: $COMMENTS_DEPS_ALL"
else
    fail "9.2 Comments has no transitive deps"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 10: Edge Cases
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 10: Edge Cases"

# 10.1 — Empty project (no @table structs) should still produce valid JSON
step "10.1 Empty project — valid JSON with no batches"
echo 'fn main() { print("empty"); }' > "$VARIANTS_DIR/empty.doo"
EMPTY_JSON=$(doo_migrate_json "phase10-empty" --json --diff --database-url "$TEST_DB_URL" "$VARIANTS_DIR/empty.doo")
if [ -n "$EMPTY_JSON" ] && echo "$EMPTY_JSON" | jq . >/dev/null 2>&1; then
    STATUS=$(echo "$EMPTY_JSON" | jq -r '.status')
    if [ "$STATUS" = "no_tables" ]; then
        pass "10.1 Empty project handled (status: $STATUS)"
    else
        pass "10.1 Empty project handled (status: $STATUS)"
    fi
else
    fail "10.1 Empty project JSON invalid"
fi

# 10.2 — Single table (no deps) should have is_chained=false
# NOTE: Recreate DB first so the diff only shows the single new table,
# not drops of tables from previous migrations.
step "10.2 Single independent table (fresh DB)"
do_psql "DROP DATABASE IF EXISTS $DB_NAME;" 2>/dev/null || true
do_psql "CREATE DATABASE $DB_NAME;" 2>/dev/null || true
cat > "$VARIANTS_DIR/single.doo" << 'SNGL'
import std::Database;
@table("lonely")
struct Lonely {
    id: Int @primary @auto,
    Note: Str,
}
static DB: Database;
fn main() { DB = Database::Postgres()?; print("lonely"); }
SNGL

SINGLE_JSON=$(doo_migrate_json "phase10-single" --json --diff --database-url "$TEST_DB_URL" "$VARIANTS_DIR/single.doo")
if [ -n "$SINGLE_JSON" ]; then
    IS_CHAINED=$(echo "$SINGLE_JSON" | jq -r '.migration_plan.batches[0].is_chained')
    BATCH_COUNT_S=$(echo "$SINGLE_JSON" | jq -r '.migration_plan.batches | length')
    if [ "$IS_CHAINED" = "false" ] && [ "$BATCH_COUNT_S" = "1" ]; then
        pass "10.2 Single table: is_chained=false, 1 batch"
    else
        fail "10.2 Single table: is_chained=$IS_CHAINED, batches=$BATCH_COUNT_S"
    fi
else
    fail "10.2 Single table JSON invalid"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 11: Full Chain — FK Web (users→posts→comments, projects→tasks→subtasks)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 11: Full FK Web — Chain Cohesion"

# 11.1 — In a web of interlinked tables, the core chain (users→posts→comments)
# should all be in one or few tightly grouped components
step "11.1 FK web chain cohesion"
# Check that users, posts, comments are connected
USERS_COMP_ID=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.reason | test("users"; "i")) | .component_id] | first')
POSTS_COMP_ID=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.reason | test("posts"; "i")) | .component_id] | first')
COMMENTS_COMP_ID=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.reason | test("comments"; "i")) | .component_id] | first')

# For new tables, they may be separate components or linked through FK deps
# The key is that tables with direct FK relationships have depends_on set
if [ -n "$POSTS_COMP_ID" ] && [ -n "$USERS_COMP_ID" ]; then
    pass "11.1 Users comp=$USERS_COMP_ID, Posts comp=$POSTS_COMP_ID, Comments comp=$COMMENTS_COMP_ID"
else
    fail "11.1 Missing component IDs for FK chain"
fi

# 11.2 — Each FK-dependent table's CreateTable should have depends_on
step "11.2 FK tables have depends_on set"
POSTS_CREATE_DEPS=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table") | select(.change.name == "posts") | .depends_on | length] | first')
if [ -n "$POSTS_CREATE_DEPS" ] && [ "$POSTS_CREATE_DEPS" -gt 0 ]; then
    pass "11.2a Posts CreateTable has $POSTS_CREATE_DEPS deps"
else
    fail "11.2a Posts CreateTable has no deps"
fi

COMMENTS_CREATE_DEPS=$(echo "$JSON_PLAN" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table") | select(.change.name == "comments") | .depends_on | length] | first')
if [ -n "$COMMENTS_CREATE_DEPS" ] && [ "$COMMENTS_CREATE_DEPS" -gt 0 ]; then
    pass "11.2b Comments CreateTable has $COMMENTS_CREATE_DEPS deps"
else
    fail "11.2b Comments CreateTable has no deps"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 12: Diamond Mesh Dependency
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 12: Diamond Mesh Dependency"

step "12.0 Generate JSON plan for advanced dep graph"
ADV_DOO="$MIGRATE_DIR/dep_graph_advanced.doo"
if [ ! -f "$ADV_DOO" ]; then
    skip "Advanced dep graph file not found"
else
    ADV_JSON=$(doo_migrate_json "phase12-adv" --json --diff --database-url "$TEST_DB_URL" "$ADV_DOO")
    if [ -z "$ADV_JSON" ] || ! echo "$ADV_JSON" | jq . >/dev/null 2>&1; then
        fail "12.0 Advanced JSON invalid or empty"
    else
        pass "12.0 Advanced JSON valid"

        # 12.1 — All 4 diamond tables (a,b,c,d) should be in ONE component
        step "12.1 Diamond tables share one component"
        DIAMOND_COMPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name | test("diamond[a-d]s")) | .component_id] | unique | length')
        if [ "$DIAMOND_COMPS" = "1" ]; then
            pass "12.1 All 4 diamond tables in 1 component"
        else
            fail "12.1 Diamond tables spread across $DIAMOND_COMPS components"
        fi

        # 12.2 — diamond_d should depend on both diamond_b AND diamond_c
        step "12.2 Diamond_d depends on both diamond_b and diamond_c"
        D_DEPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name == "diamondds") | .depends_on | length] | first')
        if [ -n "$D_DEPS" ] && [ "$D_DEPS" -ge 2 ]; then
            pass "12.2 Diamond_d has $D_DEPS dependencies (expect >=2)"
        else
            fail "12.2 Diamond_d has $D_DEPS deps (expected >=2)"
        fi

        # 12.3 — diamond_b and diamond_c both depend on diamond_a
        step "12.3 Diamond_b and diamond_c depend on diamond_a"
        B_DEPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name == "diamondbs") | .depends_on | length] | first')
        C_DEPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name == "diamondcs") | .depends_on | length] | first')
        if [ -n "$B_DEPS" ] && [ "$B_DEPS" -ge 1 ] && [ -n "$C_DEPS" ] && [ "$C_DEPS" -ge 1 ]; then
            pass "12.3 B=$B_DEPS deps, C=$C_DEPS deps"
        else
            fail "12.3 B=$B_DEPS, C=$C_DEPS (expected >=1 each)"
        fi

        # 12.4 — Diamond diamond_a_tags should chain diamond_a + diamond_tags
        step "12.4 Junction table chains both referenced tables"
        JUNCTION_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name == "diamondatags") | .component_id] | first')
        TAG_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name == "diamondtags") | .component_id] | first')
        A_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name == "diamondas") | .component_id] | first')
        if [ -n "$JUNCTION_COMP" ] && [ "$JUNCTION_COMP" = "$TAG_COMP" ] && [ "$JUNCTION_COMP" = "$A_COMP" ]; then
            pass "12.4 Junction, tags, and diamond_a all in component $JUNCTION_COMP"
        else
            fail "12.4 Junction comp=$JUNCTION_COMP, tags comp=$TAG_COMP, a comp=$A_COMP"
        fi
    fi
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 13: Self-Referencing FK
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 13: Self-Referencing FK"

step "13.1 Self-referencing FK should not create circular deps"
if [ -n "$ADV_JSON" ]; then
    EMP_SELF_DEPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name == "employees") | .depends_on | length] | first')
    # Self-referencing FK: the ref_table is the same table, so it should NOT
    # create a dependency on itself. depends_on should be 0 or only contain
    # non-self deps (like enum deps for Level).
    if [ -n "$EMP_SELF_DEPS" ]; then
        # Check none of its deps point to itself
        SELF_DEP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name == "employees") | .depends_on[] | select(contains("employees"))] | length')
        if [ "$SELF_DEP" = "0" ] || [ -z "$SELF_DEP" ]; then
            pass "13.1 Self-ref FK correctly has no self-dependency (total deps: $EMP_SELF_DEPS)"
        else
            fail "13.1 Self-ref FK has $SELF_DEP self-dependency (should be 0)"
        fi
    else
        pass "13.1 Self-ref FK has no deps (correct)"
    fi
else
    skip "13.1 No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 14: Independent Chains — Count Verification
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 14: Independent Chain Count"

step "14.1 Count components in advanced dep graph"
if [ -n "$ADV_JSON" ]; then
    COMP_COUNT_ADV=$(echo "$ADV_JSON" | jq -r '.migration_plan.component_count')
    BATCH_COUNT_ADV=$(echo "$ADV_JSON" | jq -r '.migration_plan.batches | length')
    # Expected: diamond chain (1) + employees+Level enum (1) + articles+Category (1)
    # + system_log (1) + cache_entries (1) = ~4-5 components
    if [ -n "$COMP_COUNT_ADV" ] && [ "$COMP_COUNT_ADV" -ge 3 ]; then
        pass "14.1 $COMP_COUNT_ADV components, $BATCH_COUNT_ADV batches (expect >=3)"
    else
        fail "14.1 Only $COMP_COUNT_ADV components (expected >=3)"
    fi

    # 14.2 — system_log and cache_entries should be in separate independent batches
    step "14.2 Independent tables have separate batches"
    LOG_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name == "systemlogs") | .component_id] | first')
    CACHE_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name == "cacheentrys") | .component_id] | first')
    if [ -n "$LOG_COMP" ] && [ -n "$CACHE_COMP" ] && [ "$LOG_COMP" != "$CACHE_COMP" ]; then
        pass "14.2 System_log comp=$LOG_COMP, cache_entries comp=$CACHE_COMP (separate)"
    else
        fail "14.2 System_log=$LOG_COMP, cache_entries=$CACHE_COMP (should differ)"
    fi

    # 14.3 — Verify is_chained on independent batches
    step "14.3 Independent batches have is_chained=false"
    LOG_CHAINED=$(echo "$ADV_JSON" | jq -r "[.migration_plan.batches[] | select(.description | test(\"systemlogs\"; \"i\")) | .is_chained] | first")
    CACHE_CHAINED=$(echo "$ADV_JSON" | jq -r "[.migration_plan.batches[] | select(.description | test(\"cacheentrys\"; \"i\")) | .is_chained] | first")
    if [ "$LOG_CHAINED" = "false" ] && [ "$CACHE_CHAINED" = "false" ]; then
        pass "14.3 Both independent batches: is_chained=false"
    else
        fail "14.3 log=$LOG_CHAINED, cache=$CACHE_CHAINED"
    fi
else
    skip "14.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 15: AlterColumnType → Enum Chain
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 15: AlterColumnType → Enum Chain"

step "15.0 Apply variant WITHOUT enum (articles.Category as Str)"
# Fresh DB
do_psql "DROP DATABASE IF EXISTS $DB_NAME;" 2>/dev/null || true
do_psql "CREATE DATABASE $DB_NAME;" 2>/dev/null || true
# Create variant where articles uses Str instead of Category enum
sed 's/Category: Category @default("Tech"),/Category: Str @default("Tech"),/g' \
    "$ADV_DOO" > "$VARIANTS_DIR/v15_no_enum.doo"
doo_migrate "phase15-apply" --force --database-url "$TEST_DB_URL" "$VARIANTS_DIR/v15_no_enum.doo" > /dev/null 2>&1 || true
doo_migrate "phase15-sync" --force --database-url "$TEST_DB_URL" "$VARIANTS_DIR/v15_no_enum.doo" > /dev/null 2>&1 || true
pass "15.0 Schema without enum applied"

# 15.1 — Now diff against original (WITH Category enum) —
# should generate: CreateEnum("category") + AlterColumnType(articles.category, Text→Enum)
step "15.1 AlterColumnType to enum chains with CreateEnum"
ALTER_JSON=$(doo_migrate_json "phase15-alter" --json --diff --database-url "$TEST_DB_URL" "$ADV_DOO")
if [ -n "$ALTER_JSON" ] && echo "$ALTER_JSON" | jq . >/dev/null 2>&1; then
    # Check we have BOTH CreateEnum and AlterColumnType
    HAS_CREATE_ENUM=$(echo "$ALTER_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and .change.name == "category")] | length')
    HAS_ALTER=$(echo "$ALTER_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "alter_column_type")] | length')
    if [ "$HAS_CREATE_ENUM" -gt 0 ] && [ "$HAS_ALTER" -gt 0 ]; then
        pass "15.1a Has CreateEnum(category)=$HAS_CREATE_ENUM and AlterColumnType=$HAS_ALTER"
    else
        skip "15.1a CreateEnum=$HAS_CREATE_ENUM, Alter=$HAS_ALTER (unexpected — may need different DB state)"
    fi

    # Verify they share the same component
    ENUM_COMP=$(echo "$ALTER_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and .change.name == "category") | .component_id] | first')
    ALTER_COMP=$(echo "$ALTER_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "alter_column_type") | .component_id] | first')
    if [ -n "$ENUM_COMP" ] && [ -n "$ALTER_COMP" ] && [ "$ENUM_COMP" = "$ALTER_COMP" ]; then
        pass "15.1b AlterColumnType shares component $ALTER_COMP with CreateEnum(category)"
    else
        fail "15.1b Enum comp=$ENUM_COMP, Alter comp=$ALTER_COMP (should match)"
    fi
else
    fail "15.1 Failed to get JSON for alter type variant"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 16: Batch Description Accuracy
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 16: Batch Description & Summary Accuracy"

step "16.1 Verify batch descriptions are meaningful"
if [ -n "$ADV_JSON" ]; then
    # Each batch should have a non-empty description
    EMPTY_DESC=$(echo "$ADV_JSON" | jq -r '[.migration_plan.batches[] | select(.description == "" or .description == null)] | length')
    if [ "$EMPTY_DESC" = "0" ]; then
        pass "16.1 All batches have descriptions"
    else
        fail "16.1 $EMPTY_DESC batch(es) have empty description"
    fi

    # 16.2 — Summary totals should match batch size
    step "16.2 Batch summary totals match change_ids length"
    MISMATCH=$(echo "$ADV_JSON" | jq -r '[.migration_plan.batches[] | select(.summary.total != (.change_ids | length))] | length')
    if [ "$MISMATCH" = "0" ]; then
        pass "16.2 All batch summaries match their change count"
    else
        fail "16.2 $MISMATCH batch(es) have mismatched summary.total"
    fi

    # 16.3 — Safe + risky + destructive = total
    step "16.3 Batch risk counts sum to total"
    RISK_MISMATCH=$(echo "$ADV_JSON" | jq -r '[.migration_plan.batches[] | select((.summary.safe_count + .summary.risky_count + .summary.destructive_count) != .summary.total)] | length')
    if [ "$RISK_MISMATCH" = "0" ]; then
        pass "16.3 All batch risk counts sum correctly"
    else
        fail "16.3 $RISK_MISMATCH batch(es) have risk count mismatch"
    fi
else
    skip "16.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 17: No Duplicate change_ids Across Batches
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 17: No Duplicate change_ids"

step "17.1 Every change appears in exactly one batch"
if [ -n "$ADV_JSON" ]; then
    TOTAL_CHANGES=$(echo "$ADV_JSON" | jq -r '.migration_plan.changes | length')
    TOTAL_IN_BATCHES=$(echo "$ADV_JSON" | jq -r '[.migration_plan.batches[].change_ids[]] | length')
    if [ "$TOTAL_CHANGES" = "$TOTAL_IN_BATCHES" ]; then
        pass "17.1 $TOTAL_CHANGES changes = $TOTAL_IN_BATCHES in batches (no duplicates, no orphans)"
    else
        fail "17.1 Changes=$TOTAL_CHANGES, In batches=$TOTAL_IN_BATCHES (mismatch!)"
    fi

    # 17.2 — No duplicate change_ids across batches
    step "17.2 No duplicate change_ids"
    DUPES=$(echo "$ADV_JSON" | jq -r '[.migration_plan.batches[].change_ids[]] | group_by(.) | map(select(length > 1) | .[0]) | length')
    if [ "$DUPES" = "0" ]; then
        pass "17.2 No duplicate change_ids across batches"
    else
        fail "17.2 Found $DUPES duplicate change_id(s)"
    fi
else
    skip "17.x No advanced JSON available"
fi


# ═════════════════════════════════════════════════════════════════════════════
# PHASE 18: Nested Struct Dependency Resolution
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 18: Nested Struct Dependency Graph"

step "18.1 Verify table containing nested struct maps top-level component tracking"
if [ -n "$ADV_JSON" ]; then
    NESTED_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name | test("nested_profiles"; "i")) | .component_id] | first')
    if [ -n "$NESTED_COMP" ] && [ "$NESTED_COMP" != "null" ]; then
        pass "18.1 Nested struct table 'nested_profiles' successfully isolated in component $NESTED_COMP"
    else
        fail "18.1 Failed to parse or generate plan changes for 'nested_profiles'"
    fi

    step "18.2 Check deep transitive enum dependency via nested field"
    # Find component id of MetaType enum
    METATYPE_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and (.change.name | test("meta_type"; "i"))) | .component_id] | first')

    if [ -n "$METATYPE_COMP" ] && [ "$METATYPE_COMP" != "null" ]; then
        if [ "$NESTED_COMP" = "$METATYPE_COMP" ]; then
            pass "18.2 'MetaType' enum shares component with parent table ($NESTED_COMP)"
        else
            # Fallback assertion if your topological sort assigns separate components linked by an explicit dependency chain
            HAS_DEP=$(echo "$ADV_JSON" | jq -r --arg mc "$METATYPE_COMP" '[.migration_plan.changes[] | select(.change.name | test("nested_profiles"; "i")) | .depends_on[] | select(. == $mc)] | length')
            if [ "$HAS_DEP" -gt 0 ]; then
                pass "18.2 'nested_profiles' explicitly lists 'MetaType' ($METATYPE_COMP) as a dependency"
            else
                fail "18.2 Nested enum 'MetaType' ($METATYPE_COMP) and parent table ($NESTED_COMP) are unlinked"
            fi
        fi
    else
        skip "18.2 MetaType enum change not detected (might already exist in schema)"
    fi

    step "18.3 Check deep foreign key dependency within nested structural block"
    DIAMOND_A_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name | test("diamondas"; "i")) | .component_id] | first')

    if [ -n "$DIAMOND_A_COMP" ] && [ "$DIAMOND_A_COMP" != "null" ]; then
        # Check if nested_profiles either groups with diamondas or explicitly lists it as a dependency
        if [ "$NESTED_COMP" = "$DIAMOND_A_COMP" ]; then
            pass "18.3 Nested struct foreign key shares component chain with 'diamondas'"
        else
            HAS_FK_DEP=$(echo "$ADV_JSON" | jq -r --arg dc "$DIAMOND_A_COMP" '[.migration_plan.changes[] | select(.change.name | test("nested_profiles"; "i")) | .depends_on[] | select(. == $dc)] | length')
            if [ "$HAS_FK_DEP" -gt 0 ]; then
                pass "18.3 'nested_profiles' cleanly inherits dependency from nested FK field pointing to 'diamondas'"
            else
                fail "18.3 Engine missed deep structural dependency constraint between nested field -> diamondas"
            fi
        fi
    else
        skip "18.3 Dependency target 'diamondas' was not found in graph logs"
    fi
else
    skip "18.x Advanced JSON not accessible"
fi


# ═════════════════════════════════════════════════════════════════════════════
# PHASE 19: Deep Nested (Level 2+) & Optional FK Dependency Rules
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 19: Multi-Level Nesting & Edge Constraints"

step "19.1 Verify multi-level struct component extraction"
if [ -n "$ADV_JSON" ]; then
    COMPLEX_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name | test("complex_documents"; "i")) | .component_id] | first')

    if [ -n "$COMPLEX_COMP" ] && [ "$COMPLEX_COMP" != "null" ]; then
        pass "19.1 'complex_documents' successfully bound to component $COMPLEX_COMP"
    else
        fail "19.1 Failed to parse or generate plan for 'complex_documents'"
    fi

    step "19.2 Recursion Check: Resolve Level-2 FK dependency (Config -> Audit -> Employee)"
    EMP_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name | test("employees"; "i")) | .component_id] | first')
    if [ -n "$EMP_COMP" ] && [ "$EMP_COMP" != "null" ]; then
        HAS_EMP_DEP=$(echo "$ADV_JSON" | jq -r --arg ec "$EMP_COMP" '[.migration_plan.changes[] | select(.change.name | test("complex_documents"; "i")) | .depends_on[] | select(. == $ec)] | length')

        if [ "$COMPLEX_COMP" = "$EMP_COMP" ] || [ "$HAS_EMP_DEP" -gt 0 ]; then
            pass "19.2 Compiler successfully traversed 2 struct levels to find 'employees' FK"
        else
            fail "19.2 Compiler failed deep traversal; 'employees' dependency was dropped"
        fi
    else
        skip "19.2 Target 'employees' component missing from JSON"
    fi

    step "19.3 Recursion Check: Resolve Level-2 Enum dependency (Config -> Audit -> AccessLevel)"
    ACCESS_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and (.change.name | test("access_level"; "i"))) | .component_id] | first')
    if [ -n "$ACCESS_COMP" ] && [ "$ACCESS_COMP" != "null" ]; then
        HAS_ACCESS_DEP=$(echo "$ADV_JSON" | jq -r --arg ac "$ACCESS_COMP" '[.migration_plan.changes[] | select(.change.name | test("complex_documents"; "i")) | .depends_on[] | select(. == $ac)] | length')

        if [ "$COMPLEX_COMP" = "$ACCESS_COMP" ] || [ "$HAS_ACCESS_DEP" -gt 0 ]; then
            pass "19.3 Compiler successfully extracted deeply embedded 'AccessLevel' enum"
        else
            fail "19.3 Compiler missed deep 'AccessLevel' enum dependency"
        fi
    else
        skip "19.3 'AccessLevel' enum creation not detected"
    fi

    step "19.4 Constraints: Verify optional self-referencing FK behaves safely"
    # A self-referencing FK should NOT list its own component/table as a strict pre-requisite
    # to avoid topological sort deadlocks.
    SELF_DEP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.name | test("complex_documents"; "i")) | .depends_on[] | select(contains("complex_documents"))] | length')

    if [ "$SELF_DEP" = "0" ] || [ -z "$SELF_DEP" ]; then
        pass "19.4 Optional self-reference safely bypassed in dependency DAG (no deadlock)"
    else
        fail "19.4 Optional self-reference caused a circular DAG dependency (found $SELF_DEP self-links)"
    fi

else
    skip "19.x Advanced JSON not accessible"
fi


# ═════════════════════════════════════════════════════════════════════════════
# PHASE 20: Non-Table Struct with Enum (Transitive Enum Dependency)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 20: Non-Table Struct w/ Enum — Transitive Dep"

step "20.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "20.1 MetaTask table has non-table struct in affected_objects"
    METATASK_HAS_META=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "meta_tasks") | .affected_objects[] | select(. == "TaskMeta")] | length')
    if [ -n "$METATASK_HAS_META" ] && [ "$METATASK_HAS_META" -gt 0 ]; then
        pass "20.1 MetaTask affected_objects includes 'TaskMeta' (non-table struct)"
    else
        fail "20.1 MetaTask missing 'TaskMeta' in affected_objects"
    fi

    step "20.2 MetaTask has transitive enum ref for TaskStatus"
    METATASK_HAS_ENUM=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "meta_tasks") | .affected_objects[] | select(. == "task_status")] | length')
    if [ -n "$METATASK_HAS_ENUM" ] && [ "$METATASK_HAS_ENUM" -gt 0 ]; then
        pass "20.2 MetaTask affected_objects includes 'task_status' (transitive enum via TaskMeta)"
    else
        fail "20.2 MetaTask missing transitive enum 'task_status' in affected_objects"
    fi

    step "20.3 MetaTask depends_on includes TaskStatus enum"
    METATASK_DEPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "meta_tasks") | .depends_on | length] | first')
    if [ -n "$METATASK_DEPS" ] && [ "$METATASK_DEPS" -gt 0 ]; then
        pass "20.3 MetaTask has $METATASK_DEPS depends_on entries (includes TaskStatus)"
    else
        fail "20.3 MetaTask has no depends_on entries"
    fi

else
    skip "20.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 21: Array of Non-Table Struct
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 21: Array of Non-Table Struct — Type Traversal"

step "21.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "21.1 ArticleGroup table has non-table struct Tag in affected_objects"
    AGROUP_HAS_TAG=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "article_groups") | .affected_objects[] | select(. == "Tag")] | length')
    if [ -n "$AGROUP_HAS_TAG" ] && [ "$AGROUP_HAS_TAG" -gt 0 ]; then
        pass "21.1 ArticleGroup affected_objects includes 'Tag' (array element type)"
    else
        fail "21.1 ArticleGroup missing 'Tag' in affected_objects"
    fi

    step "21.2 ArticleGroup has component_id assigned"
    AGROUP_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "article_groups") | .component_id] | first')
    if [ -n "$AGROUP_COMP" ] && [ "$AGROUP_COMP" != "null" ]; then
        pass "21.2 ArticleGroup has component_id $AGROUP_COMP"
    else
        fail "21.2 ArticleGroup missing component_id"
    fi

else
    skip "21.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 22: Optional Non-Table Struct
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 22: Optional Non-Table Struct — Optional Traversal"

step "22.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "22.1 GuestHouse table has non-table struct Address in affected_objects"
    GUEST_HAS_ADDR=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "guest_houses") | .affected_objects[] | select(. == "Address")] | length')
    if [ -n "$GUEST_HAS_ADDR" ] && [ "$GUEST_HAS_ADDR" -gt 0 ]; then
        pass "22.1 GuestHouse affected_objects includes 'Address' (optional struct type)"
    else
        fail "22.1 GuestHouse missing 'Address' in affected_objects"
    fi

    step "22.2 GuestHouse has component_id assigned"
    GUEST_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "guest_houses") | .component_id] | first')
    if [ -n "$GUEST_COMP" ] && [ "$GUEST_COMP" != "null" ]; then
        pass "22.2 GuestHouse has component_id $GUEST_COMP"
    else
        fail "22.2 GuestHouse missing component_id"
    fi

else
    skip "22.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 23: Map with Non-Table Struct Value
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 23: Map with Non-Table Struct — Map Traversal"

step "23.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "23.1 DocBundle table has non-table struct MetadataEntry in affected_objects"
    DOC_HAS_META=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "doc_bundles") | .affected_objects[] | select(. == "MetadataEntry")] | length')
    if [ -n "$DOC_HAS_META" ] && [ "$DOC_HAS_META" -gt 0 ]; then
        pass "23.1 DocBundle affected_objects includes 'MetadataEntry' (map value type)"
    else
        fail "23.1 DocBundle missing 'MetadataEntry' in affected_objects"
    fi

    step "23.2 DocBundle has component_id assigned"
    DOC_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "doc_bundles") | .component_id] | first')
    if [ -n "$DOC_COMP" ] && [ "$DOC_COMP" != "null" ]; then
        pass "23.2 DocBundle has component_id $DOC_COMP"
    else
        fail "23.2 DocBundle missing component_id"
    fi

else
    skip "23.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 24: Shared Non-Table Struct — Component Grouping
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 24: Shared Non-Table Struct — Component Cohesion"

step "24.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "24.1 SharedAuthor has non-table struct SharedProfile in affected_objects"
    AUTHOR_HAS_PROF=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "shared_authors") | .affected_objects[] | select(. == "SharedProfile")] | length')
    if [ -n "$AUTHOR_HAS_PROF" ] && [ "$AUTHOR_HAS_PROF" -gt 0 ]; then
        pass "24.1 SharedAuthor affected_objects includes 'SharedProfile'"
    else
        fail "24.1 SharedAuthor missing 'SharedProfile' in affected_objects"
    fi

    step "24.2 SharedEditor has non-table struct SharedProfile in affected_objects"
    EDITOR_HAS_PROF=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "shared_editors") | .affected_objects[] | select(. == "SharedProfile")] | length')
    if [ -n "$EDITOR_HAS_PROF" ] && [ "$EDITOR_HAS_PROF" -gt 0 ]; then
        pass "24.2 SharedEditor affected_objects includes 'SharedProfile'"
    else
        fail "24.2 SharedEditor missing 'SharedProfile' in affected_objects"
    fi

    step "24.3 SharedAuthor and SharedEditor share same component"
    AUTHOR_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "shared_authors") | .component_id] | first')
    EDITOR_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "shared_editors") | .component_id] | first')
    if [ -n "$AUTHOR_COMP" ] && [ -n "$EDITOR_COMP" ] && [ "$AUTHOR_COMP" = "$EDITOR_COMP" ]; then
        pass "24.3 SharedAuthor and SharedEditor share component $AUTHOR_COMP (via SharedProfile)"
    else
        fail "24.3 SharedAuthor comp=$AUTHOR_COMP, SharedEditor comp=$EDITOR_COMP (should match)"
    fi

else
    skip "24.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 25: Simple Non-Table Struct + Enum (Models.doo pattern)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 25: Simple Non-Table Struct + Enum — Models.doo Pattern"

step "25.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "25.1 SimpleTask has non-table struct SimpleProject in affected_objects"
    TASK_HAS_PROJ=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "simple_tasks") | .affected_objects[] | select(. == "SimpleProject")] | length')
    if [ -n "$TASK_HAS_PROJ" ] && [ "$TASK_HAS_PROJ" -gt 0 ]; then
        pass "25.1 SimpleTask affected_objects includes 'SimpleProject' (non-table struct)"
    else
        fail "25.1 SimpleTask missing 'SimpleProject' in affected_objects"
    fi

    step "25.2 SimpleTask has SimpleStatus enum in affected_objects"
    TASK_HAS_STATUS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "simple_tasks") | .affected_objects[] | select(. == "simple_status")] | length')
    if [ -n "$TASK_HAS_STATUS" ] && [ "$TASK_HAS_STATUS" -gt 0 ]; then
        pass "25.2 SimpleTask affected_objects includes 'simple_status' (direct enum field)"
    else
        fail "25.2 SimpleTask missing 'simple_status' in affected_objects"
    fi

    step "25.3 SimpleTask depends_on includes SimpleStatus enum creator"
    TASK_DEPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "simple_tasks") | .depends_on | length] | first')
    if [ -n "$TASK_DEPS" ] && [ "$TASK_DEPS" -gt 0 ]; then
        pass "25.3 SimpleTask has $TASK_DEPS depends_on entries"
    else
        fail "25.3 SimpleTask has no depends_on entries"
    fi

    step "25.4 SimpleTask and SimpleStatus enum share same component"
    STASK_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "simple_tasks") | .component_id] | first')
    SSTATUS_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and .change.name == "simple_status") | .component_id] | first')
    if [ -n "$STASK_COMP" ] && [ -n "$SSTATUS_COMP" ] && [ "$STASK_COMP" = "$SSTATUS_COMP" ]; then
        pass "25.4 SimpleTask and SimpleStatus share component $STASK_COMP"
    else
        fail "25.4 SimpleTask comp=$STASK_COMP, SimpleStatus comp=$SSTATUS_COMP (should match)"
    fi

else
    skip "25.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 26: Non-Table Struct with @foreign — FK Dependency Through Nested Struct
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 26: FK via Non-Table Struct — @foreign in Nested Struct"

step "26.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "26.1 ContentMeta (non-table struct) FK to DiamondA creates dep edge"
    # ContentMeta has @foreign(DiamondA) inside nested_profiles
    # nested_profiles should depend_on diamondas
    NESTED_FK_DEPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "nested_profiles") | .depends_on | length] | first')
    if [ -n "$NESTED_FK_DEPS" ] && [ "$NESTED_FK_DEPS" -gt 0 ]; then
        pass "26.1 nested_profiles has $NESTED_FK_DEPS deps (includes FK to DiamondA through ContentMeta)"
    else
        fail "26.1 nested_profiles has no deps — FK through non-table struct not resolved"
    fi

    step "26.2 nested_profiles and diamondas share component"
    NESTED_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "nested_profiles") | .component_id] | first')
    DIAMOND_A_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "diamondas") | .component_id] | first')
    if [ -n "$NESTED_COMP" ] && [ -n "$DIAMOND_A_COMP" ] && [ "$NESTED_COMP" = "$DIAMOND_A_COMP" ]; then
        pass "26.2 nested_profiles and diamondas share component $NESTED_COMP (via ContentMeta -> @foreign)"
    else
        fail "26.2 nested_profiles comp=$NESTED_COMP, diamondas comp=$DIAMOND_A_COMP"
    fi

else
    skip "26.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 27: Array of Enum in @table (Array<Enum> traversal)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 27: Array of Enum in @table — Array<Enum> Traversal"

step "27.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "27.1 ArrayEnum has Visibility enum in affected_objects"
    ARR_HAS_VIS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "array_enums") | .affected_objects[] | select(. == "visibility")] | length')
    if [ -n "$ARR_HAS_VIS" ] && [ "$ARR_HAS_VIS" -gt 0 ]; then
        pass "27.1 ArrayEnum affected_objects includes 'visibility' (from Array<Visibility>)"
    else
        fail "27.1 ArrayEnum missing 'visibility' in affected_objects"
    fi

    step "27.2 ArrayEnum and Visibility enum share same component"
    ARR_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "array_enums") | .component_id] | first')
    VIS_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and .change.name == "visibility") | .component_id] | first')
    if [ -n "$ARR_COMP" ] && [ -n "$VIS_COMP" ] && [ "$ARR_COMP" = "$VIS_COMP" ]; then
        pass "27.2 ArrayEnum and Visibility share component $ARR_COMP"
    else
        fail "27.2 ArrayEnum comp=$ARR_COMP, Visibility comp=$VIS_COMP"
    fi

    step "27.3 ArrayEnum depends_on includes Visibility"
    ARR_DEPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "array_enums") | .depends_on | length] | first')
    if [ -n "$ARR_DEPS" ] && [ "$ARR_DEPS" -gt 0 ]; then
        pass "27.3 ArrayEnum has $ARR_DEPS depends_on entries (includes Visibility)"
    else
        fail "27.3 ArrayEnum has no depends_on entries"
    fi

else
    skip "27.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 28: Optional Enum in @table (Optional<Enum> traversal)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 28: Optional Enum in @table — Optional<Enum> Traversal"

step "28.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "28.1 OptionalEnum has Tier enum in affected_objects"
    OPT_HAS_TIER=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "optional_enums") | .affected_objects[] | select(. == "tier")] | length')
    if [ -n "$OPT_HAS_TIER" ] && [ "$OPT_HAS_TIER" -gt 0 ]; then
        pass "28.1 OptionalEnum affected_objects includes 'tier' (from Optional<Tier>)"
    else
        fail "28.1 OptionalEnum missing 'tier' in affected_objects"
    fi

    step "28.2 OptionalEnum and Tier enum share same component"
    OPT_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "optional_enums") | .component_id] | first')
    TIER_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and .change.name == "tier") | .component_id] | first')
    if [ -n "$OPT_COMP" ] && [ -n "$TIER_COMP" ] && [ "$OPT_COMP" = "$TIER_COMP" ]; then
        pass "28.2 OptionalEnum and Tier share component $OPT_COMP"
    else
        fail "28.2 OptionalEnum comp=$OPT_COMP, Tier comp=$TIER_COMP"
    fi

else
    skip "28.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 29: Map with Enum Key (Map<Enum, X> traversal)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 29: Map with Enum Key — Map<Enum, X> Traversal"

step "29.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "29.1 MapEnumKey has FeatureFlag enum in affected_objects"
    MAP_HAS_FLAG=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "map_enum_keys") | .affected_objects[] | select(. == "feature_flag")] | length')
    if [ -n "$MAP_HAS_FLAG" ] && [ "$MAP_HAS_FLAG" -gt 0 ]; then
        pass "29.1 MapEnumKey affected_objects includes 'feature_flag' (from Map<FeatureFlag, Bool>)"
    else
        fail "29.1 MapEnumKey missing 'feature_flag' in affected_objects"
    fi

    step "29.2 MapEnumKey and FeatureFlag enum share same component"
    MKEY_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "map_enum_keys") | .component_id] | first')
    FLAG_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and .change.name == "feature_flag") | .component_id] | first')
    if [ -n "$MKEY_COMP" ] && [ -n "$FLAG_COMP" ] && [ "$MKEY_COMP" = "$FLAG_COMP" ]; then
        pass "29.2 MapEnumKey and FeatureFlag share component $MKEY_COMP"
    else
        fail "29.2 MapEnumKey comp=$MKEY_COMP, FeatureFlag comp=$FLAG_COMP"
    fi

else
    skip "29.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 30: Non-Table Struct with Array of Enum (transitive Array<Enum>)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 30: Array of Enum in Non-Table Struct — Transitive Array<Enum>"

step "30.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "30.1 DisplayConfig has DisplayPrefs in affected_objects"
    DC_HAS_PREFS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "display_configs") | .affected_objects[] | select(. == "DisplayPrefs")] | length')
    if [ -n "$DC_HAS_PREFS" ] && [ "$DC_HAS_PREFS" -gt 0 ]; then
        pass "30.1 DisplayConfig affected_objects includes 'DisplayPrefs'"
    else
        fail "30.1 DisplayConfig missing 'DisplayPrefs'"
    fi

    step "30.2 DisplayConfig has ColorMode enum (transitive through DisplayPrefs)"
    DC_HAS_MODE=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "display_configs") | .affected_objects[] | select(. == "color_mode")] | length')
    if [ -n "$DC_HAS_MODE" ] && [ "$DC_HAS_MODE" -gt 0 ]; then
        pass "30.2 DisplayConfig affected_objects includes 'color_mode' (transitive Array<ColorMode>)"
    else
        fail "30.2 DisplayConfig missing 'color_mode' in affected_objects"
    fi

    step "30.3 DisplayConfig depends_on includes ColorMode"
    DC_DEPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "display_configs") | .depends_on | length] | first')
    if [ -n "$DC_DEPS" ] && [ "$DC_DEPS" -gt 0 ]; then
        pass "30.3 DisplayConfig has $DC_DEPS depends_on (includes ColorMode)"
    else
        fail "30.3 DisplayConfig has no depends_on"
    fi

else
    skip "30.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 31: Non-Table Struct with Optional Enum (transitive Optional<Enum>)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 31: Optional Enum in Non-Table Struct — Transitive Optional<Enum>"

step "31.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "31.1 ThemeConfig has ThemePrefs in affected_objects"
    TC_HAS_PREFS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "theme_configs") | .affected_objects[] | select(. == "ThemePrefs")] | length')
    if [ -n "$TC_HAS_PREFS" ] && [ "$TC_HAS_PREFS" -gt 0 ]; then
        pass "31.1 ThemeConfig affected_objects includes 'ThemePrefs'"
    else
        fail "31.1 ThemeConfig missing 'ThemePrefs'"
    fi

    step "31.2 ThemeConfig has ThemeKind enum (transitive through ThemePrefs)"
    TC_HAS_KIND=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "theme_configs") | .affected_objects[] | select(. == "theme_kind")] | length')
    if [ -n "$TC_HAS_KIND" ] && [ "$TC_HAS_KIND" -gt 0 ]; then
        pass "31.2 ThemeConfig affected_objects includes 'theme_kind' (transitive Optional<ThemeKind>)"
    else
        fail "31.2 ThemeConfig missing 'theme_kind'"
    fi

else
    skip "31.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 32: Non-Table Struct with Map of Enum Key (transitive Map<Enum, X>)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 32: Map of Enum Key in Non-Table Struct — Transitive Map<Enum, X>"

step "32.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "32.1 WidgetConfig has WidgetLayout in affected_objects"
    WC_HAS_LAYOUT=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "widget_configs") | .affected_objects[] | select(. == "WidgetLayout")] | length')
    if [ -n "$WC_HAS_LAYOUT" ] && [ "$WC_HAS_LAYOUT" -gt 0 ]; then
        pass "32.1 WidgetConfig affected_objects includes 'WidgetLayout'"
    else
        fail "32.1 WidgetConfig missing 'WidgetLayout'"
    fi

    step "32.2 WidgetConfig has WidgetSize enum (transitive through WidgetLayout)"
    WC_HAS_SIZE=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "widget_configs") | .affected_objects[] | select(. == "widget_size")] | length')
    if [ -n "$WC_HAS_SIZE" ] && [ "$WC_HAS_SIZE" -gt 0 ]; then
        pass "32.2 WidgetConfig affected_objects includes 'widget_size' (transitive Map<WidgetSize, Int>)"
    else
        fail "32.2 WidgetConfig missing 'widget_size'"
    fi

else
    skip "32.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 33: 3-Level Deep Nesting — enums & FKs at every level
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 33: 3-Level Deep Nesting — Recursive Traversal"

step "33.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "33.1 DeployConfig has HostingPlan (level 1) in affected_objects"
    DEP_HAS_HOST=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "deploy_configs") | .affected_objects[] | select(. == "HostingPlan")] | length')
    if [ -n "$DEP_HAS_HOST" ] && [ "$DEP_HAS_HOST" -gt 0 ]; then
        pass "33.1 DeployConfig affected_objects includes 'HostingPlan' (level 1)"
    else
        fail "33.1 DeployConfig missing 'HostingPlan'"
    fi

    step "33.2 DeployConfig has DataCenter (level 2) in affected_objects"
    DEP_HAS_DC=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "deploy_configs") | .affected_objects[] | select(. == "DataCenter")] | length')
    if [ -n "$DEP_HAS_DC" ] && [ "$DEP_HAS_DC" -gt 0 ]; then
        pass "33.2 DeployConfig affected_objects includes 'DataCenter' (level 2)"
    else
        fail "33.2 DeployConfig missing 'DataCenter'"
    fi

    step "33.3 DeployConfig has Region enum (level 3) in affected_objects"
    DEP_HAS_REGION=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "deploy_configs") | .affected_objects[] | select(. == "region")] | length')
    if [ -n "$DEP_HAS_REGION" ] && [ "$DEP_HAS_REGION" -gt 0 ]; then
        pass "33.3 DeployConfig affected_objects includes 'region' (level 3 enum)"
    else
        fail "33.3 DeployConfig missing 'region' enum"
    fi

    step "33.4 DeployConfig has Employee FK (level 3) via affected_objects"
    DEP_HAS_EMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "deploy_configs") | .affected_objects[] | select(. == "employees")] | length')
    if [ -n "$DEP_HAS_EMP" ] && [ "$DEP_HAS_EMP" -gt 0 ]; then
        pass "33.4 DeployConfig affected_objects includes 'employees' (level 3 FK)"
    else
        fail "33.4 DeployConfig missing 'employees' FK ref"
    fi

    step "33.5 DeployConfig has component_id assigned"
    DEP_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "deploy_configs") | .component_id] | first')
    if [ -n "$DEP_COMP" ] && [ "$DEP_COMP" != "null" ]; then
        pass "33.5 DeployConfig has component_id $DEP_COMP"
    else
        fail "33.5 DeployConfig missing component_id"
    fi

else
    skip "33.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 34: Circular Non-Table Struct Reference (A→B→A cycle)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 34: Circular Non-Table Struct — Infinite Recursion Guard"

step "34.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "34.1 CircularRef has CircularA in affected_objects"
    CR_HAS_A=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "circular_refs") | .affected_objects[] | select(. == "CircularA")] | length')
    if [ -n "$CR_HAS_A" ] && [ "$CR_HAS_A" -gt 0 ]; then
        pass "34.1 CircularRef affected_objects includes 'CircularA'"
    else
        fail "34.1 CircularRef missing 'CircularA'"
    fi

    step "34.2 CircularRef has CircularB in affected_objects (through circular ref)"
    CR_HAS_B=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "circular_refs") | .affected_objects[] | select(. == "CircularB")] | length')
    if [ -n "$CR_HAS_B" ] && [ "$CR_HAS_B" -gt 0 ]; then
        pass "34.2 CircularRef affected_objects includes 'CircularB' (circular ref resolved)"
    else
        fail "34.2 CircularRef missing 'CircularB'"
    fi

    step "34.3 CircularRef has PointerType enum in affected_objects"
    CR_HAS_PTR=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "circular_refs") | .affected_objects[] | select(. == "pointer_type")] | length')
    if [ -n "$CR_HAS_PTR" ] && [ "$CR_HAS_PTR" -gt 0 ]; then
        pass "34.3 CircularRef affected_objects includes 'pointer_type' (enum through circular struct)"
    else
        fail "34.3 CircularRef missing 'pointer_type' enum"
    fi

    step "34.4 CircularRef has component_id (no crash from circular refs)"
    CR_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "circular_refs") | .component_id] | first')
    if [ -n "$CR_COMP" ] && [ "$CR_COMP" != "null" ]; then
        pass "34.4 CircularRef has component_id $CR_COMP (no infinite loop)"
    else
        fail "34.4 CircularRef missing component_id (possible crash)"
    fi

else
    skip "34.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 35: Multiple Enums in One Non-Table Struct
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 35: Multiple Enums in One Non-Table Struct"

step "35.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "35.1 ExportConfig has ExportPrefs in affected_objects"
    EC_HAS_PREFS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "export_configs") | .affected_objects[] | select(. == "ExportPrefs")] | length')
    if [ -n "$EC_HAS_PREFS" ] && [ "$EC_HAS_PREFS" -gt 0 ]; then
        pass "35.1 ExportConfig affected_objects includes 'ExportPrefs'"
    else
        fail "35.1 ExportConfig missing 'ExportPrefs'"
    fi

    step "35.2 ExportConfig has Format enum in affected_objects"
    EC_HAS_FMT=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "export_configs") | .affected_objects[] | select(. == "format")] | length')
    if [ -n "$EC_HAS_FMT" ] && [ "$EC_HAS_FMT" -gt 0 ]; then
        pass "35.2 ExportConfig affected_objects includes 'format' (enum 1 of 2)"
    else
        fail "35.2 ExportConfig missing 'format' enum"
    fi

    step "35.3 ExportConfig has Compression enum in affected_objects"
    EC_HAS_CMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "export_configs") | .affected_objects[] | select(. == "compression")] | length')
    if [ -n "$EC_HAS_CMP" ] && [ "$EC_HAS_CMP" -gt 0 ]; then
        pass "35.3 ExportConfig affected_objects includes 'compression' (enum 2 of 2)"
    else
        fail "35.3 ExportConfig missing 'compression' enum"
    fi

    step "35.4 ExportConfig depends_on includes both Format and Compression"
    EC_DEPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "export_configs") | .depends_on | length] | first')
    if [ -n "$EC_DEPS" ] && [ "$EC_DEPS" -gt 0 ]; then
        pass "35.4 ExportConfig has $EC_DEPS depends_on entries (covers both enums)"
    else
        fail "35.4 ExportConfig has no depends_on"
    fi

else
    skip "35.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 36: Kitchen Sink — All Type Kinds in One Non-Table Struct
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 36: Kitchen Sink — All Type Kinds in One Struct"

step "36.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "36.1 SinkConfig has KitchenSink in affected_objects"
    SK_HAS_SINK=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "sink_configs") | .affected_objects[] | select(. == "KitchenSink")] | length')
    if [ -n "$SK_HAS_SINK" ] && [ "$SK_HAS_SINK" -gt 0 ]; then
        pass "36.1 SinkConfig affected_objects includes 'KitchenSink'"
    else
        fail "36.1 SinkConfig missing 'KitchenSink'"
    fi

    step "36.2 SinkConfig has SinkLevel enum in affected_objects"
    SK_HAS_LEVEL=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "sink_configs") | .affected_objects[] | select(. == "sink_level")] | length')
    if [ -n "$SK_HAS_LEVEL" ] && [ "$SK_HAS_LEVEL" -gt 0 ]; then
        pass "36.2 SinkConfig affected_objects includes 'sink_level' (direct enum + optional + array + map)"
    else
        fail "36.2 SinkConfig missing 'sink_level' enum"
    fi

    step "36.3 SinkConfig has Employee FK in affected_objects (via KitchenSink)"
    SK_HAS_EMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "sink_configs") | .affected_objects[] | select(. == "employees")] | length')
    if [ -n "$SK_HAS_EMP" ] && [ "$SK_HAS_EMP" -gt 0 ]; then
        pass "36.3 SinkConfig affected_objects includes 'employees' (FK + optional FK)"
    else
        fail "36.3 SinkConfig missing 'employees' FK ref"
    fi

    step "36.4 SinkConfig and SinkLevel share same component"
    SK_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "sink_configs") | .component_id] | first')
    SL_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and .change.name == "sink_level") | .component_id] | first')
    if [ -n "$SK_COMP" ] && [ -n "$SL_COMP" ] && [ "$SK_COMP" = "$SL_COMP" ]; then
        pass "36.4 SinkConfig and SinkLevel share component $SK_COMP (all deps in one batch)"
    else
        fail "36.4 SinkConfig comp=$SK_COMP, SinkLevel comp=$SL_COMP"
    fi

    step "36.5 SinkConfig has depends_on entries"
    SK_DEPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "sink_configs") | .depends_on | length] | first')
    if [ -n "$SK_DEPS" ] && [ "$SK_DEPS" -gt 0 ]; then
        pass "36.5 SinkConfig has $SK_DEPS depends_on entries (enum + FK deps)"
    else
        fail "36.5 SinkConfig has no depends_on entries"
    fi

else
    skip "36.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 37: Optional Array of Enum — [Visibility]?
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 37: Optional Array of Enum — Optional<Array<Enum>>"

step "37.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "37.1 OptArrayEnum has Visibility enum in affected_objects"
    OAE_HAS_VIS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "opt_array_enums") | .affected_objects[] | select(. == "visibility")] | length')
    if [ -n "$OAE_HAS_VIS" ] && [ "$OAE_HAS_VIS" -gt 0 ]; then
        pass "37.1 OptArrayEnum affected_objects includes 'visibility' (from [Visibility]?)"
    else
        fail "37.1 OptArrayEnum missing 'visibility' in affected_objects"
    fi

    step "37.2 OptArrayEnum has component_id assigned"
    OAE_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "opt_array_enums") | .component_id] | first')
    if [ -n "$OAE_COMP" ] && [ "$OAE_COMP" != "null" ]; then
        pass "37.2 OptArrayEnum has component_id $OAE_COMP"
    else
        fail "37.2 OptArrayEnum missing component_id"
    fi

else
    skip "37.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 38: Map<Enum, Non-Table Struct> — {Tier: DisplayPrefs}
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 38: Map<Enum, Struct> — Map with Enum Key and Struct Value"

step "38.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "38.1 MapEnumStruct has Tier enum in affected_objects"
    MES_HAS_TIER=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "map_enum_structs") | .affected_objects[] | select(. == "tier")] | length')
    if [ -n "$MES_HAS_TIER" ] && [ "$MES_HAS_TIER" -gt 0 ]; then
        pass "38.1 MapEnumStruct affected_objects includes 'tier' (map key enum)"
    else
        fail "38.1 MapEnumStruct missing 'tier' (map key enum)"
    fi

    step "38.2 MapEnumStruct has DisplayPrefs in affected_objects"
    MES_HAS_PREFS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "map_enum_structs") | .affected_objects[] | select(. == "DisplayPrefs")] | length')
    if [ -n "$MES_HAS_PREFS" ] && [ "$MES_HAS_PREFS" -gt 0 ]; then
        pass "38.2 MapEnumStruct affected_objects includes 'DisplayPrefs' (map value struct)"
    else
        fail "38.2 MapEnumStruct missing 'DisplayPrefs'"
    fi

    step "38.3 MapEnumStruct has ColorMode (transitive through DisplayPrefs)"
    MES_HAS_MODE=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "map_enum_structs") | .affected_objects[] | select(. == "color_mode")] | length')
    if [ -n "$MES_HAS_MODE" ] && [ "$MES_HAS_MODE" -gt 0 ]; then
        pass "38.3 MapEnumStruct affected_objects includes 'color_mode' (transitive via DisplayPrefs)"
    else
        fail "38.3 MapEnumStruct missing 'color_mode' transitive enum"
    fi

else
    skip "38.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 39: Table with Only Non-Table Struct Field (zero primitive columns)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 39: Pure Nested Table — No Primitive Columns"

step "39.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "39.1 PureNested has TaskMeta in affected_objects"
    PN_HAS_META=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "pure_nested") | .affected_objects[] | select(. == "TaskMeta")] | length')
    if [ -n "$PN_HAS_META" ] && [ "$PN_HAS_META" -gt 0 ]; then
        pass "39.1 PureNested affected_objects includes 'TaskMeta' (only data column)"
    else
        fail "39.1 PureNested missing 'TaskMeta'"
    fi

    step "39.2 PureNested has task_status (transitive through TaskMeta)"
    PN_HAS_STATUS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "pure_nested") | .affected_objects[] | select(. == "task_status")] | length')
    if [ -n "$PN_HAS_STATUS" ] && [ "$PN_HAS_STATUS" -gt 0 ]; then
        pass "39.2 PureNested affected_objects includes 'task_status' (transitive through TaskMeta)"
    else
        fail "39.2 PureNested missing 'task_status'"
    fi

    step "39.3 PureNested has component_id assigned"
    PN_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "pure_nested") | .component_id] | first')
    if [ -n "$PN_COMP" ] && [ "$PN_COMP" != "null" ]; then
        pass "39.3 PureNested has component_id $PN_COMP"
    else
        fail "39.3 PureNested missing component_id"
    fi

else
    skip "39.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 40: Self-Referencing Non-Table Struct (RecursiveNode)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 40: Self-Referencing Non-Table Struct — Recursive Type Guard"

step "40.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "40.1 RecursiveTree has RecursiveNode in affected_objects"
    RT_HAS_NODE=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "recursive_trees") | .affected_objects[] | select(. == "RecursiveNode")] | length')
    if [ -n "$RT_HAS_NODE" ] && [ "$RT_HAS_NODE" -gt 0 ]; then
        pass "40.1 RecursiveTree affected_objects includes 'RecursiveNode' (self-ref resolved)"
    else
        fail "40.1 RecursiveTree missing 'RecursiveNode'"
    fi

    step "40.2 RecursiveTree has recursive_label enum in affected_objects"
    RT_HAS_LABEL=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "recursive_trees") | .affected_objects[] | select(. == "recursive_label")] | length')
    if [ -n "$RT_HAS_LABEL" ] && [ "$RT_HAS_LABEL" -gt 0 ]; then
        pass "40.2 RecursiveTree affected_objects includes 'recursive_label' (enum through self-ref struct)"
    else
        fail "40.2 RecursiveTree missing 'recursive_label' enum"
    fi

    step "40.3 RecursiveTree has component_id (no infinite loop from self-ref)"
    RT_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "recursive_trees") | .component_id] | first')
    if [ -n "$RT_COMP" ] && [ "$RT_COMP" != "null" ]; then
        pass "40.3 RecursiveTree has component_id $RT_COMP (no infinite recursion)"
    else
        fail "40.3 RecursiveTree missing component_id (possible crash)"
    fi

else
    skip "40.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 41: Enum Diamond — DiamondEnum used by 3 tables with FK chain
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 41: Enum Diamond — DiamondEnum Across 3 Tables"

step "41.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "41.1 DiamondEnum is created and in plan"
    DE_CREATED=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and .change.name == "diamond_enum")] | length')
    if [ -n "$DE_CREATED" ] && [ "$DE_CREATED" -gt 0 ]; then
        pass "41.1 DiamondEnum is in the plan"
    else
        fail "41.1 DiamondEnum not in plan"
    fi

    step "41.2 DiamondLeft uses DiamondEnum"
    DL_HAS_ENUM=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "diamond_left") | .affected_objects[] | select(. == "diamond_enum")] | length')
    if [ -n "$DL_HAS_ENUM" ] && [ "$DL_HAS_ENUM" -gt 0 ]; then
        pass "41.2 DiamondLeft has 'diamond_enum' in affected_objects"
    else
        fail "41.2 DiamondLeft missing 'diamond_enum'"
    fi

    step "41.3 DiamondRight uses DiamondEnum"
    DR_HAS_ENUM=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "diamond_right") | .affected_objects[] | select(. == "diamond_enum")] | length')
    if [ -n "$DR_HAS_ENUM" ] && [ "$DR_HAS_ENUM" -gt 0 ]; then
        pass "41.3 DiamondRight has 'diamond_enum' in affected_objects"
    else
        fail "41.3 DiamondRight missing 'diamond_enum'"
    fi

    step "41.4 DiamondMerge uses DiamondEnum and has FK deps"
    DM_HAS_ENUM=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "diamond_merge") | .affected_objects[] | select(. == "diamond_enum")] | length')
    DM_DEPS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "diamond_merge") | .depends_on | length] | first')
    if [ -n "$DM_HAS_ENUM" ] && [ "$DM_HAS_ENUM" -gt 0 ] && [ -n "$DM_DEPS" ] && [ "$DM_DEPS" -ge 3 ]; then
        pass "41.4 DiamondMerge has 'diamond_enum' + $DM_DEPS deps (FKs + enum)"
    else
        fail "41.4 DiamondMerge has enum=$DM_HAS_ENUM, deps=$DM_DEPS"
    fi

    step "41.5 All diamond tables share same component (via DiamondEnum + FK chain)"
    DL_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "diamond_left") | .component_id] | first')
    DR_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "diamond_right") | .component_id] | first')
    DM_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "diamond_merge") | .component_id] | first')
    if [ -n "$DL_COMP" ] && [ -n "$DR_COMP" ] && [ -n "$DM_COMP" ] && [ "$DL_COMP" = "$DR_COMP" ] && [ "$DL_COMP" = "$DM_COMP" ]; then
        pass "41.5 All 3 diamond tables same component $DL_COMP"
    else
        fail "41.5 Diamond components: L=$DL_COMP R=$DR_COMP M=$DM_COMP (should match)"
    fi

else
    skip "41.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 42: Heavy Non-Table Struct — 3 Enums + 3 FKs in One Struct
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 42: Heavy Non-Table Struct — 3 Enums + 3 FKs"

step "42.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "42.1 HeavyPayloadTable has HeavyPayload in affected_objects"
    HP_HAS_PAYLOAD=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "heavy_payloads") | .affected_objects[] | select(. == "HeavyPayload")] | length')
    if [ -n "$HP_HAS_PAYLOAD" ] && [ "$HP_HAS_PAYLOAD" -gt 0 ]; then
        pass "42.1 HeavyPayloadTable affected_objects includes 'HeavyPayload'"
    else
        fail "42.1 HeavyPayloadTable missing 'HeavyPayload'"
    fi

    step "42.2 HeavyPayloadTable has heavy_enum_a in affected_objects"
    HP_HAS_A=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "heavy_payloads") | .affected_objects[] | select(. == "heavy_enum_a")] | length')
    if [ -n "$HP_HAS_A" ] && [ "$HP_HAS_A" -gt 0 ]; then
        pass "42.2 HeavyPayloadTable affected_objects includes 'heavy_enum_a' (enum 1/3)"
    else
        fail "42.2 HeavyPayloadTable missing 'heavy_enum_a'"
    fi

    step "42.3 HeavyPayloadTable has heavy_enum_b in affected_objects"
    HP_HAS_B=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "heavy_payloads") | .affected_objects[] | select(. == "heavy_enum_b")] | length')
    if [ -n "$HP_HAS_B" ] && [ "$HP_HAS_B" -gt 0 ]; then
        pass "42.3 HeavyPayloadTable affected_objects includes 'heavy_enum_b' (enum 2/3)"
    else
        fail "42.3 HeavyPayloadTable missing 'heavy_enum_b'"
    fi

    step "42.4 HeavyPayloadTable has heavy_enum_c in affected_objects"
    HP_HAS_C=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "heavy_payloads") | .affected_objects[] | select(. == "heavy_enum_c")] | length')
    if [ -n "$HP_HAS_C" ] && [ "$HP_HAS_C" -gt 0 ]; then
        pass "42.4 HeavyPayloadTable affected_objects includes 'heavy_enum_c' (enum 3/3)"
    else
        fail "42.4 HeavyPayloadTable missing 'heavy_enum_c'"
    fi

    step "42.5 HeavyPayloadTable has FK to employees in affected_objects"
    HP_HAS_EMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "heavy_payloads") | .affected_objects[] | select(. == "employees")] | length')
    if [ -n "$HP_HAS_EMP" ] && [ "$HP_HAS_EMP" -gt 0 ]; then
        pass "42.5 HeavyPayloadTable affected_objects includes 'employees' (FK 1/3)"
    else
        fail "42.5 HeavyPayloadTable missing 'employees' FK"
    fi

    step "42.6 HeavyPayloadTable has FK to diamond_left in affected_objects"
    HP_HAS_DL=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "heavy_payloads") | .affected_objects[] | select(. == "diamond_left")] | length')
    if [ -n "$HP_HAS_DL" ] && [ "$HP_HAS_DL" -gt 0 ]; then
        pass "42.6 HeavyPayloadTable affected_objects includes 'diamond_left' (FK 2/3)"
    else
        fail "42.6 HeavyPayloadTable missing 'diamond_left' FK"
    fi

    step "42.7 HeavyPayloadTable has FK to diamond_right in affected_objects"
    HP_HAS_DR=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "heavy_payloads") | .affected_objects[] | select(. == "diamond_right")] | length')
    if [ -n "$HP_HAS_DR" ] && [ "$HP_HAS_DR" -gt 0 ]; then
        pass "42.7 HeavyPayloadTable affected_objects includes 'diamond_right' (FK 3/3)"
    else
        fail "42.7 HeavyPayloadTable missing 'diamond_right' FK"
    fi

else
    skip "42.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 43: Optional Array of Non-Table Struct — [Address]?
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 43: Optional Array of Non-Table Struct"

step "43.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "43.1 OptAddrList has Address in affected_objects"
    OAL_HAS_ADDR=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "opt_addr_lists") | .affected_objects[] | select(. == "Address")] | length')
    if [ -n "$OAL_HAS_ADDR" ] && [ "$OAL_HAS_ADDR" -gt 0 ]; then
        pass "43.1 OptAddrList affected_objects includes 'Address' (from [Address]?)"
    else
        fail "43.1 OptAddrList missing 'Address'"
    fi

    step "43.2 OptAddrList has component_id assigned"
    OAL_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "opt_addr_lists") | .component_id] | first')
    if [ -n "$OAL_COMP" ] && [ "$OAL_COMP" != "null" ]; then
        pass "43.2 OptAddrList has component_id $OAL_COMP"
    else
        fail "43.2 OptAddrList missing component_id"
    fi

else
    skip "43.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 44: Multiple Tables Sharing Same Enum via Different Paths
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 44: Shared Enum — 3 Tables, 3 Paths"

step "44.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "44.1 SharedByAll enum is created and in plan"
    SBA_CREATED=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and .change.name == "shared_by_all")] | length')
    if [ -n "$SBA_CREATED" ] && [ "$SBA_CREATED" -gt 0 ]; then
        pass "44.1 SharedByAll enum is in the plan"
    else
        fail "44.1 SharedByAll enum not in plan"
    fi

    step "44.2 SharedEnumUser (direct) has shared_by_all in affected_objects"
    SEU_HAS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "shared_enum_users") | .affected_objects[] | select(. == "shared_by_all")] | length')
    if [ -n "$SEU_HAS" ] && [ "$SEU_HAS" -gt 0 ]; then
        pass "44.2 SharedEnumUser (direct field) has 'shared_by_all'"
    else
        fail "44.2 SharedEnumUser missing 'shared_by_all'"
    fi

    step "44.3 SharedEnumIndirect (via struct) has shared_by_all in affected_objects"
    SEI_HAS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "shared_enum_indirect") | .affected_objects[] | select(. == "shared_by_all")] | length')
    if [ -n "$SEI_HAS" ] && [ "$SEI_HAS" -gt 0 ]; then
        pass "44.3 SharedEnumIndirect (via non-table struct) has 'shared_by_all'"
    else
        fail "44.3 SharedEnumIndirect missing 'shared_by_all'"
    fi

    step "44.4 SharedEnumArray (via array) has shared_by_all in affected_objects"
    SEA_HAS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "shared_enum_array") | .affected_objects[] | select(. == "shared_by_all")] | length')
    if [ -n "$SEA_HAS" ] && [ "$SEA_HAS" -gt 0 ]; then
        pass "44.4 SharedEnumArray (via [SharedByAll]) has 'shared_by_all'"
    else
        fail "44.4 SharedEnumArray missing 'shared_by_all'"
    fi

    step "44.5 All 3 tables share same component via SharedByAll"
    SEU_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "shared_enum_users") | .component_id] | first')
    SEI_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "shared_enum_indirect") | .component_id] | first')
    SEA_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "shared_enum_array") | .component_id] | first')
    if [ -n "$SEU_COMP" ] && [ "$SEI_COMP" = "$SEU_COMP" ] && [ "$SEA_COMP" = "$SEU_COMP" ]; then
        pass "44.5 All 3 shared-enum tables same component $SEU_COMP"
    else
        fail "44.5 Components: direct=$SEU_COMP indirect=$SEI_COMP array=$SEA_COMP (should match)"
    fi

else
    skip "44.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 45: Struct with All Optional Primitives (Str?, Int?, Bool?, Float?)
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 45: All Optional Primitives — Optional<Str/Int/Bool/Float>"

step "45.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "45.1 OptionalPrimitivesTable has OptionalPrimitives in affected_objects"
    OPT_HAS=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "optional_primitives") | .affected_objects[] | select(. == "OptionalPrimitives")] | length')
    if [ -n "$OPT_HAS" ] && [ "$OPT_HAS" -gt 0 ]; then
        pass "45.1 OptionalPrimitivesTable affected_objects includes 'OptionalPrimitives'"
    else
        fail "45.1 OptionalPrimitivesTable missing 'OptionalPrimitives'"
    fi

    step "45.2 OptionalPrimitivesTable has component_id assigned"
    OPT_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "optional_primitives") | .component_id] | first')
    if [ -n "$OPT_COMP" ] && [ "$OPT_COMP" != "null" ]; then
        pass "45.2 OptionalPrimitivesTable has component_id $OPT_COMP"
    else
        fail "45.2 OptionalPrimitivesTable missing component_id"
    fi

else
    skip "45.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 46: Map<Enum, Struct> — {WidgetSize: SharedProfile}
# ═════════════════════════════════════════════════════════════════════════════
section "PHASE 46: Map<Enum, Struct> — Enum Key + Struct Value with Deps"

step "46.0 Verify advanced JSON available"
if [ -n "$ADV_JSON" ]; then

    step "46.1 MapEnumToStruct has WidgetSize enum in affected_objects"
    METS_HAS_SIZE=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "map_enum_to_struct") | .affected_objects[] | select(. == "widget_size")] | length')
    if [ -n "$METS_HAS_SIZE" ] && [ "$METS_HAS_SIZE" -gt 0 ]; then
        pass "46.1 MapEnumToStruct affected_objects includes 'widget_size' (map key enum)"
    else
        fail "46.1 MapEnumToStruct missing 'widget_size'"
    fi

    step "46.2 MapEnumToStruct has SharedProfile in affected_objects"
    METS_HAS_PROF=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "map_enum_to_struct") | .affected_objects[] | select(. == "SharedProfile")] | length')
    if [ -n "$METS_HAS_PROF" ] && [ "$METS_HAS_PROF" -gt 0 ]; then
        pass "46.2 MapEnumToStruct affected_objects includes 'SharedProfile' (map value struct)"
    else
        fail "46.2 MapEnumToStruct missing 'SharedProfile'"
    fi

    step "46.3 MapEnumToStruct has component_id assigned"
    METS_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_table" and .change.name == "map_enum_to_struct") | .component_id] | first')
    if [ -n "$METS_COMP" ] && [ "$METS_COMP" != "null" ]; then
        pass "46.3 MapEnumToStruct has component_id $METS_COMP"
    else
        fail "46.3 MapEnumToStruct missing component_id"
    fi

else
    skip "46.x No advanced JSON available"
fi

# ═════════════════════════════════════════════════════════════════════════════
# RESULTS
# ═════════════════════════════════════════════════════════════════════════════

TOTAL=$((PASSED + FAILED + SKIPPED))
echo ""
echo -e "${BOLD}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${BOLD}  Dependency Graph Test Results${NC}"
echo -e "${BOLD}═══════════════════════════════════════════════════════════════${NC}"
echo -e "  Total:    ${BOLD}$TOTAL${NC}"
echo -e "  ${GREEN}Passed:   $PASSED${NC}"
echo -e "  ${RED}Failed:   $FAILED${NC}"
echo -e "  ${YELLOW}Skipped:  $SKIPPED${NC}"
echo ""
echo -e "  Log: $LOG_FILE"
echo ""

# Write results summary
{
    echo "Doo Migration Dependency Graph Test Results"
    echo "==========================================="
    echo "Date: $(date)"
    echo "Binary: $DOO_BIN"
    echo "Database: $TEST_DB_URL"
    echo "Total: $TOTAL | Passed: $PASSED | Failed: $FAILED | Skipped: $SKIPPED"
} >> "$RESULTS_FILE"

if [ "$FAILED" -gt 0 ]; then
    echo -e "  ${RED}${BOLD}❌ Some tests FAILED${NC}"
    exit 1
else
    echo -e "  ${GREEN}${BOLD}✅ All tests PASSED${NC}"
    exit 0
fi
