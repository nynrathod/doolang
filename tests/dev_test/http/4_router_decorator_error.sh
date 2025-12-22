#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3104
FILE="4_router_decorator_error.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo "Test: Valid User"
curl -s -X POST http://127.0.0.1:$PORT/api/users/create \
  -H "Content-Type: application/json" \
  -d '{"Email":"ok@t.com","Age":20,"Score":50,"Username":"okt","Rank":2,"Role":"user","Name":"Ok"}' \
  | pretty_json
echo ""

echo "Test: Invalid Email (Expect 422)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/create \
  -H "Content-Type: application/json" \
  -d '{"Email":"bad","Age":20,"Score":50,"Username":"baduser","Rank":3,"Role":"user","Name":"Bad"}' \
  | pretty_json
echo ""

echo "Test: Age below minimum (Expect 422)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/create \
  -H "Content-Type: application/json" \
  -d '{"Email":"john@example.com","Age":16,"Score":50,"Username":"johnny","Rank":2,"Role":"user","Name":"John"}' \
  | pretty_json
echo ""

echo "Test: Score above maximum (Expect 422)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/create \
  -H "Content-Type: application/json" \
  -d '{"Email":"john@example.com","Age":20,"Score":150,"Username":"johnny","Rank":2,"Role":"user","Name":"John"}' \
  | pretty_json
echo ""

echo "Test: Invalid Role enum (Expect 422)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/create \
  -H "Content-Type: application/json" \
  -d '{"Email":"john@example.com","Age":20,"Score":50,"Username":"johnny","Rank":2,"Role":"guest","Name":"John"}' \
  | pretty_json
echo ""

echo "Test: Username too short (Expect 422)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/create \
  -H "Content-Type: application/json" \
  -d '{"Email":"john@example.com","Age":20,"Score":50,"Username":"ab","Rank":0,"Role":"user","Name":"John"}' \
  | pretty_json
echo ""

echo "Test: Rank out of range (Expect 422)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/create \
  -H "Content-Type: application/json" \
  -d '{"Email":"john@example.com","Age":20,"Score":50,"Username":"john_doe","Rank":6,"Role":"user","Name":"John"}' \
  | pretty_json
echo ""

echo "Test: Fully valid payload (Expect 200)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/create \
  -H "Content-Type: application/json" \
  -d '{"Email":"john@example.com","Age":20,"Score":50,"Username":"JohnDoe","Rank":1,"Role":"user","Name":"John"}' \
  | pretty_json
echo ""
