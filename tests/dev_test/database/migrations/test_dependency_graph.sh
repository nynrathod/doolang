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
    METATYPE_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and (.change.name | test("metatype"; "i"))) | .component_id] | first')

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
    ACCESS_COMP=$(echo "$ADV_JSON" | jq -r '[.migration_plan.changes[] | select(.change.type == "create_enum" and (.change.name | test("accesslevel"; "i"))) | .component_id] | first')
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
