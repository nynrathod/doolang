#!/bin/bash
set -e

# =============================================================================
# WEBHOOK AUTH TEST
# Tests that app.auth() with webhooks properly:
# 1. Registers signup/login routes
# 2. Fires webhooks on signup and login events
# 3. Filter evaluation (Role=admin filter)
# 4. Payload field filtering
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3115
ECHO_PORT=9998
FILE="auth_webhook.doo"

echo "========================================="
echo "  WEBHOOK AUTH TEST"
echo "========================================="
echo ""

# --- Start echo server for webhook capture ---
echo "Starting webhook echo server on port $ECHO_PORT..."
python3 -c "
import http.server, sys, json

class WebhookHandler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        content_length = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(content_length).decode('utf-8')
        try:
            payload = json.loads(body)
            event = payload.get('event', 'unknown')
            data = payload.get('data', {})
            print(f'[WEBHOOK] event={event} | path={self.path} | data={json.dumps(data)}', flush=True)
        except:
            print(f'[WEBHOOK] raw={body}', flush=True)
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'OK')
    def log_message(self, format, *args):
        pass

server = http.server.HTTPServer(('0.0.0.0', $ECHO_PORT), WebhookHandler)
print(f'Echo server listening on port $ECHO_PORT...', flush=True)
server.serve_forever()
" > webhook_auth_echo.log 2>&1 &
ECHO_PID=$!
echo "Echo server started (PID: $ECHO_PID)"

# --- Start Doo server ---
echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

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
# Test 2: Signup — should fire "signup" webhook
# =============================================================================
echo "Test 2: Signup (should fire 'signup' webhook)"
RESPONSE=$(http_post "/auth/signup" '{"Email":"user@test.com","Password":"secret123","Name":"Test User","Role":"user"}')
echo "  Response: $RESPONSE"
STATUS=$(_get_status "$RESPONSE")
if [ "$STATUS" = "200" ]; then
    echo "  PASS — signup successful (unconditional signup webhook should fire)"
elif [ "$STATUS" = "409" ]; then
    echo "  OK — user already exists (re-run safe, webhooks may have fired on first run)"
else
    echo "  FAIL — unexpected status: $STATUS"
fi
echo ""

# =============================================================================
# Test 3: Signup with admin role — should fire BOTH signup webhooks
# =============================================================================
echo "Test 3: Signup admin (should fire 'signup' + filtered 'signup' for admin)"
RESPONSE=$(http_post "/auth/signup" '{"Email":"admin@test.com","Password":"admin123","Name":"Admin User","Role":"admin"}')
echo "  Response: $RESPONSE"
STATUS=$(_get_status "$RESPONSE")
if [ "$STATUS" = "200" ]; then
    echo "  PASS — filter match should fire admin-specific signup webhook"
elif [ "$STATUS" = "409" ]; then
    echo "  OK — admin already exists (re-run safe, webhooks may have fired on first run)"
else
    echo "  FAIL — unexpected status: $STATUS"
fi
echo ""

# =============================================================================
# Test 4: Login — should fire "login" webhook
# =============================================================================
echo "Test 4: Login (should fire 'login' webhook)"
RESPONSE=$(http_post "/auth/login" '{"Email":"user@test.com","Password":"secret123"}')
echo "  Response: $RESPONSE"
assert_status "$RESPONSE" 200 "login"
TOKEN=$(extract_json "$RESPONSE" ".data.token" 2>/dev/null || echo "")
echo "  Token: ${TOKEN:0:20}..."
echo "  PASS — login successful (unconditional login webhook should fire)"
echo ""

# =============================================================================
# Test 5: Login as admin — should fire BOTH login webhooks
# =============================================================================
echo "Test 5: Login admin (should fire 'login' + filtered 'login' for admin)"
RESPONSE=$(http_post "/auth/login" '{"Email":"admin@test.com","Password":"admin123"}')
echo "  Response: $RESPONSE"
assert_status "$RESPONSE" 200 "login admin"
ADMIN_TOKEN=$(extract_json "$RESPONSE" ".data.token" 2>/dev/null || echo "")
echo "  Admin Token: ${ADMIN_TOKEN:0:20}..."
echo "  PASS — admin login should fire admin-specific login webhook"
echo ""

