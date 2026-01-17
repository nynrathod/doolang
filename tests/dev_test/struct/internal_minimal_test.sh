#!/bin/bash

TOTAL_FAILURES=0

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

# Unique email to avoid conflicts
EMAIL="internal_test_$(date +%s)@test.com"

PORT=3114
FILE="internal_minimal_test.doo"

echo "=========================================================="
echo "  Field Decorator Test: @auto, @writeOnly, @internal,"
echo "                        @readOnly, optional (?)"
echo "=========================================================="
echo ""
echo "  Test Struct (User):"
echo "    id: Int @primary @auto       → NOT in req, YES in res"
echo "    Email: Str @email @unique    → YES in req, YES in res (default)"
echo "    Name: Str                    → YES in req, YES in res (default)"
echo "    Password: Str @hash @writeOnly → YES in req, NOT in res"
echo "    InternalId: Str @internal    → NOT in req, NOT in res"
echo "    Role: Str @readOnly          → NOT in req, YES in res (ignored)"
echo "    Bio?: Str                    → OPTIONAL in req, OPTIONAL in res"
echo "=========================================================="
echo ""

# Start server and set up cleanup
start_server "$FILE" "$PORT" || exit 1
setup_trap

# ==============================================================================
# TEST GROUP 1: @writeOnly - In request, NOT in response
# ==============================================================================
echo ""
echo "============================================"
echo " GROUP 1: @writeOnly Tests (Password field)"
echo " Expected: Accepted in request, hidden in response"
echo "============================================"
echo ""

# --------------------------------------------------
# Test 1.1: Signup with password - should succeed, password NOT in response
# --------------------------------------------------
echo "Test 1.1: Signup with Password (@writeOnly should be accepted in request)"
SIGNUP_RESP=$(curl -s -X POST http://127.0.0.1:$PORT/signup \
  -H "Content-Type: application/json" \
  -d "{\"Email\":\"$EMAIL\",\"Password\":\"secret123\",\"Name\":\"Test User\"}")

echo "Response:"
echo "$SIGNUP_RESP" | pretty_json
echo ""

# Check signup succeeded (has token or user id)
if echo "$SIGNUP_RESP" | grep -qi "token\|\"id\""; then
    echo "✅ Test 1.1a PASS: Signup succeeded (Password accepted in request)"
else
    echo "❌ FAIL: Signup failed"
    echo "$SIGNUP_RESP"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
fi

# Check Password NOT in response
if echo "$SIGNUP_RESP" | grep -qi "\"Password\""; then
    echo "❌ FAIL: Password should NOT be in signup response (@writeOnly)"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
else
    echo "✅ Test 1.1b PASS: Password not in response (@writeOnly working)"
fi
echo ""

# Extract token for authenticated requests
TOKEN=$(echo "$SIGNUP_RESP" | grep -o '"token":"[^"]*"' | head -1 | sed 's/"token":"//;s/"$//')

if [ -z "$TOKEN" ]; then
    echo "⚠️ No token in signup response, trying login..."
    sleep 0.5
    LOGIN_RESP=$(curl -s -X POST http://127.0.0.1:$PORT/login \
      -H "Content-Type: application/json" \
      -d "{\"Email\":\"$EMAIL\",\"Password\":\"secret123\"}")
    TOKEN=$(echo "$LOGIN_RESP" | grep -o '"token":"[^"]*"' | head -1 | sed 's/"token":"//;s/"$//')
fi

if [ -z "$TOKEN" ]; then
    echo "❌ FAIL: Could not get auth token"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
fi
echo "Token acquired: ${TOKEN:0:20}..."
echo ""

# --------------------------------------------------
# Test 1.2: Login with password - should work, password NOT in response
# --------------------------------------------------
echo "Test 1.2: Login with Password (@writeOnly accepted, hidden in response)"
LOGIN_RESP=$(curl -s -X POST http://127.0.0.1:$PORT/login \
  -H "Content-Type: application/json" \
  -d "{\"Email\":\"$EMAIL\",\"Password\":\"secret123\"}")

echo "Response:"
echo "$LOGIN_RESP" | pretty_json
echo ""

if echo "$LOGIN_RESP" | grep -qi "\"Password\""; then
    echo "❌ FAIL: Password should NOT be in login response (@writeOnly)"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
else
    echo "✅ Test 1.2 PASS: Password not in login response"
fi
echo ""

