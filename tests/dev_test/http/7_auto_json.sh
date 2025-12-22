#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3107
FILE="7_auto_json.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo "Test: Query Params"
curl -s "http://127.0.0.1:$PORT/search?q=test&page=1" | pretty_json
echo ""

echo "Test: Path Params"
curl -s http://127.0.0.1:$PORT/users/42 | pretty_json
echo ""

echo "Test: Body Params"
curl -s -X POST http://127.0.0.1:$PORT/users/42/profile \
  -H "Content-Type: application/json" \
  -d '{"name":"Alice","email":"alice@example.com"}' \
  | pretty_json
echo ""
