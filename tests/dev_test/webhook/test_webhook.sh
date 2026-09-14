#!/bin/bash
set -e

# =============================================================================
# WEBHOOK CRUD TEST
# Tests that crudWithWebhooks properly:
# 1. Registers CRUD routes
# 2. Creates/reads/updates/deletes records via DB
# 3. Fires webhooks on matching events (fire-and-forget)
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3105
ECHO_PORT=9999
FILE="main.doo"

echo "========================================="
echo "  WEBHOOK CRUD TEST"
echo "========================================="
echo ""

# --- Start a simple echo server to receive webhooks ---
echo "Starting webhook echo server on port $ECHO_PORT..."
# Use Python's built-in HTTP server as a simple echo receiver
# We'll capture its output to verify webhooks arrive
if command -v python3 &>/dev/null; then
    python3 -c "
import http.server
import sys

class WebhookHandler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        content_length = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(content_length).decode('utf-8')
        print(f'[WEBHOOK_RECEIVED] {self.path} | Body: {body}', flush=True)
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'OK')
    def log_message(self, format, *args):
        pass  # Suppress default logging

server = http.server.HTTPServer(('0.0.0.0', $ECHO_PORT), WebhookHandler)
print(f'Echo server listening on port $ECHO_PORT...', flush=True)
server.serve_forever()
" > webhook_echo.log 2>&1 &
    ECHO_PID=$!
    echo "Echo server started (PID: $ECHO_PID)"
elif command -v nc &>/dev/null; then
    # Fallback: use netcat
    while true; do echo -e "HTTP/1.1 200 OK\r\n\r\nOK" | nc -l -p $ECHO_PORT -q 1; done > webhook_echo.log 2>&1 &
    ECHO_PID=$!
    echo "Echo server started via nc (PID: $ECHO_PID)"
else
    echo "⚠️  No echo server available — webhook firing will be verified via server logs only."
    ECHO_PID=""
fi

# --- Start Doo server ---
echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

# --- Wait for both servers to be ready ---
sleep 2
echo ""

# =============================================================================
# Test 1: Health check
# =============================================================================
echo "Test 1: Health check (GET /ping)"
RESPONSE=$(http_get "/ping")
assert_status "$RESPONSE" 200 "GET /ping"
echo "  PASS"
echo ""

# =============================================================================
# Test 2: Create product — should fire "created" webhook
# =============================================================================
echo "Test 2: Create Product (should fire 'created' webhook)"
RESPONSE=$(http_post "/products" '{"Name":"Gaming Laptop","Price":999,"Category":"Electronics"}')
assert_status "$RESPONSE" 200 "create product"
assert_json_exists "$RESPONSE" ".data.id" "product has id"
assert_json "$RESPONSE" ".data.Name" "Gaming Laptop" "product name"
assert_json "$RESPONSE" ".data.Price" "999" "product price"
PRODUCT_ID=$(extract_json "$RESPONSE" ".data.id")
echo "  Created product ID: $PRODUCT_ID"
echo "  PASS"
echo ""

# =============================================================================
# Test 3: List products
# =============================================================================
echo "Test 3: List Products"
RESPONSE=$(http_get "/products")
assert_status "$RESPONSE" 200 "list products"
assert_json_type "$RESPONSE" ".data" "array" "data is array"
echo "  PASS"
echo ""

# =============================================================================
# Test 4: Get product by ID
# =============================================================================
echo "Test 4: Get Product by ID"
RESPONSE=$(http_get "/products/$PRODUCT_ID")
assert_status "$RESPONSE" 200 "get product"
assert_json "$RESPONSE" ".data.Name" "Gaming Laptop" "product name"
echo "  PASS"
echo ""

# =============================================================================
# Test 5: Update product — should fire "updated" webhook (Category=Electronics matches filter)
# =============================================================================
echo "Test 5: Update Product (should fire 'updated' webhook)"
RESPONSE=$(http_put "/products/$PRODUCT_ID" '{"Name":"Gaming Laptop Pro","Price":1299,"Category":"Electronics"}')
assert_status "$RESPONSE" 200 "update product"
assert_json "$RESPONSE" ".data.Name" "Gaming Laptop Pro" "updated name"
assert_json "$RESPONSE" ".data.Price" "1299" "updated price"
echo "  PASS"
echo ""

