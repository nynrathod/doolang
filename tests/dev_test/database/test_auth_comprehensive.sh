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
# Signup
# =========================================================
echo ""
echo "Test 1: Signup"
RESPONSE=$(http_post "/signup" "{\"Email\":\"$EMAIL\",\"Password\":\"p\",\"Name\":\"U\",\"Role\":\"userrole\"}")
assert_status "$RESPONSE" 200 "signup"
assert_json_exists "$RESPONSE" ".data.token" "signup returns token"
assert_json_type "$RESPONSE" ".data.token" "string" "token is string"
assert_json_not_has "$RESPONSE" "Password" "password not exposed"
TOKEN=$(extract_json "$RESPONSE" ".data.token")

# =========================================================
# Signup duplicate (409)
# =========================================================
echo ""
echo "Test 2: Signup duplicate"
RESPONSE=$(http_post "/signup" "{\"Email\":\"$EMAIL\",\"Password\":\"p\",\"Name\":\"Dup\",\"Role\":\"user\"}")
assert_status "$RESPONSE" 409 "duplicate rejected"

# =========================================================
# Signup missing email (400)
# =========================================================
echo ""
echo "Test 3: Signup missing email"
RESPONSE=$(http_post "/signup" '{"Password":"p","Name":"X","Role":"user"}')
assert_status "$RESPONSE" 400 "missing email rejected"

# =========================================================
# Signup missing password (400)
# =========================================================
echo ""
echo "Test 4: Signup missing password"
RESPONSE=$(http_post "/signup" '{"Email":"nopass@t.com","Name":"X","Role":"user"}')
assert_status "$RESPONSE" 400 "missing password rejected"

# =========================================================
# Signup invalid email format (400)
# =========================================================
echo ""
echo "Test 5: Signup invalid email"
RESPONSE=$(http_post "/signup" '{"Email":"not-an-email","Password":"p","Name":"X","Role":"user"}')
assert_status "$RESPONSE" 400 "invalid email rejected"

# =========================================================
# Signup empty body (400)
# =========================================================
echo ""
echo "Test 6: Signup empty body"
RESPONSE=$(http_post "/signup" '{}')
assert_status "$RESPONSE" 400 "empty body rejected"

# =========================================================
# Signup invalid JSON (400)
# =========================================================
echo ""
echo "Test 7: Signup invalid JSON"
RESPONSE=$(http_post "/signup" 'not json')
assert_status "$RESPONSE" 400 "invalid JSON rejected"

# =========================================================
# Login
# =========================================================
echo ""
echo "Test 8: Login"
RESPONSE=$(http_post "/login" "{\"Email\":\"$EMAIL\",\"Password\":\"p\"}")
assert_status "$RESPONSE" 200 "login"
assert_json_exists "$RESPONSE" ".data.token" "login returns token"
NEW_TOKEN=$(extract_json "$RESPONSE" ".data.token")
if [ -n "$NEW_TOKEN" ] && [ "$NEW_TOKEN" != "null" ]; then
    TOKEN="$NEW_TOKEN"
fi

# =========================================================
# Login wrong password (401)
# =========================================================
echo ""
echo "Test 9: Login wrong password"
RESPONSE=$(http_post "/login" "{\"Email\":\"$EMAIL\",\"Password\":\"wrong\"}")
assert_status "$RESPONSE" 401 "wrong password rejected"

# =========================================================
# Login non-existent user (401)
# =========================================================
echo ""
echo "Test 10: Login non-existent user"
RESPONSE=$(http_post "/login" '{"Email":"nobody@t.com","Password":"p"}')
assert_status "$RESPONSE" 401 "non-existent user rejected"

# =========================================================
# Login missing email (400)
# =========================================================
echo ""
echo "Test 11: Login missing email"
RESPONSE=$(http_post "/login" '{"Password":"p"}')
assert_status "$RESPONSE" 400 "login missing email rejected"

# =========================================================
# Login missing password (400)
# =========================================================
echo ""
echo "Test 12: Login missing password"
RESPONSE=$(http_post "/login" "{\"Email\":\"$EMAIL\"}")
assert_status "$RESPONSE" 400 "login missing password rejected"

