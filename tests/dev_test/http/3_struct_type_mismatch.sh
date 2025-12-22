#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3103
FILE="3_struct_type_mismatch.doo"

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo "Test: Valid Struct (Expect 200)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/signup \
  -H "Content-Type: application/json" \
  -d '{
    "email": "alice@test.com",
    "age": 30,
    "active": true,
    "score": 99.5,
    "address": {
      "city": "Ahmedabad",
      "zip": 380015
    },
    "tagsStr": ["dev", "api"],
    "tagsInt": [1, 2, 3],
    "tagsBool": [true, false],
    "tagsFloat": [1.1, 2.2]
  }' | pretty_json
echo ""

echo "Test: Primitive Type Mismatch (Expect 400)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/signup \
  -H "Content-Type: application/json" \
  -d '{
    "email": "bob@test.com",
    "age": "not-int",
    "active": true,
    "score": 10.5,
    "address": { "city": "A", "zip": 1 },
    "tagsStr": [],
    "tagsInt": [],
    "tagsBool": [],
    "tagsFloat": []
  }' | pretty_json
echo ""

echo "Test: Nested Struct Type Mismatch (Expect 400)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/signup \
  -H "Content-Type: application/json" \
  -d '{
    "email": "carol@test.com",
    "age": 22,
    "active": true,
    "score": 12.3,
    "address": {
      "city": 123,
      "zip": "bad"
    },
    "tagsStr": [],
    "tagsInt": [],
    "tagsBool": [],
    "tagsFloat": []
  }' | pretty_json
echo ""

echo "Test: Array Element Type Mismatch (Expect 400)"
curl -s -X POST http://127.0.0.1:$PORT/api/users/signup \
  -H "Content-Type: application/json" \
  -d '{
    "email": "dan@test.com",
    "age": 40,
    "active": false,
    "score": 1.2,
    "address": { "city": "X", "zip": 9 },
    "tagsStr": ["ok", 123],
    "tagsInt": [1, "bad"],
    "tagsBool": [true, "false"],
    "tagsFloat": [1.1, "2.2"]
  }' | pretty_json
echo ""

echo "Test: Get whole struct return (Expect 200)"
curl -s -X GET http://127.0.0.1:$PORT/api/users/info | pretty_json
echo ""