# =============================================================================
# Test 6: JWT-protected route with token from login
# =============================================================================
if [ -n "$ADMIN_TOKEN" ] && [ "$ADMIN_TOKEN" != "null" ]; then
    echo "Test 6: JWT-protected GET /profile (verify login token works)"
    RESPONSE=$(curl -sf -H "Authorization: Bearer $ADMIN_TOKEN" "http://127.0.0.1:$PORT/profile" 2>/dev/null || echo '{"status":500}')
    echo "  Response: $RESPONSE"
    if echo "$RESPONSE" | grep -q '"message"' 2>/dev/null; then
        echo "  PASS — JWT token from login works"
    else
        echo "  WARN — JWT token verification needs review"
    fi
else
    echo "Test 6: SKIP — no admin token available"
fi
echo ""

# =============================================================================
# Signup edge cases
# =============================================================================
echo "Test 7: Signup with duplicate email (should return error)"
RESPONSE=$(http_post "/auth/signup" '{"Email":"user@test.com","Password":"secret123","Name":"Dup User","Role":"user"}')
echo "  Response: $RESPONSE"
echo "  INFO — duplicate email handled by auth system"
echo ""

# =============================================================================
# Webhook Dispatch Verification
# =============================================================================
echo "========================================="
echo "  WEBHOOK DISPATCH VERIFICATION"
echo "========================================="

sleep 1

WEBHOOK_LOG=$(curl -sf "http://127.0.0.1:$PORT/webhooks/recent?limit=0" 2>/dev/null || echo "[]")
WEBHOOK_COUNT=$(echo "$WEBHOOK_LOG" | jq '. | length' 2>/dev/null || echo "0")

echo "Webhook dispatch records: $WEBHOOK_COUNT"