# =========================================================
# Protected route with valid token (200)
# =========================================================
echo ""
echo "Test 13: Protected route with valid token"
RESPONSE=$(http_get "/profile" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "protected route accessible"

# =========================================================
# Protected route no token (401)
# =========================================================
echo ""
echo "Test 14: Protected route no token"
RESPONSE=$(http_get "/profile")
assert_status "$RESPONSE" 401 "no token rejected"

# =========================================================
# Protected route bad token (401)
# =========================================================
echo ""
echo "Test 15: Protected route bad token"
RESPONSE=$(http_get "/profile" "Authorization: Bearer not.a.valid.jwt")
assert_status "$RESPONSE" 401 "bad token rejected"

# =========================================================
# Tampered token (401 — signature check)
# =========================================================
echo ""
echo "Test 16: Tampered token (signature check)"
if [ -n "$TOKEN" ] && [ "${#TOKEN}" -gt 10 ]; then
    TLEN=${#TOKEN}
    TAMPERED_TOKEN="${TOKEN:0:$((TLEN-5))}XXXXX"
else
    TAMPERED_TOKEN="eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.tampered"
fi
RESPONSE=$(http_get "/profile" "Authorization: Bearer $TAMPERED_TOKEN")
assert_status "$RESPONSE" 401 "tampered token rejected"

# =========================================================
# Fake token (401 — wrong secret)
# =========================================================
echo ""
echo "Test 17: Fake token (wrong secret)"
FAKE_TOKEN="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJoYWNrZXJAdC5jb20iLCJ1c2VyX2lkIjo5OTksImV4cCI6OTk5OTk5OTk5OSwiaWF0IjoxNjAwMDAwMDAwLCJpc3MiOiJkb28ifQ.fakesig123"
RESPONSE=$(http_get "/profile" "Authorization: Bearer $FAKE_TOKEN")
assert_status "$RESPONSE" 401 "fake token rejected"

# =========================================================
# Missing Bearer prefix (401)
# =========================================================
echo ""
echo "Test 18: Missing Bearer prefix"
RESPONSE=$(http_get "/profile" "Authorization: $TOKEN")
assert_status "$RESPONSE" 401 "missing Bearer rejected"

# =========================================================
# Empty auth header (401)
# =========================================================
echo ""
echo "Test 19: Empty auth header"
RESPONSE=$(http_get "/profile" "Authorization: ")
assert_status "$RESPONSE" 401 "empty auth rejected"

# =========================================================
# Public route (200)
# =========================================================
echo ""
echo "Test 20: Public route"
RESPONSE=$(http_get "/public")
assert_status "$RESPONSE" 200 "public"

# =========================================================
# Create product
# =========================================================
echo ""
echo "Test 21: Create Product"
RESPONSE=$(http_post "/products" '{"name":"Laptop","price":999,"description":"Test"}' "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "create product"
assert_json_exists "$RESPONSE" ".data.id" "product has id"
PRODUCT_ID=$(extract_json "$RESPONSE" ".data.id")

# =========================================================
# List products
# =========================================================
echo ""
echo "Test 22: List Products"
RESPONSE=$(http_get "/products" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "list products"
assert_json_type "$RESPONSE" ".data" "array" "data is array"
assert_json_array_min "$RESPONSE" ".data" 1 "at least 1 product"

# =========================================================
# Get product by ID
# =========================================================
echo ""
echo "Test 23: Get Product"
RESPONSE=$(http_get "/products/$PRODUCT_ID" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "get product"
assert_json "$RESPONSE" ".data.Name" "Laptop" "product name"

# =========================================================
# Get product not found (404)
# =========================================================
echo ""
echo "Test 24: Get Product Not Found"
RESPONSE=$(http_get "/products/$NOT_FOUND_ID" "Authorization: Bearer $TOKEN")
assert_rfc7807 "$RESPONSE" 404 "Not Found" "not_found"

# =========================================================
# Update product
# =========================================================
echo ""
echo "Test 25: Update Product"
RESPONSE=$(http_put "/products/$PRODUCT_ID" '{"name":"Gaming Laptop","price":1299,"description":"Updated"}' "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "update product"
assert_json "$RESPONSE" ".data.Name" "Gaming Laptop" "updated name"
assert_json "$RESPONSE" ".data.Price" "1299" "updated price"

# =========================================================
# Update not found (404)
# =========================================================
echo ""
echo "Test 26: Update Product Not Found"
RESPONSE=$(http_put "/products/$NOT_FOUND_ID" '{"name":"X","price":0,"description":"X"}' "Authorization: Bearer $TOKEN")
assert_rfc7807 "$RESPONSE" 404 "Not Found" "not_found"

# =========================================================
# Delete product
# =========================================================
echo ""
echo "Test 27: Delete Product"
RESPONSE=$(http_delete "/products/$PRODUCT_ID" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "delete product"

# =========================================================
# Delete not found (404)
# =========================================================
echo ""
echo "Test 28: Delete Product Not Found"
RESPONSE=$(http_delete "/products/$NOT_FOUND_ID" "Authorization: Bearer $TOKEN")
assert_rfc7807 "$RESPONSE" 404 "Not Found" "not_found"

# =========================================================
# List after delete
# =========================================================
echo ""
echo "Test 29: List Products (after delete)"
RESPONSE=$(http_get "/products" "Authorization: Bearer $TOKEN")
assert_status "$RESPONSE" 200 "list after delete"

# =========================================================
# Second user signup + login
# =========================================================
echo ""
echo "Test 30: Second user signup + login"
EMAIL2="u2$(date +%s)@t.com"
RESPONSE=$(http_post "/signup" "{\"Email\":\"$EMAIL2\",\"Password\":\"p\",\"Name\":\"U2\",\"Role\":\"viewer\"}")
assert_status "$RESPONSE" 200 "second user signup"
TOKEN2=$(extract_json "$RESPONSE" ".data.token")
RESPONSE=$(http_post "/login" "{\"Email\":\"$EMAIL2\",\"Password\":\"p\"}")
assert_status "$RESPONSE" 200 "second user login"

# =========================================================
# Second user protected route
# =========================================================
echo ""
echo "Test 31: Second user protected route"
RESPONSE=$(http_get "/profile" "Authorization: Bearer $TOKEN2")
assert_status "$RESPONSE" 200 "second user can access"

print_http_summary
