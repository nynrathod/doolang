#!/bin/bash
# =============================================================================
# Tasks API Test Script
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../tests/dev_test/common.sh"

PORT=3000
BASE="http://127.0.0.1:$PORT"
TS=$(date +%s)
EMAIL="test${TS}@tasks.com"

CURL_MAX_TIME="${CURL_MAX_TIME:-8}"
CURL_CONNECT_TIMEOUT="${CURL_CONNECT_TIMEOUT:-2}"

req() { curl -s --max-time "$CURL_MAX_TIME" --connect-timeout "$CURL_CONNECT_TIMEOUT" "$@"; }
show() { echo "$1" | "$SCRIPT_DIR/../../tests/dev_test/pretty.sh"; echo ""; }

kill_port $PORT
cd "$SCRIPT_DIR"
start_server main.doo $PORT || exit 1
setup_trap

echo ""

# Test 1: Signup
echo "═══════════════════════════════════════════════════════════════"
echo "Test 1: Signup"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req -X POST "$BASE/api/auth/signup" -H "Content-Type: application/json" \
    -d "{\"Email\":\"$EMAIL\",\"Password\":\"password123\",\"Name\":\"Test User\",\"Role\":\"user\"}")
show "$RESP"

# Test 2: Login
echo "═══════════════════════════════════════════════════════════════"
echo "Test 2: Login"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req -X POST "$BASE/api/auth/login" -H "Content-Type: application/json" \
    -d "{\"Email\":\"$EMAIL\",\"Password\":\"password123\"}")
show "$RESP"
TOKEN=$(echo "$RESP" | jq -r '.data.token // empty' 2>/dev/null)
[ -z "$TOKEN" ] && TOKEN=$(echo "$RESP" | grep -o '"token":"[^"]*' | sed 's/"token":"//')
AUTH="Authorization: Bearer $TOKEN"

# Test 3: Create Task
echo "═══════════════════════════════════════════════════════════════"
echo "Test 3: Create Task"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req -X POST "$BASE/tasks" -H "Content-Type: application/json" -H "$AUTH" \
    -d '{"Title":"Test Task","Priority":"High","Status":"Todo"}')
show "$RESP"
TASK_ID=$(echo "$RESP" | jq -r '.data.id // empty' 2>/dev/null)
[ -z "$TASK_ID" ] && TASK_ID=$(echo "$RESP" | grep -o '"id":[0-9]*' | head -1 | sed 's/"id"://')

# Test 4: Create Second Task (Done)
echo "═══════════════════════════════════════════════════════════════"
echo "Test 4: Create Second Task (Done)"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req -X POST "$BASE/tasks" -H "Content-Type: application/json" -H "$AUTH" \
    -d '{"Title":"Completed Task","Priority":"Low","Status":"Done"}')
show "$RESP"

# Test 4b: Create Urgent Task (High priority, not done)
echo "═══════════════════════════════════════════════════════════════"
echo "Test 4b: Create Urgent Task"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req -X POST "$BASE/tasks" -H "Content-Type: application/json" -H "$AUTH" \
    -d '{"Title":"Urgent Task","Priority":"High","Status":"Todo"}')
show "$RESP"

# Test 5: List Tasks
echo "═══════════════════════════════════════════════════════════════"
echo "Test 5: List Tasks"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req "$BASE/tasks" -H "$AUTH")
show "$RESP"

# Test 6: Get Task by ID
echo "═══════════════════════════════════════════════════════════════"
echo "Test 6: Get Task by ID"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req "$BASE/tasks/$TASK_ID" -H "$AUTH")
show "$RESP"

# Test 7: Update Task
echo "═══════════════════════════════════════════════════════════════"
echo "Test 7: Update Task"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req -X PUT "$BASE/tasks/$TASK_ID" -H "Content-Type: application/json" -H "$AUTH" \
    -d '{"Title":"Updated Task","Priority":"Medium","Status":"InProgress"}')
show "$RESP"

# Test 8: Get Done Tasks
echo "═══════════════════════════════════════════════════════════════"
echo "Test 8: Get Done Tasks"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req "$BASE/tasks/done" -H "$AUTH")
show "$RESP"

# Test 9: Get Urgent Tasks
echo "═══════════════════════════════════════════════════════════════"
echo "Test 9: Get Urgent Tasks"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req "$BASE/tasks/urgent" -H "$AUTH")
show "$RESP"

# Test 10: Get Pending Tasks
echo "═══════════════════════════════════════════════════════════════"
echo "Test 10: Get Pending Tasks"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req "$BASE/tasks/pending" -H "$AUTH")
show "$RESP"

# Test 11: Get Stats
echo "═══════════════════════════════════════════════════════════════"
echo "Test 11: Get Stats"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req "$BASE/tasks/stats" -H "$AUTH")
show "$RESP"

# Test 12: Delete Task
echo "═══════════════════════════════════════════════════════════════"
echo "Test 12: Delete Task"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req -X DELETE "$BASE/tasks/$TASK_ID" -H "$AUTH")
show "$RESP"

# Test 13: List Tasks After Delete
echo "═══════════════════════════════════════════════════════════════"
echo "Test 13: List Tasks After Delete"
echo "═══════════════════════════════════════════════════════════════"
RESP=$(req "$BASE/tasks" -H "$AUTH")
show "$RESP"

cleanup_server
rm -f server.log
echo "✅ All Tasks API tests completed!"
exit 0
