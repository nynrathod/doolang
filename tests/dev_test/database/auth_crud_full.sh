#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

PORT=3100
FILE="auth_crud_full.doo"
EMAIL="u$(date +%s)@t.com"
NOT_FOUND_ID=99999999

echo "Starting server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

# =========================================================
# Signup — dynamic token + ID
# =========================================================
echo ""
echo "Test 1: Signup"
RESPONSE=$(http_post "/signup" "{\"Email\":\"$EMAIL\",\"Password\":\"p\",\"Name\":\"U\",\"Role\":\"userrole\"}")
assert_status "$RESPONSE" 200 "signup"
assert_json_exists "$RESPONSE" ".data.token" "signup returns token"
assert_json_type "$RESPONSE" ".data.token" "string" "token is string"
assert_json_exists "$RESPONSE" ".data.Email" "signup returns Email"
assert_json_not_has "$RESPONSE" "Password" "password not exposed"
TOKEN=$(extract_json "$RESPONSE" ".data.token")

# =========================================================
# Login — dynamic token
# =========================================================
echo ""
echo "Test 2: Login"
RESPONSE=$(http_post "/login" "{\"Email\":\"$EMAIL\",\"Password\":\"p\"}")
assert_status "$RESPONSE" 200 "login"
assert_json_exists "$RESPONSE" ".data.token" "login returns token"
assert_json_type "$RESPONSE" ".data.token" "string" "token is string"
NEW_TOKEN=$(extract_json "$RESPONSE" ".data.token")
if [ -n "$NEW_TOKEN" ] && [ "$NEW_TOKEN" != "null" ]; then
    TOKEN="$NEW_TOKEN"
fi

# =========================================================
# Create Product — dynamic ID
# =========================================================
echo ""
echo "Test 3: Create Product"
RESPONSE=$(http_post "/products" '{"name":"Laptop","price":999,"description":"Test product"}' "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "create product"
assert_json_exists "$RESPONSE" ".data.id" "product has id"
assert_json_type "$RESPONSE" ".data.id" "number" "id is number"
assert_json "$RESPONSE" ".data.Name" "Laptop" "product name"
assert_json "$RESPONSE" ".data.Price" "999" "product price"
PRODUCT_ID=$(extract_json "$RESPONSE" ".data.id")

# =========================================================
# List Products — should have at least 1
# =========================================================
echo ""
echo "Test 4: List Products"
RESPONSE=$(http_get "/products" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "list products"
assert_json_type "$RESPONSE" ".data" "array" "data is array"
assert_json_array_min "$RESPONSE" ".data" 1 "at least 1 product"

# =========================================================
# Get by ID — Not Found (RFC 7807)
# =========================================================
echo ""
echo "Test 5: Get Product Not Found (404)"
RESPONSE=$(http_get "/products/$NOT_FOUND_ID" "Authorization: Bearer $TOKEN")
assert_rfc7807 "$RESPONSE" 404 "Not Found" "not_found"

# =========================================================
# Get by ID — Found
# =========================================================
echo ""
echo "Test 6: Get Product Found"
RESPONSE=$(http_get "/products/$PRODUCT_ID" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "get product"
assert_json "$RESPONSE" ".data.Name" "Laptop" "product name"

# =========================================================
# Update — Not Found (RFC 7807)
# =========================================================
echo ""
echo "Test 7: Update Product Not Found (404)"
RESPONSE=$(http_put "/products/$NOT_FOUND_ID" '{"name":"Gaming Laptop","price":1299,"description":"Updated description"}' "Authorization: Bearer $TOKEN")
assert_rfc7807 "$RESPONSE" 404 "Not Found" "not_found"

# =========================================================
# Update — Found
# =========================================================
echo ""
echo "Test 8: Update Product Found"
RESPONSE=$(http_put "/products/$PRODUCT_ID" '{"name":"Gaming Laptop","price":1299,"description":"Updated description"}' "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "update product"
assert_json "$RESPONSE" ".data.Name" "Gaming Laptop" "updated name"
assert_json "$RESPONSE" ".data.Price" "1299" "updated price"

# =========================================================
# Delete — Not Found (RFC 7807)
# =========================================================
echo ""
echo "Test 9: Delete Product Not Found (404)"
RESPONSE=$(http_delete "/products/$NOT_FOUND_ID" "Authorization: Bearer $TOKEN")
assert_rfc7807 "$RESPONSE" 404 "Not Found" "not_found"

# =========================================================
# Delete — Found
# =========================================================
echo ""
echo "Test 10: Delete Product Found"
RESPONSE=$(http_delete "/products/$PRODUCT_ID" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "delete product"
assert_json "$RESPONSE" ".data.deleted" "true" "deleted flag"

# =========================================================
# List after delete — empty array
# =========================================================
echo ""
echo "Test 11: List Products (after delete)"
RESPONSE=$(http_get "/products" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "list after delete"

# =========================================================
# Public route (no auth)
# =========================================================
echo ""
echo "Test 12: Public route"
RESPONSE=$(http_get "/public")
assert_status "$RESPONSE" 200 "public"

print_http_summary
