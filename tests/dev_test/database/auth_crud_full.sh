#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3100
FILE="auth_crud_full.doo"
EMAIL="u$(date +%s)@t.com"
NOT_FOUND_ID=99999999

echo "Starting server on port $PORT..."

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

echo "Running tests..."

# signup
echo "Signup"
curl -s -X POST http://127.0.0.1:$PORT/signup \
  -H "Content-Type: application/json" \
  -d "{\"Email\":\"$EMAIL\",\"Password\":\"p\",\"Name\":\"U\",\"Role\":\"userrole\"}" \
  | pretty_json

# login
echo "Login"
RESP=$(curl -s -X POST http://127.0.0.1:$PORT/login \
  -H "Content-Type: application/json" \
  -d "{\"Email\":\"$EMAIL\",\"Password\":\"p\"}")

echo "$RESP" | pretty_json
TOKEN=$(echo "$RESP" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')

# protected
if [ -n "$TOKEN" ]; then
  echo "Create"
  CREATE_RESP=$(curl -s -X POST http://127.0.0.1:$PORT/products \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"name":"Laptop","price":999,"description":"Test product"}')
  echo "$CREATE_RESP" | pretty_json
  
  # Extract created product ID from response (handles both {"data":{"id":N}} and {"id":N} formats)
  PRODUCT_ID=$(echo "$CREATE_RESP" | sed -n 's/.*"id":\([0-9]*\).*/\1/p' | head -1)
  echo "Created product ID: $PRODUCT_ID"

  echo "List"
  curl -s http://127.0.0.1:$PORT/products \
    -H "Authorization: Bearer $TOKEN" \
    | pretty_json

  # get product by id not found
  echo "Get by id (Not found - ID: $NOT_FOUND_ID)"
  curl -s http://127.0.0.1:$PORT/products/$NOT_FOUND_ID \
    -H "Authorization: Bearer $TOKEN" \
    | pretty_json

  echo "Get by id (Found - ID: $PRODUCT_ID)"
  curl -s http://127.0.0.1:$PORT/products/$PRODUCT_ID \
    -H "Authorization: Bearer $TOKEN" \
    | pretty_json

  echo "Update (Not found - ID: $NOT_FOUND_ID)"
  curl -s -X PUT http://127.0.0.1:$PORT/products/$NOT_FOUND_ID \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"name":"Gaming Laptop","price":1299,"description":"Updated description"}' \
    | pretty_json

  echo "Update (Found - ID: $PRODUCT_ID)"
  curl -s -X PUT http://127.0.0.1:$PORT/products/$PRODUCT_ID \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"name":"Gaming Laptop","price":1299,"description":"Updated description"}' \
    | pretty_json

  echo "Delete (Not found - ID: $NOT_FOUND_ID)"
  curl -s -X DELETE http://127.0.0.1:$PORT/products/$NOT_FOUND_ID \
       -H "Authorization: Bearer $TOKEN" \
       | pretty_json

  echo "Delete (Found - ID: $PRODUCT_ID)"
  curl -s -X DELETE http://127.0.0.1:$PORT/products/$PRODUCT_ID \
       -H "Authorization: Bearer $TOKEN" \
       | pretty_json

  echo "List (after delete)"
  curl -s http://127.0.0.1:$PORT/products \
    -H "Authorization: Bearer $TOKEN" \
    | pretty_json

  echo "Profile"
  curl -s http://127.0.0.1:$PORT/profile \
    -H "Authorization: Bearer $TOKEN" \
    | pretty_json

  echo "Public"
  curl -s http://127.0.0.1:$PORT/public \
    | pretty_json

fi
