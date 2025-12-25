#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3101
FILE="1_router_not_found.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo "Test: Valid route"
curl -s http://127.0.0.1:$PORT/users | pretty_json
echo ""

echo "Test: 404 not found (Expect 404 JSON)"
curl -s http://127.0.0.1:$PORT/notfound | pretty_json
echo ""

echo "Test: POST Invalid (Expect 404 JSON)"
curl -s -X POST http://127.0.0.1:$PORT/invalid | pretty_json
echo ""
