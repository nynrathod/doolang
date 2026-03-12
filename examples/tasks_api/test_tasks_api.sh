#!/bin/bash
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../tests/dev_test/common.sh"

PORT=3000
TS=$(date +%s)
EMAIL="test${TS}@tasks.com"

cd "$SCRIPT_DIR"
kill_port $PORT
start_server main.doo $PORT || exit 1
setup_trap

# --- Test 1: Signup ---
echo ""
echo "Test 1: Signup"
RESPONSE=$(http_post "/api/auth/signup" "{\"Email\":\"$EMAIL\",\"Password\":\"password123\",\"Name\":\"Test User\",\"Role\":\"user\"}")
assert_status "$RESPONSE" 200 "signup"
assert_json_not_has "$RESPONSE" "Password" "password hidden"

# --- Test 2: Login ---
echo ""
echo "Test 2: Login"
RESPONSE=$(http_post "/api/auth/login" "{\"Email\":\"$EMAIL\",\"Password\":\"password123\"}")
assert_status "$RESPONSE" 200 "login"
assert_json_exists "$RESPONSE" ".data.token" "has token"
TOKEN=$(extract_json "$RESPONSE" ".data.token")
AUTH="Authorization: Bearer $TOKEN"

# --- Test 3: Create Task ---
echo ""
echo "Test 3: Create Task"
RESPONSE=$(http_post "/tasks" '{"Title":"Test Task","Priority":"High","Status":"Todo"}' "$AUTH")
assert_status "$RESPONSE" 200 "create task"
assert_json_exists "$RESPONSE" ".data.id" "task has id"
TASK_ID=$(extract_json "$RESPONSE" ".data.id")

# --- Test 4: Create Second Task (Done) ---
echo ""
echo "Test 4: Create Second Task (Done)"
RESPONSE=$(http_post "/tasks" '{"Title":"Completed Task","Priority":"Low","Status":"Done"}' "$AUTH")
assert_status "$RESPONSE" 200 "create done task"

# --- Test 4b: Create Urgent Task ---
echo ""
echo "Test 4b: Create Urgent Task"
RESPONSE=$(http_post "/tasks" '{"Title":"Urgent Task","Priority":"High","Status":"Todo"}' "$AUTH")
assert_status "$RESPONSE" 200 "create urgent task"

# --- Test 5: List Tasks ---
echo ""
echo "Test 5: List Tasks"
RESPONSE=$(http_get "/tasks" "$AUTH")
assert_status "$RESPONSE" 200 "list tasks"
assert_json_type "$RESPONSE" ".data" "array" "data is array"

# --- Test 6: Get Task by ID ---
echo ""
echo "Test 6: Get Task by ID"
RESPONSE=$(http_get "/tasks/$TASK_ID" "$AUTH")
assert_status "$RESPONSE" 200 "get task"

# --- Test 7: Update Task ---
echo ""
echo "Test 7: Update Task"
RESPONSE=$(http_put "/tasks/$TASK_ID" '{"Title":"Updated Task","Priority":"Medium","Status":"InProgress"}' "$AUTH")
assert_status "$RESPONSE" 200 "update task"

# --- Test 8: Get Done Tasks ---
echo ""
echo "Test 8: Get Done Tasks"
RESPONSE=$(http_get "/tasks/done" "$AUTH")
assert_status "$RESPONSE" 200 "done tasks"

# --- Test 9: Get Urgent Tasks ---
echo ""
echo "Test 9: Get Urgent Tasks"
RESPONSE=$(http_get "/tasks/urgent" "$AUTH")
assert_status "$RESPONSE" 200 "urgent tasks"

# --- Test 10: Get Pending Tasks ---
echo ""
echo "Test 10: Get Pending Tasks"
RESPONSE=$(http_get "/tasks/pending" "$AUTH")
assert_status "$RESPONSE" 200 "pending tasks"

# --- Test 11: Get Stats ---
echo ""
echo "Test 11: Get Stats"
RESPONSE=$(http_get "/tasks/stats" "$AUTH")
assert_status "$RESPONSE" 200 "stats"

# --- Test 12: Delete Task ---
echo ""
echo "Test 12: Delete Task"
RESPONSE=$(http_delete "/tasks/$TASK_ID" "$AUTH")
assert_status "$RESPONSE" 200 "delete task"

# --- Test 13: List Tasks After Delete ---
echo ""
echo "Test 13: List Tasks After Delete"
RESPONSE=$(http_get "/tasks" "$AUTH")
assert_status "$RESPONSE" 200 "list after delete"

print_http_summary