# =============================================================================
# Test 6: Update product — should NOT fire "updated" webhook (Category!=Electronics, filter doesn't match)
# =============================================================================
echo "Test 6: Update Product (should NOT fire 'updated' webhook — filter doesn't match)"
RESPONSE=$(http_put "/products/$PRODUCT_ID" '{"Name":"Gaming Laptop Pro","Price":1299,"Category":"Books"}')
assert_status "$RESPONSE" 200 "update product (no webhook)"
assert_json "$RESPONSE" ".data.Name" "Gaming Laptop Pro" "name unchanged"
echo "  PASS"
echo ""

# =============================================================================
# Test 7: Delete product — should fire "deleted" webhook (Price 1299 > 500 matches filter)
# =============================================================================
echo "Test 7: Delete Product (should fire 'deleted' webhook)"
RESPONSE=$(http_delete "/products/$PRODUCT_ID")
assert_status "$RESPONSE" 200 "delete product"
assert_json "$RESPONSE" ".data.deleted" "true" "deleted flag"
echo "  PASS"
echo ""

# =============================================================================
# Test 8: Create product with low price — should fire "created", but NOT "deleted" webhook
# =============================================================================
echo "Test 8: Create low-price Product (fires 'created' webhook)"
RESPONSE=$(http_post "/products" '{"Name":"Cheap Item","Price":10,"Category":"Misc"}')
assert_status "$RESPONSE" 200 "create cheap product"
LOW_ID=$(extract_json "$RESPONSE" ".data.id")
echo "  Created product ID: $LOW_ID"
echo "  PASS"
echo ""

# =============================================================================
# Test 9: Delete low-price product — should NOT fire "deleted" webhook (Price 10 < 500)
# =============================================================================
echo "Test 9: Delete low-price Product (should NOT fire 'deleted' webhook — filter doesn't match)"
RESPONSE=$(http_delete "/products/$LOW_ID")
assert_status "$RESPONSE" 200 "delete cheap product"
assert_json "$RESPONSE" ".data.deleted" "true" "deleted flag"
echo "  PASS"
echo ""

# =============================================================================
# Webhook Dispatch Verification (BEFORE summary — summary exits)
# =============================================================================
echo ""
echo "========================================="
echo "  WEBHOOK DISPATCH VERIFICATION"
echo "========================================="

# Wait a moment for fire-and-forget webhook threads to complete
sleep 1

# Query the built-in webhook audit log endpoint
WEBHOOK_LOG=$(curl -sf "http://127.0.0.1:$PORT/webhooks/recent?limit=0" 2>/dev/null || echo "[]")
WEBHOOK_COUNT=$(echo "$WEBHOOK_LOG" | jq '. | length' 2>/dev/null || echo "0")

echo "Webhook dispatch records: $WEBHOOK_COUNT"

