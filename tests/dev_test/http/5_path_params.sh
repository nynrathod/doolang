#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3105
FILE="5_path_params.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo "Valid Int (200)"
curl -s http://127.0.0.1:$PORT/api/users/int/123 | pretty_json
echo ""

echo "Invalid Int (400)"
curl -s http://127.0.0.1:$PORT/api/users/int/abc | pretty_json
echo ""

echo "Valid Str (200)"
curl -s http://127.0.0.1:$PORT/api/users/str/hello | pretty_json
echo ""

echo "Valid Str (200)"
curl -s http://127.0.0.1:$PORT/api/users/str/true | pretty_json
echo ""

echo "Valid Bool (200)"
curl -s http://127.0.0.1:$PORT/api/users/bool/true | pretty_json
echo ""

echo "Invalid Bool (400)"
curl -s http://127.0.0.1:$PORT/api/users/bool/yes | pretty_json
echo ""

echo "Valid Float (200)"
curl -s http://127.0.0.1:$PORT/api/users/float/12.34 | pretty_json
echo ""

echo "Invalid Float (400)"
curl -s http://127.0.0.1:$PORT/api/users/float/notnum | pretty_json
echo ""

echo "Missing ID (404 — correct behavior)"
curl -s http://127.0.0.1:$PORT/api/users/int | pretty_json
echo ""
