#!/bin/bash
set -e

# Source common utilities (includes assertion framework)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../../common.sh"

PORT=3200
FILE="main.doo"
TS=$(date +%s)
ADMIN_EMAIL="admin_${TS}@t.com"
EDITOR_EMAIL="editor_${TS}@t.com"
USER_EMAIL="user_${TS}@t.com"
ANON_EMAIL="anon_${TS}@t.com"
NOT_FOUND_ID=99999999

echo "================================================================="
echo " RBAC Integration Test Suite"
echo " Tests: role-based access control, ownership, public/private routes"
echo "================================================================="

echo ""
echo "Starting RBAC server on port $PORT..."
start_server "$FILE" "$PORT" || exit 1
setup_trap

# ==========================================================================
# SECTION 1: Signup — register users with different roles
# ==========================================================================
echo ""
echo "--- SECTION 1: Signup ---"

echo ""
echo "Test 1.1: Signup Admin user"
RESPONSE=$(http_post "/auth/signup" "{\"Email\":\"$ADMIN_EMAIL\",\"Password\":\"pass123\",\"Name\":\"Admin User\",\"Role\":\"Admin\"}")
assert_status "$RESPONSE" 200 "admin signup"
assert_json_exists "$RESPONSE" ".data.token" "admin signup returns token"
assert_json_not_has "$RESPONSE" "Password" "password not exposed"
ADMIN_TOKEN=$(extract_json "$RESPONSE" ".data.token")
ADMIN_ID=$(extract_json "$RESPONSE" ".data.id" 2>/dev/null || echo "0")

echo ""
echo "Test 1.2: Signup Editor user"
RESPONSE=$(http_post "/auth/signup" "{\"Email\":\"$EDITOR_EMAIL\",\"Password\":\"pass123\",\"Name\":\"Editor User\",\"Role\":\"Editor\"}")
assert_status "$RESPONSE" 200 "editor signup"
assert_json_exists "$RESPONSE" ".data.token" "editor signup returns token"
EDITOR_TOKEN=$(extract_json "$RESPONSE" ".data.token")
EDITOR_ID=$(extract_json "$RESPONSE" ".data.id" 2>/dev/null || echo "0")

echo ""
echo "Test 1.3: Signup regular User"
RESPONSE=$(http_post "/auth/signup" "{\"Email\":\"$USER_EMAIL\",\"Password\":\"pass123\",\"Name\":\"Regular User\",\"Role\":\"User\"}")
assert_status "$RESPONSE" 200 "user signup"
assert_json_exists "$RESPONSE" ".data.token" "user signup returns token"
USER_TOKEN=$(extract_json "$RESPONSE" ".data.token")
USER_ID=$(extract_json "$RESPONSE" ".data.id" 2>/dev/null || echo "0")

echo ""
echo "Test 1.4: Duplicate signup — should fail 409"
RESPONSE=$(http_post "/auth/signup" "{\"Email\":\"$ADMIN_EMAIL\",\"Password\":\"pass123\",\"Name\":\"Dup\",\"Role\":\"Admin\"}")
assert_status "$RESPONSE" 409 "duplicate signup rejected"

# ==========================================================================
# SECTION 2: Login
# ==========================================================================
echo ""
echo "--- SECTION 2: Login ---"

echo ""
echo "Test 2.1: Login as Admin"
RESPONSE=$(http_post "/auth/login" "{\"Email\":\"$ADMIN_EMAIL\",\"Password\":\"pass123\"}")
assert_status "$RESPONSE" 200 "admin login"
assert_json_exists "$RESPONSE" ".data.token" "login returns token"
ADMIN_TOKEN=$(extract_json "$RESPONSE" ".data.token")

echo ""
echo "Test 2.2: Login as User"
RESPONSE=$(http_post "/auth/login" "{\"Email\":\"$USER_EMAIL\",\"Password\":\"pass123\"}")
assert_status "$RESPONSE" 200 "user login"
USER_TOKEN=$(extract_json "$RESPONSE" ".data.token")