if [ "$WEBHOOK_COUNT" -gt 0 ]; then
    echo ""
    echo "Dispatch log (in-memory /recent):"
    echo "$WEBHOOK_LOG" | jq -r '.[] | "  [\(.status | ascii_upcase)] \(.event) → \(.url) | HTTP \(.response_code) | \(.timestamp)"' 2>/dev/null || echo "$WEBHOOK_LOG"
    
    CREATED_COUNT=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.event == "created")] | length' 2>/dev/null || echo "0")
    UPDATED_COUNT=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.event == "updated")] | length' 2>/dev/null || echo "0")
    DELETED_COUNT=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.event == "deleted")] | length' 2>/dev/null || echo "0")
    SUCCESS_COUNT=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.status == "success")] | length' 2>/dev/null || echo "0")
    FAILED_COUNT=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.status == "failed")] | length' 2>/dev/null || echo "0")
    
    echo ""
    echo "In-memory summary:"
    echo "  created: $CREATED_COUNT (expected: 2)"
    echo "  updated: $UPDATED_COUNT (expected: 1)"
    echo "  deleted: $DELETED_COUNT (expected: 1)"
    echo "  success: $SUCCESS_COUNT"
    echo "  failed:  $FAILED_COUNT"
    
    # --- DB-backed deliveries query (persistent, filterable) ---
    echo ""
    echo "-----------------------------------------"
    echo "  DB-backed deliveries (/deliveries)"
    echo "-----------------------------------------"
    
    DB_LOG=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?limit=0" 2>/dev/null || echo "[]")
    DB_COUNT=$(echo "$DB_LOG" | jq '. | length' 2>/dev/null || echo "0")
    echo "DB delivery records: $DB_COUNT"
    
    if [ "$DB_COUNT" -gt 0 ]; then
        # Filter by resource — isolates just this test's records
        RES_FILTERED=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?resource=products&limit=0" 2>/dev/null || echo "[]")
        RES_COUNT=$(echo "$RES_FILTERED" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by resource=products: $RES_COUNT"

        # Filter by event
        EVT_FILTERED=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?resource=products&event=created&limit=0" 2>/dev/null || echo "[]")
        EVT_COUNT=$(echo "$EVT_FILTERED" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by resource=products + event=created: $EVT_COUNT (expected: 2)"

        # Filter by webhook_id
        WH_FILTERED=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?resource=products&webhook_id=wh-created&limit=0" 2>/dev/null || echo "[]")
        WH_COUNT=$(echo "$WH_FILTERED" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by resource=products + webhook_id=wh-created: $WH_COUNT (expected: 2)"

        # Filter by status
        ST_FILTERED=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?resource=products&status=success&limit=0" 2>/dev/null || echo "[]")
        ST_COUNT=$(echo "$ST_FILTERED" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by resource=products + status=success: $ST_COUNT (expected: 4)"

        # Compare resource-scoped DB counts against in-memory counts
        if [ "$RES_COUNT" -eq "$WEBHOOK_COUNT" ] && [ "$EVT_COUNT" -eq "$CREATED_COUNT" ] && [ "$WH_COUNT" -eq "$CREATED_COUNT" ] && [ "$ST_COUNT" -eq "$SUCCESS_COUNT" ]; then
            echo ""
            echo "✓ DB persistence & filtering PASSED — DB matches in-memory (resource-scoped), all filters work"
        else
            echo ""
            echo "✗ DB persistence check — counts mismatch with in-memory"
            echo "  DB(resource=products)=$RES_COUNT vs mem=$WEBHOOK_COUNT"
            echo "  DB(products+created)=$EVT_COUNT vs mem(created)=$CREATED_COUNT"
            echo "  DB(products+wh-created)=$WH_COUNT vs mem(created)=$CREATED_COUNT"
            echo "  DB(products+success)=$ST_COUNT vs mem(success)=$SUCCESS_COUNT"
        fi
    else
        echo "  ⚠️  No DB records — persistence may be disabled (DB not connected?)"
    fi
    
    # Final pass/fail
    if [ "$CREATED_COUNT" -eq 2 ] && [ "$UPDATED_COUNT" -eq 1 ] && [ "$DELETED_COUNT" -eq 1 ]; then
        echo ""
        echo "✓ Webhook dispatch verification PASSED — all expected webhooks fired"
    else
        echo ""
        echo "✗ Webhook dispatch verification FAILED — counts don't match expected"
        echo "  Expected: created=2, updated=1, deleted=1"
        echo "  Got:      created=$CREATED_COUNT, updated=$UPDATED_COUNT, deleted=$DELETED_COUNT"
    fi
else
    echo "  ⚠️  No webhook dispatch records found"
    echo "  (webhooks may not be firing — check echo server on port $ECHO_PORT)"
fi

# --- Show webhook echo server log (if available) ---
if [ -n "$ECHO_PID" ] && [ -f webhook_echo.log ]; then
    echo ""
    echo "========================================="
    echo "  WEBHOOK ECHO SERVER LOG (raw capture)"
    echo "========================================="
    cat webhook_echo.log 2>/dev/null || echo "  (no webhook events captured)"
fi

# --- Show server-side webhook logs ---
echo ""
echo "========================================="
echo "  SERVER LOGS (webhook events)"
echo "========================================="
grep -i "WEBHOOK" server.log 2>/dev/null || echo "  (no WEBHOOK entries in server.log)"

# =============================================================================
# Summary (must be LAST — calls exit)
# =============================================================================
print_http_summary