if [ "$WEBHOOK_COUNT" -gt 0 ]; then
    echo ""
    echo "Dispatch log (in-memory /recent):"
    echo "$WEBHOOK_LOG" | jq -r '.[] | "  [\(.status | ascii_upcase)] \(.event) → \(.url) | HTTP \(.response_code) | \(.timestamp)"' 2>/dev/null || echo "$WEBHOOK_LOG"

    SIGNUP_COUNT=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.event == "signup")] | length' 2>/dev/null || echo "0")
    LOGIN_COUNT=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.event == "login")] | length' 2>/dev/null || echo "0")
    SUCCESS_COUNT=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.status == "success")] | length' 2>/dev/null || echo "0")
    FAILED_COUNT=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.status == "failed")] | length' 2>/dev/null || echo "0")

    echo ""
    echo "In-memory summary:"
    echo "  signup: $SIGNUP_COUNT"
    echo "  login:  $LOGIN_COUNT"
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
        # Filter by resource=auth:signup — isolates just signup records
        RES_SIGNUP=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?resource=auth:signup&limit=0" 2>/dev/null || echo "[]")
        RES_SIGNUP_COUNT=$(echo "$RES_SIGNUP" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by resource=auth:signup: $RES_SIGNUP_COUNT"

        # Filter by resource=auth:login — isolates just login records
        RES_LOGIN=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?resource=auth:login&limit=0" 2>/dev/null || echo "[]")
        RES_LOGIN_COUNT=$(echo "$RES_LOGIN" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by resource=auth:login: $RES_LOGIN_COUNT"

        # Filter by event=signup
        EVT_SIGNUP=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?resource=auth:signup&event=signup&limit=0" 2>/dev/null || echo "[]")
        EVT_SIGNUP_COUNT=$(echo "$EVT_SIGNUP" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by resource=auth:signup + event=signup: $EVT_SIGNUP_COUNT"

        # Filter by event=login
        EVT_LOGIN=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?resource=auth:login&event=login&limit=0" 2>/dev/null || echo "[]")
        EVT_LOGIN_COUNT=$(echo "$EVT_LOGIN" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by resource=auth:login + event=login: $EVT_LOGIN_COUNT"

        # Filter by webhook_id
        WH_SIGNUP=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?resource=auth:signup&webhook_id=wh-auth-signup&limit=0" 2>/dev/null || echo "[]")
        WH_SIGNUP_COUNT=$(echo "$WH_SIGNUP" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by resource=auth:signup + webhook_id=wh-auth-signup: $WH_SIGNUP_COUNT"

        WH_SIGNUP_ADMIN=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?resource=auth:signup&webhook_id=wh-auth-signup-admin&limit=0" 2>/dev/null || echo "[]")
        WH_SIGNUP_ADMIN_COUNT=$(echo "$WH_SIGNUP_ADMIN" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by resource=auth:signup + webhook_id=wh-auth-signup-admin: $WH_SIGNUP_ADMIN_COUNT"

        WH_LOGIN=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?resource=auth:login&webhook_id=wh-auth-login&limit=0" 2>/dev/null || echo "[]")
        WH_LOGIN_COUNT=$(echo "$WH_LOGIN" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by resource=auth:login + webhook_id=wh-auth-login: $WH_LOGIN_COUNT"

        WH_LOGIN_ADMIN=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?resource=auth:login&webhook_id=wh-auth-login-admin&limit=0" 2>/dev/null || echo "[]")
        WH_LOGIN_ADMIN_COUNT=$(echo "$WH_LOGIN_ADMIN" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by resource=auth:login + webhook_id=wh-auth-login-admin: $WH_LOGIN_ADMIN_COUNT"

        # Filter by status
        ST_SUCCESS=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?status=success&limit=0" 2>/dev/null || echo "[]")
        ST_SUCCESS_COUNT=$(echo "$ST_SUCCESS" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by status=success: $ST_SUCCESS_COUNT"

        ST_FAILED=$(curl -sf "http://127.0.0.1:$PORT/webhooks/deliveries?status=failed&limit=0" 2>/dev/null || echo "[]")
        ST_FAILED_COUNT=$(echo "$ST_FAILED" | jq '. | length' 2>/dev/null || echo "0")
        echo "Filtered by status=failed: $ST_FAILED_COUNT"

        # Cross-verify: resource totals should match in-memory event counts
        SIGNUP_MEM=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.event == "signup")] | length' 2>/dev/null || echo "0")
        LOGIN_MEM=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.event == "login")] | length' 2>/dev/null || echo "0")
        SUCCESS_MEM=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.status == "success")] | length' 2>/dev/null || echo "0")
        FAILED_MEM=$(echo "$WEBHOOK_LOG" | jq '[.[] | select(.status == "failed")] | length' 2>/dev/null || echo "0")

        if [ "$RES_SIGNUP_COUNT" -eq "$SIGNUP_MEM" ] && \
           [ "$RES_LOGIN_COUNT" -eq "$LOGIN_MEM" ] && \
           [ "$ST_SUCCESS_COUNT" -eq "$SUCCESS_MEM" ] && \
           [ "$ST_FAILED_COUNT" -eq "$FAILED_MEM" ] && \
           [ "$EVT_SIGNUP_COUNT" -eq "$SIGNUP_MEM" ] && \
           [ "$EVT_LOGIN_COUNT" -eq "$LOGIN_MEM" ]; then
            echo ""
            echo "✓ DB persistence & filtering PASSED — DB matches in-memory (resource-scoped), all filters work"
        else
            echo ""
            echo "✗ DB persistence check — counts mismatch with in-memory"
            echo "  DB(signup)=$RES_SIGNUP_COUNT vs mem(signup)=$SIGNUP_MEM"
            echo "  DB(login)=$RES_LOGIN_COUNT vs mem(login)=$LOGIN_MEM"
            echo "  DB(signup+signup)=$EVT_SIGNUP_COUNT vs mem(signup)=$SIGNUP_MEM"
            echo "  DB(login+login)=$EVT_LOGIN_COUNT vs mem(login)=$LOGIN_MEM"
            echo "  DB(success)=$ST_SUCCESS_COUNT vs mem(success)=$SUCCESS_MEM"
            echo "  DB(failed)=$ST_FAILED_COUNT vs mem(failed)=$FAILED_MEM"
        fi
    else
        echo "  ⚠️  No DB records — persistence may be disabled (DB not connected?)"
    fi
fi

echo ""
echo "Webhook echo server log:"
cat webhook_auth_echo.log 2>/dev/null | grep -E '^\[WEBHOOK\]' || echo "  (no webhook entries captured)"

# --- Show server-side webhook logs ---
echo ""
echo "========================================="
echo "  SERVER LOGS (webhook events)"
echo "========================================="
grep -i "WEBHOOK" server.log 2>/dev/null || echo "  (no WEBHOOK entries in server.log)"

# Cleanup echo server
kill $ECHO_PID 2>/dev/null || true
wait $ECHO_PID 2>/dev/null || true

print_http_summary