echo ""
echo "Test 2.3: Login with wrong password — 401"
RESPONSE=$(http_post "/auth/login" "{\"Email\":\"$USER_EMAIL\",\"Password\":\"wrongpassword\"}")
assert_status "$RESPONSE" 401 "wrong password rejected"

echo ""
echo "Test 2.4: Login with unknown email — 401"
RESPONSE=$(http_post "/auth/login" "{\"Email\":\"nobody_${TS}@t.com\",\"Password\":\"pass123\"}")
assert_status "$RESPONSE" 401 "unknown user rejected"

echo ""
echo "Test 2.5: Login with empty body — 400"
RESPONSE=$(http_post "/auth/login" "{}")
assert_status "$RESPONSE" 400 "empty login body rejected"

# ==========================================================================
# SECTION 3: Public read (policy: read = public)
# ==========================================================================
echo ""
echo "--- SECTION 3: Public Read (no auth required) ---"

echo ""
echo "Test 3.1: List posts — no token — should succeed (public)"
RESPONSE=$(http_get "/posts")
assert_status "$RESPONSE" 200 "list posts public"
assert_json_type "$RESPONSE" ".data" "array" "data is array"

echo ""
echo "Test 3.2: List posts — with User token — should succeed"
RESPONSE=$(http_get "/posts" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 200 "list posts with auth"

echo ""
echo "Test 3.3: List posts — with Admin token — should succeed"
RESPONSE=$(http_get "/posts" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "list posts admin"

# ==========================================================================
# SECTION 4: Create (policy: create = authenticated)
# ==========================================================================
echo ""
echo "--- SECTION 4: Create Post (authenticated required) ---"

echo ""
echo "Test 4.1: Create post without token — should fail 401"
RESPONSE=$(http_post "/posts" '{"title":"Anon Post","body":"This should fail","user_id":0}')
assert_status "$RESPONSE" 401 "unauthenticated create rejected"

echo ""
echo "Test 4.2: Create post as User — should succeed"
RESPONSE=$(http_post "/posts" "{\"title\":\"User Post\",\"body\":\"Hello World\",\"user_id\":$USER_ID}" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 200 "user creates post"
assert_json_exists "$RESPONSE" ".data.id" "post has id"
USER_POST_ID=$(extract_json "$RESPONSE" ".data.id")

echo ""
echo "Test 4.3: Create post as Admin — should succeed"
RESPONSE=$(http_post "/posts" "{\"title\":\"Admin Post\",\"body\":\"Admin content\",\"user_id\":$ADMIN_ID}" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin creates post"
ADMIN_POST_ID=$(extract_json "$RESPONSE" ".data.id")

echo ""
echo "Test 4.4: Create post as Editor — should succeed (authenticated)"
RESPONSE=$(http_post "/posts" "{\"title\":\"Editor Post\",\"body\":\"Editor content\",\"user_id\":$EDITOR_ID}" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor creates post"
assert_json_exists "$RESPONSE" ".data.id" "editor post has id"
EDITOR_POST_ID=$(extract_json "$RESPONSE" ".data.id")

echo ""
echo "Test 4.5: Create post with missing required fields — should fail 400"
RESPONSE=$(http_post "/posts" '{"body":"No title here"}' "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 400 "missing title field rejected"

# ==========================================================================
# SECTION 5: Read by ID (policy: read = public)
# ==========================================================================
echo ""
echo "--- SECTION 5: Get Post by ID (public) ---"

echo ""
echo "Test 5.1: Get user post — no auth — should succeed"
RESPONSE=$(http_get "/posts/$USER_POST_ID")
assert_status "$RESPONSE" 200 "get post public"
assert_json_exists "$RESPONSE" ".data.id" "response has id"

echo ""
echo "Test 5.2: Get admin post — with user token — should succeed"
RESPONSE=$(http_get "/posts/$ADMIN_POST_ID" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 200 "get other's post with auth"

echo ""
echo "Test 5.3: Get non-existent post — should fail 404"
RESPONSE=$(http_get "/posts/$NOT_FOUND_ID" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 404 "get nonexistent 404"

# ==========================================================================
# SECTION 6: Update — own | Admin (ownership check)
# ==========================================================================
echo ""
echo "--- SECTION 6: Update Post (own | Admin) ---"

echo ""
echo "Test 6.1: Update own post as User — should succeed"
RESPONSE=$(http_put "/posts/$USER_POST_ID" '{"title":"Updated by User","body":"Updated body","user_id":1}' "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 200 "user updates own post"
assert_json "$RESPONSE" ".data.Title" "Updated by User" "title updated"

echo ""
echo "Test 6.2: Update other's post as User — should fail 403"
RESPONSE=$(http_put "/posts/$ADMIN_POST_ID" '{"title":"Hijacked","body":"bad actor","user_id":1}' "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 403 "user cannot update others post"

echo ""
echo "Test 6.2b: Update Editor's post as User — should fail 403"
RESPONSE=$(http_put "/posts/$EDITOR_POST_ID" '{"title":"User Hijack Editor","body":"bad"}' "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 403 "user cannot update editor-owned post"

echo ""
echo "Test 6.3: Update any post as Admin — should succeed"
RESPONSE=$(http_put "/posts/$USER_POST_ID" "{\"title\":\"Admin Override\",\"body\":\"Admin edited this\",\"user_id\":$USER_ID}" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin updates any post"

echo ""
echo "Test 6.4: Update without token — should fail 401"
RESPONSE=$(http_put "/posts/$USER_POST_ID" '{"title":"No Auth","body":"No token","user_id":0}')
assert_status "$RESPONSE" 401 "unauthenticated update rejected"

echo ""
echo "Test 6.5: Update non-existent post as Admin — should fail 404"
RESPONSE=$(http_put "/posts/$NOT_FOUND_ID" '{"title":"Ghoster","body":"Not here","user_id":0}' "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 404 "update nonexistent 404"

# ==========================================================================
# SECTION 7: Delete — own | Admin
# ==========================================================================
echo ""
echo "--- SECTION 7: Delete Post (own | Admin) ---"

echo ""
echo "Test 7.1: Delete other's post as User — should fail 403"
RESPONSE=$(http_delete "/posts/$ADMIN_POST_ID" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 403 "user cannot delete others post"

echo ""
echo "Test 7.2: Delete own post as User — should succeed"
RESPONSE=$(http_delete "/posts/$USER_POST_ID" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 200 "user deletes own post"
assert_json "$RESPONSE" ".data.deleted" "true" "deleted flag set"

echo ""
echo "Test 7.3: Get deleted post — should fail 404"
RESPONSE=$(http_get "/posts/$USER_POST_ID")
assert_status "$RESPONSE" 404 "deleted post returns 404"

echo ""
echo "Test 7.4: Delete any post as Admin — should succeed"
RESPONSE=$(http_delete "/posts/$ADMIN_POST_ID" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin deletes any post"

echo ""
echo "Test 7.4b: Admin deletes Editor's post (hierarchy delete) — should succeed"
RESPONSE=$(http_delete "/posts/$EDITOR_POST_ID" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin hierarchy delete of editor post"
assert_json "$RESPONSE" ".data.deleted" "true" "editor post deleted by admin"

echo ""
echo "Test 7.5: Delete without token — should fail 401"
RESPONSE=$(http_delete "/posts/$ADMIN_POST_ID")
assert_status "$RESPONSE" 401 "unauthenticated delete rejected"

echo ""
echo "Test 7.6: Delete non-existent post as Admin — should fail 404"
RESPONSE=$(http_delete "/posts/$NOT_FOUND_ID" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 404 "delete nonexistent 404"

echo ""
echo "Test 7.7: Editor deletes Admin's post — should fail 403"
RESPONSE=$(http_post "/posts" "{\"title\":\"Admin Protected\",\"body\":\"Editor should not delete this\",\"user_id\":$ADMIN_ID}" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin creates post for editor-delete test"
ADMIN_DELETE_TARGET_ID=$(extract_json "$RESPONSE" ".data.id")
RESPONSE=$(http_delete "/posts/$ADMIN_DELETE_TARGET_ID" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 403 "editor cannot delete admin-owned post"

echo ""
echo "Test 7.8: User deletes Editor's post — should fail 403"
RESPONSE=$(http_post "/posts" "{\"title\":\"Editor Protected\",\"body\":\"User should not delete this\",\"user_id\":$EDITOR_ID}" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor creates post for user-delete test"
EDITOR_DELETE_TARGET_ID=$(extract_json "$RESPONSE" ".data.id")
RESPONSE=$(http_delete "/posts/$EDITOR_DELETE_TARGET_ID" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 403 "user cannot delete editor-owned post"

echo ""
echo "Test 7.9: Editor deletes own post — should succeed"
RESPONSE=$(http_post "/posts" "{\"title\":\"Editor Own\",\"body\":\"Editor will delete this\",\"user_id\":$EDITOR_ID}" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor creates post to self-delete"
EDITOR_SELF_DELETE_ID=$(extract_json "$RESPONSE" ".data.id")
RESPONSE=$(http_delete "/posts/$EDITOR_SELF_DELETE_ID" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor deletes own post"
assert_json "$RESPONSE" ".data.deleted" "true" "editor own post deleted"

echo ""
echo "Test 7.10: Update already-deleted post — should fail 404"
RESPONSE=$(http_put "/posts/$EDITOR_SELF_DELETE_ID" '{"title":"Ghost Update","body":"post is gone"}' "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 404 "update on deleted post returns 404"

# ==========================================================================
# SECTION 8: Role hierarchy — Admin inherits Editor
# ==========================================================================
echo ""
echo "--- SECTION 8: Role Hierarchy (Admin inherits Editor) ---"

# Re-create a post to test hierarchy-dependent policies
RESPONSE=$(http_post "/posts" "{\"title\":\"Hierarchy Test\",\"body\":\"Testing hierarchy\",\"user_id\":$EDITOR_ID}" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor creates post (authenticated)"
HIERARCHY_POST_ID=$(extract_json "$RESPONSE" ".data.id")

echo ""
echo "Test 8.1: Admin can update any post (inherits permissions via hierarchy)"
RESPONSE=$(http_put "/posts/$HIERARCHY_POST_ID" '{"title":"Admin via Hierarchy","body":"Updated"}' "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin hierarchy update succeeds"

echo ""
echo "Test 8.2: Editor can update their own post"
RESPONSE=$(http_put "/posts/$HIERARCHY_POST_ID" '{"title":"Editor Own Update","body":"Updated by editor"}' "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor updates own post"

echo ""
echo "Test 8.3: Editor cannot update Admin's post (not own, not Admin role)"
RESPONSE=$(http_post "/posts" "{\"title\":\"Admin Only Post\",\"body\":\"Admin owns this\",\"user_id\":$ADMIN_ID}" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin creates post for hierarchy test"
ADMIN_ONLY_ID=$(extract_json "$RESPONSE" ".data.id")
RESPONSE=$(http_put "/posts/$ADMIN_ONLY_ID" '{"title":"Editor Hijack","body":"bad"}' "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 403 "editor cannot update admin-owned post"

echo ""
echo "Test 8.4: Admin deletes any post via hierarchy (Editor's post)"
RESPONSE=$(http_post "/posts" "{\"title\":\"Hierarchy Delete Test\",\"body\":\"Admin should delete this\",\"user_id\":$EDITOR_ID}" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor creates post for hierarchy delete test"
HIERARCHY_DELETE_ID=$(extract_json "$RESPONSE" ".data.id")
RESPONSE=$(http_delete "/posts/$HIERARCHY_DELETE_ID" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin can delete editor post via hierarchy"

echo ""
echo "Test 8.5: User cannot update Editor's post (no hierarchy, not own)"
RESPONSE=$(http_post "/posts" "{\"title\":\"Editor Exclusive\",\"body\":\"User cannot touch\",\"user_id\":$EDITOR_ID}" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor creates post for user-update test"
EDITOR_EXCL_ID=$(extract_json "$RESPONSE" ".data.id")
RESPONSE=$(http_put "/posts/$EDITOR_EXCL_ID" '{"title":"User Sneaks In","body":"bad"}' "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 403 "user cannot update editor post"

# ==========================================================================
# SECTION 9: /auth/me endpoint
# ==========================================================================
echo ""
echo "--- SECTION 9: /auth/me ---"

echo ""
echo "Test 9.1: GET /auth/me without token — 401"
RESPONSE=$(http_get "/auth/me")
assert_status "$RESPONSE" 401 "auth/me no token"

echo ""
echo "Test 9.2: GET /auth/me with valid token — 200"
RESPONSE=$(http_get "/auth/me" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 200 "auth/me with token"
assert_json_exists "$RESPONSE" ".data.Email" "auth/me returns email"
assert_json_not_has "$RESPONSE" "Password" "auth/me strips password"
assert_json "$RESPONSE" ".data.Role" "User" "auth/me returns correct role for User"

echo ""
echo "Test 9.3: GET /auth/me with Admin token — returns Admin role"
RESPONSE=$(http_get "/auth/me" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "auth/me with admin token"
assert_json "$RESPONSE" ".data.Role" "Admin" "auth/me returns correct role for Admin"

echo ""
echo "Test 9.4: GET /auth/me with Editor token — returns Editor role"
RESPONSE=$(http_get "/auth/me" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "auth/me with editor token"
assert_json "$RESPONSE" ".data.Role" "Editor" "auth/me returns correct role for Editor"

echo ""
echo "Test 9.5: GET /auth/me with invalid JWT — 401"
RESPONSE=$(http_get "/auth/me" "Authorization: Bearer invalid.jwt.token")
assert_status "$RESPONSE" 401 "auth/me with invalid jwt"

# ==========================================================================
# SECTION 10: Edge cases
# ==========================================================================
echo ""
echo "--- SECTION 10: Edge Cases ---"

echo ""
echo "Test 10.1: Invalid JWT token — list posts still public (200)"
RESPONSE=$(http_get "/posts" "Authorization: Bearer invalid.jwt.token")
assert_status "$RESPONSE" 200 "invalid jwt, public route still accessible"

echo ""
echo "Test 10.2: Invalid JWT token — create post 401"
RESPONSE=$(http_post "/posts" '{"title":"Bad JWT","body":"test","user_id":0}' "Authorization: Bearer invalid.jwt.token")
assert_status "$RESPONSE" 401 "invalid jwt, create rejected"

echo ""
echo "Test 10.3: Expired/malformed Bearer header — create post 401"
RESPONSE=$(http_post "/posts" '{"title":"Bad Header","body":"test","user_id":0}' "Authorization: NotBearer abc")
assert_status "$RESPONSE" 401 "malformed auth header rejected"

echo ""
echo "Test 10.4: Missing required fields in signup — 400"
RESPONSE=$(http_post "/auth/signup" '{"Email":"incomplete@t.com"}')
assert_status "$RESPONSE" 400 "incomplete signup body rejected"

echo ""
echo "Test 10.5: Create post with empty body — 400"
RESPONSE=$(http_post "/posts" "" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 400 "empty post body rejected"

echo ""
echo "Test 10.6: Signup with unknown role — 400"
RESPONSE=$(http_post "/auth/signup" "{\"Email\":\"unknown_role_${TS}@t.com\",\"Password\":\"pass123\",\"Name\":\"Bad Role\",\"Role\":\"SuperAdmin\"}")
assert_status "$RESPONSE" 400 "unknown role rejected"

echo ""
echo "Test 10.7: Verify deleted post is absent from list"
RESPONSE=$(http_get "/posts")
assert_status "$RESPONSE" 200 "list posts after deletions"
# Deleted IDs should not appear — use jq for exact numeric match (not substring)
HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))
if _body=$(_get_body "$RESPONSE") && echo "$_body" | jq -e ".data | map(.id) | contains([$USER_POST_ID])" > /dev/null 2>&1; then
    HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
    echo -e "  \033[0;31mFAIL\033[0m deleted USER_POST_ID still in list"
else
    HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
    echo -e "  \033[0;32mPASS\033[0m deleted post absent from list"
fi

# ==========================================================================
# ==========================================================================
# SECTION 11: PrivateNote — read: authenticated (not public)
# ==========================================================================
echo ""
echo "--- SECTION 11: PrivateNote (read: authenticated) ---"

echo ""
echo "Test 11.1: List notes without token — should fail 401 (read is auth-gated)"
RESPONSE=$(http_get "/notes")
assert_status "$RESPONSE" 401 "unauthenticated list notes rejected"

echo ""
echo "Test 11.2: List notes with User token — should succeed"
RESPONSE=$(http_get "/notes" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 200 "user lists notes"
assert_json_type "$RESPONSE" ".data" "array" "notes data is array"

echo ""
echo "Test 11.3: List notes with Editor token — should succeed"
RESPONSE=$(http_get "/notes" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor lists notes"

echo ""
echo "Test 11.4: List notes with invalid JWT — should fail 401"
RESPONSE=$(http_get "/notes" "Authorization: Bearer invalid.jwt.token")
assert_status "$RESPONSE" 401 "invalid jwt, notes list rejected"

echo ""
echo "Test 11.5: Create note as User — should succeed"
RESPONSE=$(http_post "/notes" "{\"Content\":\"My private note\",\"UserId\":$USER_ID}" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 200 "user creates note"
assert_json_exists "$RESPONSE" ".data.id" "note has id"
USER_NOTE_ID=$(extract_json "$RESPONSE" ".data.id")

echo ""
echo "Test 11.6: Get note by ID without token — should fail 401"
RESPONSE=$(http_get "/notes/$USER_NOTE_ID")
assert_status "$RESPONSE" 401 "unauthenticated get note rejected"

echo ""
echo "Test 11.7: Get note by ID with User token — should succeed"
RESPONSE=$(http_get "/notes/$USER_NOTE_ID" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 200 "authenticated get note"
assert_json_exists "$RESPONSE" ".data.id" "note response has id"

echo ""
echo "Test 11.8: Update own note as User — should succeed"
RESPONSE=$(http_put "/notes/$USER_NOTE_ID" "{\"Content\":\"Updated note\",\"UserId\":$USER_ID}" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 200 "user updates own note"

echo ""
echo "Test 11.9: Create note as Editor — update Editor's note as User — should fail 403 (not own)"
RESPONSE=$(http_post "/notes" "{\"Content\":\"Editor note\",\"UserId\":$EDITOR_ID}" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor creates note"
EDITOR_NOTE_ID=$(extract_json "$RESPONSE" ".data.id")
RESPONSE=$(http_put "/notes/$EDITOR_NOTE_ID" "{\"Content\":\"User steals note\",\"UserId\":$USER_ID}" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 403 "user cannot update editor's note (own-only policy)"

echo ""
echo "Test 11.10: Admin updates other's note — should fail 403 (policy is own-only, no Admin override)"
RESPONSE=$(http_put "/notes/$EDITOR_NOTE_ID" "{\"Content\":\"Admin override\",\"UserId\":$ADMIN_ID}" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 403 "admin cannot override own-only update policy on notes"

echo ""
echo "Test 11.11: User deletes own note — should succeed"
RESPONSE=$(http_delete "/notes/$USER_NOTE_ID" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 200 "user deletes own note"
assert_json "$RESPONSE" ".data.deleted" "true" "note deleted"

echo ""
echo "Test 11.12: Get deleted note — should fail 404"
RESPONSE=$(http_get "/notes/$USER_NOTE_ID" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 404 "deleted note returns 404"

# ==========================================================================
# SECTION 12: Announcement — create/update/delete: Admin only (no own check)
# ==========================================================================
echo ""
echo "--- SECTION 12: Announcement (create/update/delete: Admin only) ---"

echo ""
echo "Test 12.1: List announcements without token — should succeed (read: public)"
RESPONSE=$(http_get "/announcements")
assert_status "$RESPONSE" 200 "list announcements public"
assert_json_type "$RESPONSE" ".data" "array" "announcements data is array"

echo ""
echo "Test 12.2: Create announcement without token — should fail 401"
RESPONSE=$(http_post "/announcements" '{"title":"Anon","content":"No auth"}')
assert_status "$RESPONSE" 401 "unauthenticated create announcement rejected"

echo ""
echo "Test 12.3: Create announcement as User — should fail 403 (not Admin)"
RESPONSE=$(http_post "/announcements" "{\"Title\":\"User Ann\",\"Content\":\"Bad\",\"UserId\":$USER_ID}" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 403 "user cannot create announcement"

echo ""
echo "Test 12.4: Create announcement as Editor — should fail 403 (not Admin)"
RESPONSE=$(http_post "/announcements" "{\"Title\":\"Editor Ann\",\"Content\":\"Bad\",\"UserId\":$EDITOR_ID}" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 403 "editor cannot create announcement"

echo ""
echo "Test 12.5: Create announcement as Admin — should succeed"
RESPONSE=$(http_post "/announcements" "{\"Title\":\"Admin Announcement\",\"Content\":\"Important news\",\"UserId\":$ADMIN_ID}" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin creates announcement"
assert_json_exists "$RESPONSE" ".data.id" "announcement has id"
ANN_ID=$(extract_json "$RESPONSE" ".data.id")

echo ""
echo "Test 12.6: Get announcement — no token — should succeed (public read)"
RESPONSE=$(http_get "/announcements/$ANN_ID")
assert_status "$RESPONSE" 200 "get announcement public"

echo ""
echo "Test 12.7: Update announcement as User — should fail 403"
RESPONSE=$(http_put "/announcements/$ANN_ID" '{"title":"User Hijack","content":"bad"}' "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 403 "user cannot update announcement"

echo ""
echo "Test 12.8: Update announcement as Editor — should fail 403"
RESPONSE=$(http_put "/announcements/$ANN_ID" '{"title":"Editor Hijack","content":"bad"}' "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 403 "editor cannot update announcement"

echo ""
echo "Test 12.9: Update announcement as Admin — should succeed"
RESPONSE=$(http_put "/announcements/$ANN_ID" "{\"Title\":\"Updated Announcement\",\"Content\":\"Updated news\",\"UserId\":$ADMIN_ID}" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin updates announcement"

echo ""
echo "Test 12.10: Delete announcement as User — should fail 403"
RESPONSE=$(http_delete "/announcements/$ANN_ID" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 403 "user cannot delete announcement"

echo ""
echo "Test 12.11: Delete announcement as Editor — should fail 403"
RESPONSE=$(http_delete "/announcements/$ANN_ID" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 403 "editor cannot delete announcement"

echo ""
echo "Test 12.12: Delete announcement as Admin — should succeed"
RESPONSE=$(http_delete "/announcements/$ANN_ID" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin deletes announcement"
assert_json "$RESPONSE" ".data.deleted" "true" "announcement deleted"

echo ""
echo "Test 12.13: Non-existent announcement — 404"
RESPONSE=$(http_get "/announcements/$NOT_FOUND_ID")
assert_status "$RESPONSE" 404 "nonexistent announcement 404"

# ==========================================================================
# SECTION 13: Article — create/update/delete: Editor | Admin (no own check)
# ==========================================================================
echo ""
echo "--- SECTION 13: Article (create/update/delete: Editor | Admin, no own) ---"

echo ""
echo "Test 13.1: List articles without token — should succeed (read: public)"
RESPONSE=$(http_get "/articles")
assert_status "$RESPONSE" 200 "list articles public"
assert_json_type "$RESPONSE" ".data" "array" "articles data is array"

echo ""
echo "Test 13.2: Create article without token — should fail 401"
RESPONSE=$(http_post "/articles" '{"title":"Anon","body":"No auth"}')
assert_status "$RESPONSE" 401 "unauthenticated create article rejected"

echo ""
echo "Test 13.3: Create article as User — should fail 403 (not Editor or Admin)"
RESPONSE=$(http_post "/articles" '{}' "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 403 "user cannot create article"

echo ""
echo "Test 13.4: Create article as Editor — should succeed"
RESPONSE=$(http_post "/articles" "{\"Title\":\"Editor Article\",\"Body\":\"Good content\",\"UserId\":$EDITOR_ID}" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor creates article"
assert_json_exists "$RESPONSE" ".data.id" "article has id"
EDITOR_ARTICLE_ID=$(extract_json "$RESPONSE" ".data.id")

echo ""
echo "Test 13.5: Create article as Admin — should succeed"
RESPONSE=$(http_post "/articles" "{\"Title\":\"Admin Article\",\"Body\":\"Admin content\",\"UserId\":$ADMIN_ID}" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin creates article"
ADMIN_ARTICLE_ID=$(extract_json "$RESPONSE" ".data.id")

echo ""
echo "Test 13.6: Get article — no token — should succeed (public read)"
RESPONSE=$(http_get "/articles/$EDITOR_ARTICLE_ID")
assert_status "$RESPONSE" 200 "get article public"

echo ""
echo "Test 13.7: Update Editor's article as User — should fail 403"
RESPONSE=$(http_put "/articles/$EDITOR_ARTICLE_ID" '{"title":"User Hijack","body":"bad"}' "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 403 "user cannot update article"

echo ""
echo "Test 13.8: Update Editor's article as Editor (not owner) — should succeed (no own check)"
RESPONSE=$(http_post "/articles" "{\"Title\":\"Another Editor Article\",\"Body\":\"Content\",\"UserId\":$EDITOR_ID}" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor creates second article"
EDITOR_ARTICLE2_ID=$(extract_json "$RESPONSE" ".data.id")
# A second editor would need a separate token, so we verify Editor can update any article (no own restriction)
RESPONSE=$(http_put "/articles/$ADMIN_ARTICLE_ID" "{\"Title\":\"Editor Edits Admin Article\",\"Body\":\"Editor has permission\",\"UserId\":$EDITOR_ID}" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor can update admin-owned article (no own restriction)"

echo ""
echo "Test 13.9: Admin updates Editor's article — should succeed"
RESPONSE=$(http_put "/articles/$EDITOR_ARTICLE_ID" "{\"Title\":\"Admin Edits Editor Article\",\"Body\":\"Admin has permission\",\"UserId\":$ADMIN_ID}" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin can update editor-owned article"

echo ""
echo "Test 13.10: Delete article as User — should fail 403"
RESPONSE=$(http_delete "/articles/$EDITOR_ARTICLE_ID" "Authorization: Bearer $USER_TOKEN")
assert_status "$RESPONSE" 403 "user cannot delete article"

echo ""
echo "Test 13.11: Delete article as Editor — should succeed (no own check, role sufficient)"
RESPONSE=$(http_delete "/articles/$EDITOR_ARTICLE2_ID" "Authorization: Bearer $EDITOR_TOKEN")
assert_status "$RESPONSE" 200 "editor deletes article"
assert_json "$RESPONSE" ".data.deleted" "true" "article deleted by editor"

echo ""
echo "Test 13.12: Delete article as Admin — should succeed"
RESPONSE=$(http_delete "/articles/$ADMIN_ARTICLE_ID" "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$RESPONSE" 200 "admin deletes article"

echo ""
echo "Test 13.13: Non-existent article — 404"
RESPONSE=$(http_get "/articles/$NOT_FOUND_ID")
assert_status "$RESPONSE" 404 "nonexistent article 404"

# ==========================================================================
print_http_summary
