#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3109
FILE="12_middlware_chaining.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

# --------------------------------------------------
# PUBLIC ROUTE
# --------------------------------------------------

echo "Test 1: Public route (/status)"
curl -s http://127.0.0.1:$PORT/status | pretty_json
echo ""

# Expected:
# LogMiddleware → TimingMiddleware → CorsMiddleware → Handler

# --------------------------------------------------
# PROTECTED ROUTE — NO TOKEN
# --------------------------------------------------

echo "Test 2: Protected route (/protected) without token → 401"
curl -s http://127.0.0.1:$PORT/protected | pretty_json
echo ""

# Expected:
# LogMiddleware → TimingMiddleware → CorsMiddleware → AuthMiddleware (fails)

# --------------------------------------------------
# PROTECTED ROUTE — INVALID TOKEN
# --------------------------------------------------

echo "Test 3: Protected route (/protected) with invalid token → 401"
curl -s \
  -H "Authorization: Bearer wrong-token" \
  http://127.0.0.1:$PORT/protected | pretty_json
echo ""

# Expected:
# LogMiddleware → TimingMiddleware → CorsMiddleware → AuthMiddleware (fails)

# --------------------------------------------------
# PROTECTED ROUTE — VALID TOKEN
# --------------------------------------------------

echo "Test 4: Protected route (/protected) with valid token → 200"
curl -s \
  -H "Authorization: Bearer valid-token" \
  http://127.0.0.1:$PORT/protected | pretty_json
echo ""

# Expected:
# LogMiddleware → TimingMiddleware → CorsMiddleware
# → AuthMiddleware → Handler → unwind back through middleware

echo "✅ All middleware chaining tests completed"