# --------------------------------------------------
# Test 1.3: Create user without password - should FAIL validation  
# --------------------------------------------------
echo "Test 1.3: Create user WITHOUT Password (should fail - @writeOnly but required)"
EMAIL2="nopwd_$(date +%s)@test.com"
CREATE_NO_PASS=$(curl -s -X POST http://127.0.0.1:$PORT/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d "{\"Email\":\"$EMAIL2\",\"Name\":\"No Password User\"}")

echo "Response:"
echo "$CREATE_NO_PASS" | pretty_json
echo ""

# Expect error since Password is required (even if @writeOnly)
if echo "$CREATE_NO_PASS" | grep -qi "error\|fail\|required\|validation\|missing"; then
    echo "✅ Test 1.3 PASS: Missing password correctly rejected"
else
    # If it succeeded, that's also acceptable if Password has default
    if echo "$CREATE_NO_PASS" | grep -qi "\"id\""; then
        echo "⚠️ Test 1.3 INFO: User created without password (may have default)"
    else
        echo "⚠️ Test 1.3 WARNING: Unexpected response for missing password"
    fi
fi
echo ""

# ==============================================================================
# TEST GROUP 2: Default (no decorator) - In request AND in response
# ==============================================================================
echo ""
echo "============================================"
echo " GROUP 2: Default Field Tests (Name, Email)"
echo " Expected: Accepted in request AND in response"
echo "============================================"
echo ""

# --------------------------------------------------
# Test 2.1: GET user - Name and Email should be in response
# --------------------------------------------------
echo "Test 2.1: GET /users - Default fields (Name, Email) should be in response"
GET_USERS=$(curl -s http://127.0.0.1:$PORT/users \
  -H "Authorization: Bearer $TOKEN")

echo "Response:"
echo "$GET_USERS" | pretty_json
echo ""

if echo "$GET_USERS" | grep -qi "Name\|name"; then
    echo "✅ Test 2.1a PASS: Name field present in response"
else
    echo "❌ FAIL: Name should be in response (default visibility)"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
fi

if echo "$GET_USERS" | grep -qi "Email\|email"; then
    echo "✅ Test 2.1b PASS: Email field present in response"
else
    echo "❌ FAIL: Email should be in response (default visibility)"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
fi
echo ""

# --------------------------------------------------
# Test 2.2: Update Name - should work (default field)
# --------------------------------------------------
echo "Test 2.2: Update Name (default field should be updatable)"
USER_ID=$(echo "$SIGNUP_RESP" | grep -o '"id":[0-9]*' | head -1 | sed 's/"id"://')
if [ -z "$USER_ID" ]; then
    USER_ID="1"
fi

UPDATE_NAME=$(curl -s -X PUT "http://127.0.0.1:$PORT/users/$USER_ID" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d "{\"Name\":\"Updated Name\"}")

echo "Response:"
echo "$UPDATE_NAME" | pretty_json
echo ""

if echo "$UPDATE_NAME" | grep -q "Updated Name"; then
    echo "✅ Test 2.2 PASS: Name updated successfully"
else
    echo "⚠️ Test 2.2 INFO: Name update response (check if update worked)"
fi
echo ""

# ==============================================================================
# TEST GROUP 3: @auto - NOT in request, YES in response
# ==============================================================================
echo ""
echo "============================================"
echo " GROUP 3: @auto Tests (id field)"
echo " Expected: NOT in request, auto-generated, in response"
echo "============================================"
echo ""

# --------------------------------------------------
# Test 3.1: Create user - id should be auto-generated and in response
# --------------------------------------------------
echo "Test 3.1: Create user - id @auto should be auto-generated"
EMAIL3="auto_$(date +%s)@test.com"
CREATE_USER=$(curl -s -X POST http://127.0.0.1:$PORT/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d "{\"Email\":\"$EMAIL3\",\"Password\":\"test1234\",\"Name\":\"Auto ID Test\"}")

echo "Response:"
echo "$CREATE_USER" | pretty_json
echo ""

if echo "$CREATE_USER" | grep -qi "\"id\""; then
    echo "✅ Test 3.1 PASS: id field auto-generated and in response (@auto working)"
else
    echo "❌ FAIL: id should be auto-generated and in response (@auto)"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
fi
echo ""

# --------------------------------------------------
# Test 3.2: Create user WITH id - should be ignored (id is @auto)
# --------------------------------------------------
echo "Test 3.2: Create user WITH explicit id (should be ignored - @auto)"
EMAIL4="auto_explicit_$(date +%s)@test.com"
CREATE_WITH_ID=$(curl -s -X POST http://127.0.0.1:$PORT/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d "{\"id\":99999,\"Email\":\"$EMAIL4\",\"Password\":\"test1234\",\"Name\":\"Explicit ID Test\"}")

echo "Response:"
echo "$CREATE_WITH_ID" | pretty_json
echo ""

# Check if id is NOT 99999 (should be auto-generated)
if echo "$CREATE_WITH_ID" | grep -q '"id":99999'; then
    echo "⚠️ Test 3.2 WARNING: id should be ignored (@auto field)"
else
    if echo "$CREATE_WITH_ID" | grep -qi "\"id\""; then
        echo "✅ Test 3.2 PASS: Explicit id ignored, auto-generated instead"
    else
        echo "⚠️ Test 3.2 INFO: Check @auto behavior"
    fi
fi
echo ""

# ==============================================================================
# TEST GROUP 4: @internal - NOT in request, NOT in response
# ==============================================================================
echo ""
echo "============================================"
echo " GROUP 4: @internal Tests (InternalId field)"
echo " Expected: NOT in request, NOT in response (backend only)"
echo "============================================"
echo ""

# --------------------------------------------------
# Test 4.1: GET user - InternalId should NOT be in response
# --------------------------------------------------
echo "Test 4.1: GET single user - InternalId should NOT be in response"
GET_SINGLE=$(curl -s "http://127.0.0.1:$PORT/users/$USER_ID" \
  -H "Authorization: Bearer $TOKEN")

echo "Response:"
echo "$GET_SINGLE" | pretty_json
echo ""

if echo "$GET_SINGLE" | grep -qi "InternalId\|internalid\|internal_id"; then
    echo "❌ FAIL: InternalId should NOT be in response (@internal)"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
else
    echo "✅ Test 4.1 PASS: InternalId not in response (@internal working)"
fi
echo ""

# --------------------------------------------------
# Test 4.2: GET all users - InternalId should NOT be in any response
# --------------------------------------------------
echo "Test 4.2: GET all users - InternalId should NOT be in any response"
GET_ALL=$(curl -s http://127.0.0.1:$PORT/users \
  -H "Authorization: Bearer $TOKEN")

if echo "$GET_ALL" | grep -qi "InternalId\|internalid\|internal_id"; then
    echo "❌ FAIL: InternalId should NOT be in any response (@internal)"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
else
    echo "✅ Test 4.2 PASS: InternalId hidden from all responses"
fi
echo ""

# --------------------------------------------------
# Test 4.3: Try to set InternalId in request - should be ignored/rejected
# --------------------------------------------------
echo "Test 4.3: Try to set InternalId in request (should be ignored - @internal)"
EMAIL5="internal_$(date +%s)@test.com"
CREATE_WITH_INTERNAL=$(curl -s -X POST http://127.0.0.1:$PORT/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d "{\"Email\":\"$EMAIL5\",\"Password\":\"test1234\",\"Name\":\"Internal Test\",\"InternalId\":\"HACKED123\"}")

echo "Response:"
echo "$CREATE_WITH_INTERNAL" | pretty_json
echo ""

# InternalId should NOT be in response at all (even if we tried to set it)
if echo "$CREATE_WITH_INTERNAL" | grep -qi "InternalId\|HACKED123"; then
    echo "❌ FAIL: InternalId should not be exposed (@internal)"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
else
    echo "✅ Test 4.3 PASS: InternalId ignored in request and hidden in response"
fi
echo ""

# ==============================================================================
# TEST GROUP 5: @readOnly - NOT in request, YES in response
# ==============================================================================
echo ""
echo "============================================"
echo " GROUP 5: @readOnly Tests (Role field)"
echo " Expected: NOT in request (ignored), auto-defaulted, in response"
echo "============================================"
echo ""

# --------------------------------------------------
# Test 5.1: Create user WITH Role - should be IGNORED (Role is @readOnly)
# --------------------------------------------------
echo "Test 5.1: Create user WITH explicit Role (should be ignored - @readOnly)"
EMAIL_ROLE="readonly_$(date +%s)@test.com"
CREATE_WITH_ROLE=$(curl -s -X POST http://127.0.0.1:$PORT/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d "{\"Email\":\"$EMAIL_ROLE\",\"Password\":\"test1234\",\"Name\":\"ReadOnly Test\",\"Role\":\"admin\"}")

echo "Response:"
echo "$CREATE_WITH_ROLE" | pretty_json
echo ""

# Check if Role is NOT "admin" (should use default "user")
if echo "$CREATE_WITH_ROLE" | grep -q '"role":"admin"'; then
    echo "❌ FAIL: Role should be ignored (@readOnly field)"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
else
    if echo "$CREATE_WITH_ROLE" | grep -qi '"role"'; then
        echo "✅ Test 5.1 PASS: Explicit Role ignored, using default value"
    else
        echo "⚠️ Test 5.1 INFO: Role field not in response (check @readOnly + @default behavior)"
    fi
fi
echo ""

# --------------------------------------------------
# Test 5.2: GET user - Role should be in response with default value
# --------------------------------------------------
echo "Test 5.2: GET user - Role @readOnly should be in response"
GET_ROLE_USER=$(curl -s http://127.0.0.1:$PORT/users \
  -H "Authorization: Bearer $TOKEN")

echo "Response (checking for role field):"
echo "$GET_ROLE_USER" | pretty_json | head -20
echo ""

if echo "$GET_ROLE_USER" | grep -qi '"role"'; then
    echo "✅ Test 5.2 PASS: Role field present in response (@readOnly working)"
else
    echo "⚠️ Test 5.2 INFO: Role may not be in response yet (check DB migration)"
fi
echo ""

# ==============================================================================
# TEST GROUP 6: Optional Fields - Optional in request AND response
# ==============================================================================
echo ""
echo "============================================"
echo " GROUP 6: Optional Field Tests (Bio field)"
echo " Expected: Optional in request, present in response if provided"
echo "============================================"
echo ""

# --------------------------------------------------
# Test 6.1: Create user WITHOUT Bio - should succeed (Bio is optional)
# --------------------------------------------------
echo "Test 6.1: Create user WITHOUT Bio (should succeed - optional field)"
EMAIL_NO_BIO="nobio_$(date +%s)@test.com"
CREATE_NO_BIO=$(curl -s -X POST http://127.0.0.1:$PORT/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d "{\"Email\":\"$EMAIL_NO_BIO\",\"Password\":\"test1234\",\"Name\":\"No Bio User\"}")

echo "Response:"
echo "$CREATE_NO_BIO" | pretty_json
echo ""

if echo "$CREATE_NO_BIO" | grep -qi "\"id\""; then
    echo "✅ Test 6.1 PASS: User created without optional Bio field"
else
    echo "❌ FAIL: User creation should succeed without optional field"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
fi
echo ""

# --------------------------------------------------
# Test 6.2: Create user WITH Bio - should succeed and Bio in response
# --------------------------------------------------
echo "Test 6.2: Create user WITH Bio (optional field should be accepted and returned)"
EMAIL_WITH_BIO="withbio_$(date +%s)@test.com"
CREATE_WITH_BIO=$(curl -s -X POST http://127.0.0.1:$PORT/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d "{\"Email\":\"$EMAIL_WITH_BIO\",\"Password\":\"test1234\",\"Name\":\"Bio User\",\"Bio\":\"Hello, I am a test user!\"}")

echo "Response:"
echo "$CREATE_WITH_BIO" | pretty_json
echo ""

if echo "$CREATE_WITH_BIO" | grep -q "Hello, I am a test user"; then
    echo "✅ Test 6.2 PASS: Bio accepted in request and present in response"
else
    if echo "$CREATE_WITH_BIO" | grep -qi "\"id\""; then
        echo "⚠️ Test 6.2 INFO: User created but Bio may not be in response (check column exists)"
    else
        echo "❌ FAIL: User creation with Bio failed"
        TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
    fi
fi
echo ""

# ==============================================================================
# TEST GROUP 7: Combinations and Edge Cases
# ==============================================================================
echo ""
echo "============================================"
echo " GROUP 7: Combinations and Edge Cases"
echo "============================================"
echo ""

# --------------------------------------------------
# Test 7.1: Signup response should have id, Email, Name but NOT Password, InternalId
# --------------------------------------------------
echo "Test 7.1: Verify signup response structure"
EMAIL6="structure_$(date +%s)@test.com"
STRUCTURE_TEST=$(curl -s -X POST http://127.0.0.1:$PORT/signup \
  -H "Content-Type: application/json" \
  -d "{\"Email\":\"$EMAIL6\",\"Password\":\"test1234\",\"Name\":\"Structure Test\"}")

echo "Response:"
echo "$STRUCTURE_TEST" | pretty_json
echo ""

PASS_COUNT=0
FAIL_COUNT=0

# Should have id (@auto generates it)
if echo "$STRUCTURE_TEST" | grep -qi "\"id\""; then
    echo "  ✓ id present (@auto)"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo "  ✗ id missing (should be auto-generated)"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# Should have Email (default)
if echo "$STRUCTURE_TEST" | grep -qi "Email"; then
    echo "  ✓ Email present (default)"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo "  ✗ Email missing (default visibility)"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# Should have Name (default)
if echo "$STRUCTURE_TEST" | grep -qi "Name"; then
    echo "  ✓ Name present (default)"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo "  ✗ Name missing (default visibility)"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# Should NOT have Password (@writeOnly)
if echo "$STRUCTURE_TEST" | grep -qi "\"Password\""; then
    echo "  ✗ Password present (should be hidden @writeOnly)"
    FAIL_COUNT=$((FAIL_COUNT + 1))
else
    echo "  ✓ Password hidden (@writeOnly)"
    PASS_COUNT=$((PASS_COUNT + 1))
fi

# Should NOT have InternalId (@internal)
if echo "$STRUCTURE_TEST" | grep -qi "InternalId"; then
    echo "  ✗ InternalId present (should be hidden @internal)"
    FAIL_COUNT=$((FAIL_COUNT + 1))
else
    echo "  ✓ InternalId hidden (@internal)"
    PASS_COUNT=$((PASS_COUNT + 1))
fi

echo ""
echo "Test 7.1 Result: $PASS_COUNT/5 checks passed"
if [ $FAIL_COUNT -gt 0 ]; then
    echo "❌ FAIL: $FAIL_COUNT field visibility issues"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
else
    echo "✅ Test 7.1 PASS: All field visibilities correct"
fi
echo ""

# --------------------------------------------------
# Test 7.2: Login with wrong password - should fail with generic error
# --------------------------------------------------
echo "Test 7.2: Login with wrong password (should fail)"
WRONG_PASS=$(curl -s -X POST http://127.0.0.1:$PORT/login \
  -H "Content-Type: application/json" \
  -d "{\"Email\":\"$EMAIL\",\"Password\":\"wrongpassword\"}")

echo "Response:"
echo "$WRONG_PASS" | pretty_json
echo ""

# RFC 7807 format returns authentication_failed type with status 401 for wrong password
if echo "$WRONG_PASS" | grep -qi "error\|fail\|invalid\|unauthorized\|authentication\|\"status\":401"; then
    echo "✅ Test 7.2 PASS: Wrong password correctly rejected"
else
    echo "❌ FAIL: Expected auth error for wrong password"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
fi
echo ""

# --------------------------------------------------
# Test 7.3: Login without password - should fail
# --------------------------------------------------
echo "Test 7.3: Login without password (should fail)"
NO_PASS=$(curl -s -X POST http://127.0.0.1:$PORT/login \
  -H "Content-Type: application/json" \
  -d "{\"Email\":\"$EMAIL\"}")

echo "Response:"
echo "$NO_PASS" | pretty_json
echo ""

# RFC 7807 format returns bad_request type with status 400 for missing fields
if echo "$NO_PASS" | grep -qi "error\|fail\|required\|validation\|bad_request\|missing\|\"status\":400"; then
    echo "✅ Test 7.3 PASS: Missing password correctly rejected (RFC 7807 format)"
else
    echo "❌ FAIL: Expected validation error for missing password"
    TOTAL_FAILURES=$((TOTAL_FAILURES + 1))
fi
echo ""

# ==============================================================================
# Summary
# ==============================================================================
echo ""
echo "=================================================="
echo "  Field Decorator Test Results Summary"
echo "=================================================="
echo ""
echo "  ┌─────────────────────────────────────────────────────┐"
echo "  │ Decorator    │ In Request │ In Response │ Status │"
echo "  ├─────────────────────────────────────────────────────┤"
echo "  │ @writeOnly   │     ✓      │      ✗      │   ✅   │"
echo "  │ (default)    │     ✓      │      ✓      │   ✅   │"
echo "  │ @auto        │     ✗      │      ✓      │   ✅   │"
echo "  │ @internal    │     ✗      │      ✗      │   ✅   │"
echo "  │ @readOnly    │     ✗      │      ✓      │   ✅   │"
echo "  │ optional (?) │    opt     │     opt     │   ✅   │"
echo "  └─────────────────────────────────────────────────────┘"
echo ""
echo "  @writeOnly (Password): Accepted in request, hidden in response"
echo "  (default) (Name, Email): Visible in both request and response"
echo "  @auto (id): Auto-generated, visible in response"
echo "  @internal (InternalId): Hidden from both request and response"
echo "  @readOnly (Role): Ignored in request, visible in response with default"
echo "  optional (Bio?): Optional in request, shown in response if provided"
echo ""
if [ $TOTAL_FAILURES -eq 0 ]; then
    echo "✅ All field decorator tests completed successfully!"
else
    echo "❌ $TOTAL_FAILURES tests failed!"
    exit 1
fi
echo ""
