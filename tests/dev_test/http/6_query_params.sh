#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3106
FILE="6_query_params.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo "Valid Int (200)"
curl -s "http://127.0.0.1:$PORT/api/users/int?id=1" | pretty_json
echo ""

echo "Invalid Int (400)"
curl -s "http://127.0.0.1:$PORT/api/users/int?id=abc" | pretty_json
echo ""

echo "Missing params (400)"
curl -s "http://127.0.0.1:$PORT/api/users/int" | pretty_json
echo ""

echo "Valid Bool (200)"
curl -s "http://127.0.0.1:$PORT/api/users/bool?id=true" | pretty_json
echo ""

echo "Invalid Bool (400)"
curl -s "http://127.0.0.1:$PORT/api/users/bool?id=yes" | pretty_json
echo ""

echo "Valid Float (200)"
curl -s "http://127.0.0.1:$PORT/api/users/float?id=1.5" | pretty_json
echo ""

echo "Invalid Float (400)"
curl -s "http://127.0.0.1:$PORT/api/users/float?id=abc" | pretty_json
echo ""

echo "Valid Str (200)"
curl -s "http://127.0.0.1:$PORT/api/users/str?id=alice" | pretty_json
echo ""

echo "Invalid Str (400)"
curl -s "http://127.0.0.1:$PORT/api/users/str?id=" | pretty_json
echo ""
