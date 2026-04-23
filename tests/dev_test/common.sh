#!/bin/bash
# =============================================================================
# Doo Test Common Utilities (Linux/WSL Native Only)
# Reusable functions for all test scripts
# =============================================================================

# Get the directory where common.sh is located
COMMON_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export PROJECT_ROOT="${PROJECT_ROOT:-$(cd "$COMMON_DIR/../.." && pwd)}"

UNAME_STR="$(uname 2>/dev/null || echo '')"
if [[ "$UNAME_STR" == MINGW* || "$UNAME_STR" == MSYS* || "$UNAME_STR" == CYGWIN* ]]; then
    if [ -n "${DOO_BUILD_ROOT:-}" ]; then
        export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$DOO_BUILD_ROOT/target-windows}"
    else
        export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target-windows}"
    fi
    export BIN="${BIN:-$CARGO_TARGET_DIR/release/doo.exe}"
else
    if [[ "$UNAME_STR" == Darwin* ]]; then
        if [ -n "${DOO_BUILD_ROOT:-}" ]; then
            export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$DOO_BUILD_ROOT/target}"
        else
            export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}"
        fi
    else
        if [ -n "${DOO_BUILD_ROOT:-}" ]; then
            # DOO_BUILD_ROOT/linux (not target-linux) — matches run_pass/mod.rs WSL path
            export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$DOO_BUILD_ROOT/linux}"
        else
            export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target-linux}"
        fi
    fi
    export BIN="${BIN:-$CARGO_TARGET_DIR/release/doo}"
fi

# Determine environment variables if not set
env_candidates=()
if [ -n "${PROJECT_ENV_FILE:-}" ]; then
    env_candidates+=("$PROJECT_ENV_FILE")
fi
env_candidates+=("$PWD/.env")
env_candidates+=("$COMMON_DIR/.env")
env_candidates+=("$PROJECT_ROOT/.env")

for env_file in "${env_candidates[@]}"; do
    if [ -f "$env_file" ]; then
        # Load JWT_SECRET
        if [ -z "${JWT_SECRET:-}" ]; then
            jwt_line=$(grep -E '^JWT_SECRET=' "$env_file" | head -n 1 || true)
            if [ -n "$jwt_line" ]; then
                jwt_val=${jwt_line#JWT_SECRET=}
                jwt_val=${jwt_val%\"}
                jwt_val=${jwt_val#\"}
                jwt_val=${jwt_val%\'}
                jwt_val=${jwt_val#\'}
                jwt_val=${jwt_val%$'\r'}
                if [ -n "$jwt_val" ]; then
                    export JWT_SECRET="$jwt_val"
                fi
            fi
        fi
        
        # Load DATABASE_URL
        if [ -z "${DATABASE_URL:-}" ]; then
            db_line=$(grep -E '^DATABASE_URL=' "$env_file" | head -n 1 || true)
            if [ -n "$db_line" ]; then
                db_val=${db_line#DATABASE_URL=}
                db_val=${db_val%\"}
                db_val=${db_val#\"}
                db_val=${db_val%\'}
                db_val=${db_val#\'}
                db_val=${db_val%$'\r'}
                if [ -n "$db_val" ]; then
                    export DATABASE_URL="$db_val"
                fi
            fi
        fi
    fi
done

export JWT_SECRET="${JWT_SECRET:-test-key}"

# =============================================================================
# wait_for_health - Wait for server /health endpoint to respond
# Usage: wait_for_health PORT [MAX_ATTEMPTS]
# =============================================================================
wait_for_health() {
    local port="$1"
    local max_attempts="${2:-100}"
    local health_url="http://127.0.0.1:$port/health"
    local attempt=0

    while [ "$attempt" -lt "$max_attempts" ]; do
        if curl -sf "$health_url" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.05
        attempt=$((attempt + 1))
    done
    return 1
}

# =============================================================================
# kill_port - Kill any process listening on a given port
# Usage: kill_port PORT
# =============================================================================
kill_port() {
    local port="$1"

    # Use lsof (works on both macOS and Linux)
    if command -v lsof >/dev/null 2>&1; then
        lsof -ti :"$port" 2>/dev/null | xargs kill -9 2>/dev/null || true
        return
    fi

    # Fallback: Try fuser on Linux only (macOS fuser has different syntax)
    if [[ "$(uname)" == "Linux" ]] && command -v fuser >/dev/null 2>&1; then
        fuser -k "$port/tcp" 2>/dev/null || true
    fi
}

# =============================================================================
# start_server - Start a Doo server in background
# Usage: start_server DOO_FILE PORT
# Returns server PID in $SERVER_PID
# =============================================================================
start_server() {
    local doo_file="$1"
    local port="$2"

    # Kill any existing process on this port
    kill_port "$port"

    if [ ! -x "$BIN" ]; then
        echo "Building doo binary for tests..."
        (cd "$PROJECT_ROOT" && cargo build --release --workspace >/dev/null 2>&1) || true
    fi

    if [ ! -x "$BIN" ]; then
        echo "❌ doo binary not found or not executable at: $BIN"
        return 1
    fi

    # Start server in background AND LOG TO server.log
    # Use stdbuf -oL for line-buffered output so log is readable immediately
    if command -v stdbuf >/dev/null 2>&1; then
        stdbuf -oL "$BIN" run "$doo_file" --debug>server.log 2>&1 &
    else
        "$BIN" run "$doo_file" >server.log 2>&1 &
    fi
    export SERVER_PID=$!

    # Wait for health endpoint
    if ! wait_for_health "$port" 100; then
        echo "❌ Server failed to start (health check timeout)"
        echo "Server logs:"
        cat server.log
        kill -9 "$SERVER_PID" 2>/dev/null || true
        return 1
    fi


    return 0
}

# =============================================================================
# cleanup_server - Kill the server process and clean up
# Usage: cleanup_server (uses $SERVER_PID)
# =============================================================================
cleanup_server() {
    if [ -n "$SERVER_PID" ]; then
        kill -9 "$SERVER_PID" 2>/dev/null || true
        unset SERVER_PID
    fi

    if [ -n "$PORT" ]; then
        kill_port "$PORT"
    fi
}

# =============================================================================
# setup_trap - Set up cleanup trap for script exit
# Usage: setup_trap
# =============================================================================
setup_trap() {
    trap 'cleanup_server' EXIT INT TERM
}

# =============================================================================
# pretty_json - Pretty print JSON (or raw if jq unavailable)
# Usage: echo '{"foo":"bar"}' | pretty_json
# IMPORTANT: We buffer all input first to avoid macOS pipe timing issues
# where jq starts reading before curl finishes writing
# =============================================================================
pretty_json() {
    local input
    input="$(cat)"

    if command -v jq >/dev/null 2>&1; then
        case "$input" in
            \{*|\[*)
                if printf '%s' "$input" | jq . 2>/dev/null; then
                    return 0
                fi
                ;;
        esac
    fi

    printf '%s\n' "$input"
}

# =============================================================================
# HTTP Test Assertion Framework
# Usage in .sh test files:
#   RESPONSE=$(http_get "/users")           # GET request (captures body + status)
#   assert_status "$RESPONSE" 200           # Check HTTP status code
#   assert_body "$RESPONSE" "User found!"   # Check body exact match
#   assert_body_contains "$RESPONSE" "found" # Check body contains substring
#   assert_json "$RESPONSE" ".status" "404" # Check JSON field value
#   assert_json_type "$RESPONSE" ".type" "not_found"
#   assert_rfc7807 "$RESPONSE" 404 "Not Found" "not_found"  # Full RFC 7807 check
# =============================================================================

# Colors for assertions
_ARED='\033[0;31m'
_AGREEN='\033[0;32m'
_AYELLOW='\033[1;33m'
_ABOLD='\033[1m'
_ADIM='\033[2m'
_ANC='\033[0m'

# Verbose mode: set HTTP_VERBOSE=1 or pass -v to see full responses
# Usage: HTTP_VERBOSE=1 bash test.sh  OR  bash test.sh -v
HTTP_VERBOSE="${HTTP_VERBOSE:-0}"
for _arg in "$@"; do
    case "$_arg" in -v|--verbose) HTTP_VERBOSE=1 ;; esac
done

# Counters
HTTP_TESTS_TOTAL=0
HTTP_TESTS_PASSED=0
HTTP_TESTS_FAILED=0

# ---- extract helpers (parse body + status from combined response) ----
# Response format: <body>\n<http_code>  (last line is status code)
_get_body() { echo "$1" | sed '$d'; }
_get_status() { echo "$1" | tail -1; }

# =============================================================================
# http_get / http_post / http_put / http_delete
# Makes an HTTP request, returns body + status code (last line)
# Usage: RESPONSE=$(http_get "/path")
#        RESPONSE=$(http_post "/path" '{"key":"value"}')
#        RESPONSE=$(http_get "/path" "Authorization: Bearer $TOKEN")
# =============================================================================
http_get() {
    local path="$1"
    local header="${2:-}"
    local header2="${3:-}"
    local url="http://127.0.0.1:$PORT$path"
    local result
    local -a curl_args=(curl -s -w "\n%{http_code}")
    [ -n "$header" ] && curl_args+=(-H "$header")
    [ -n "$header2" ] && curl_args+=(-H "$header2")
    curl_args+=("$url")
    result=$("${curl_args[@]}" 2>/dev/null)
    if [ "$HTTP_VERBOSE" = "1" ]; then
        local body status
        body=$(_get_body "$result")
        status=$(_get_status "$result")
        echo -e "  ${_ADIM}[${status}] ${body}${_ANC}" >&2
    fi
    echo "$result"
}

http_post() {
    local path="$1"
    local data="${2:-}"
    local header="${3:-}"
    local url="http://127.0.0.1:$PORT$path"
    local cmd="curl -s -w \"\n%{http_code}\" -X POST"
    if [ -n "$data" ]; then
        cmd+=" -H 'Content-Type: application/json' -d '$data'"
    fi
    if [ -n "$header" ]; then
        cmd+=" -H '$header'"
    fi
    cmd+=" '$url'"
    local result
    result=$(eval $cmd 2>/dev/null)
    if [ "$HTTP_VERBOSE" = "1" ]; then
        local body status
        body=$(_get_body "$result")
        status=$(_get_status "$result")
        echo -e "  ${_ADIM}[${status}] ${body}${_ANC}" >&2
    fi
    echo "$result"
}

http_put() {
    local path="$1"
    local data="${2:-}"
    local header="${3:-}"
    local url="http://127.0.0.1:$PORT$path"
    local cmd="curl -s -w \"\n%{http_code}\" -X PUT"
    if [ -n "$data" ]; then
        cmd+=" -H 'Content-Type: application/json' -d '$data'"
    fi
    if [ -n "$header" ]; then
        cmd+=" -H '$header'"
    fi
    cmd+=" '$url'"
    local result
    result=$(eval $cmd 2>/dev/null)
    if [ "$HTTP_VERBOSE" = "1" ]; then
        local body status
        body=$(_get_body "$result")
        status=$(_get_status "$result")
        echo -e "  ${_ADIM}[${status}] ${body}${_ANC}" >&2
    fi
    echo "$result"
}

http_delete() {
    local path="$1"
    local header="${2:-}"
    local header2="${3:-}"
    local url="http://127.0.0.1:$PORT$path"
    local result
    local -a curl_args=(curl -s -w "\n%{http_code}" -X DELETE)
    [ -n "$header" ] && curl_args+=(-H "$header")
    [ -n "$header2" ] && curl_args+=(-H "$header2")
    curl_args+=("$url")
    result=$("${curl_args[@]}" 2>/dev/null)
    if [ "$HTTP_VERBOSE" = "1" ]; then
        local body status
        body=$(_get_body "$result")
        status=$(_get_status "$result")
        echo -e "  ${_ADIM}[${status}] ${body}${_ANC}" >&2
    fi
    echo "$result"
}

# =============================================================================
# assert_status - Check HTTP status code
# Usage: assert_status "$RESPONSE" 404
# =============================================================================
assert_status() {
    local response="$1"
    local expected="$2"
    local label="${3:-}"
    local actual
    actual=$(_get_status "$response")
    HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))

    if [ "$actual" = "$expected" ]; then
        HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
        echo -e "  ${_AGREEN}PASS${_ANC} status=$expected ${_ADIM}$label${_ANC}"
    else
        HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
        echo -e "  ${_ARED}FAIL${_ANC} status: expected=$expected actual=$actual ${_ADIM}$label${_ANC}"
    fi
}

# =============================================================================
# assert_body - Check response body exact match
# Usage: assert_body "$RESPONSE" "User found!"
# =============================================================================
assert_body() {
    local response="$1"
    local expected="$2"
    local label="${3:-}"
    local actual
    actual=$(_get_body "$response")
    HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))

    if [ "$actual" = "$expected" ]; then
        HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
        echo -e "  ${_AGREEN}PASS${_ANC} body=\"$expected\" ${_ADIM}$label${_ANC}"
    else
        HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
        echo -e "  ${_ARED}FAIL${_ANC} body mismatch ${_ADIM}$label${_ANC}"
        echo -e "       expected: $expected"
        echo -e "       actual:   $actual"
    fi
}

# =============================================================================
# assert_body_contains - Check response body contains substring
# Usage: assert_body_contains "$RESPONSE" "found"
# =============================================================================
assert_body_contains() {
    local response="$1"
    local substr="$2"
    local label="${3:-}"
    local actual
    actual=$(_get_body "$response")
    HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))

    if echo "$actual" | grep -qF -- "$substr"; then
        HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
        echo -e "  ${_AGREEN}PASS${_ANC} contains \"$substr\" ${_ADIM}$label${_ANC}"
    else
        HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
        echo -e "  ${_ARED}FAIL${_ANC} missing \"$substr\" ${_ADIM}$label${_ANC}"
        echo -e "       actual: $actual"
    fi
}

# =============================================================================
# assert_json - Check a JSON field value using jq
# Usage: assert_json "$RESPONSE" ".status" "404"
#        assert_json "$RESPONSE" ".title" '"Not Found"'
# =============================================================================
assert_json() {
    local response="$1"
    local field="$2"
    local expected="$3"
    local label="${4:-$field=$expected}"
    local body
    body=$(_get_body "$response")
    HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))

    if ! command -v jq >/dev/null 2>&1; then
        echo -e "  ${_AYELLOW}SKIP${_ANC} jq not installed ($label)"
        return
    fi

    local actual
    actual=$(printf '%s' "$body" | jq -r "$field" 2>/dev/null)

    if [ "$actual" = "$expected" ]; then
        HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
        echo -e "  ${_AGREEN}PASS${_ANC} $field = $expected ${_ADIM}$label${_ANC}"
    else
        HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
        echo -e "  ${_ARED}FAIL${_ANC} $field: expected=$expected actual=$actual ${_ADIM}$label${_ANC}"
    fi
}

# =============================================================================
# assert_rfc7807 - Verify full RFC 7807 error response
# Usage: assert_rfc7807 "$RESPONSE" STATUS TITLE TYPE [DETAIL] [INSTANCE] [METHOD]
# Example: assert_rfc7807 "$RESPONSE" 404 "Not Found" "not_found" \
#              "The requested route does not exist" "/notfound" "GET"
# =============================================================================
assert_rfc7807() {
    local response="$1"
    local exp_status="$2"
    local exp_title="$3"
    local exp_type="$4"
    local exp_detail="${5:-}"
    local exp_instance="${6:-}"
    local exp_method="${7:-}"

    echo -e "  ${_ABOLD}RFC 7807 check:${_ANC}"
    assert_status "$response" "$exp_status"
    assert_json "$response" ".status" "$exp_status"
    assert_json "$response" ".title" "$exp_title"
    assert_json "$response" ".type" "$exp_type"
    if [ -n "$exp_detail" ]; then
        assert_json "$response" ".detail" "$exp_detail"
    fi
    if [ -n "$exp_instance" ]; then
        assert_json "$response" ".instance" "$exp_instance"
    fi
    if [ -n "$exp_method" ]; then
        assert_json "$response" ".method" "$exp_method"
    fi
}

# =============================================================================
# assert_rfc7807_body - Verify RFC 7807 error in JSON body only (no HTTP status check)
# Use this when server returns HTTP 200 but error details in body
# (e.g., middleware 403 Forbidden returns HTTP 200 with JSON status=403)
# Usage: assert_rfc7807_body "$RESPONSE" STATUS TITLE TYPE
# =============================================================================
assert_rfc7807_body() {
    local response="$1"
    local exp_status="$2"
    local exp_title="$3"
    local exp_type="$4"
    local exp_detail="${5:-}"

    echo -e "  ${_ABOLD}RFC 7807 body check:${_ANC}"
    assert_json "$response" ".status" "$exp_status"
    assert_json "$response" ".title" "$exp_title"
    assert_json "$response" ".type" "$exp_type"
    if [ -n "$exp_detail" ]; then
        assert_json "$response" ".detail" "$exp_detail"
    fi
}

# =============================================================================
# assert_json_exists - Check a JSON field exists and is not null
# Usage: assert_json_exists "$RESPONSE" ".data.token"
# =============================================================================
assert_json_exists() {
    local response="$1"
    local field="$2"
    local label="${3:-$field exists}"
    local body
    body=$(_get_body "$response")
    HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))

    if ! command -v jq >/dev/null 2>&1; then
        echo -e "  ${_AYELLOW}SKIP${_ANC} jq not installed ($label)"
        return
    fi

    local val
    val=$(printf '%s' "$body" | jq -r "$field" 2>/dev/null)

    if [ -n "$val" ] && [ "$val" != "null" ]; then
        HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
        echo -e "  ${_AGREEN}PASS${_ANC} $field exists ${_ADIM}$label${_ANC}"
    else
        HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
        echo -e "  ${_ARED}FAIL${_ANC} $field missing or null ${_ADIM}$label${_ANC}"
    fi
}

# =============================================================================
# assert_json_type - Check a JSON field has correct type
# Types: string, number, boolean, array, object
# Usage: assert_json_type "$RESPONSE" ".data.id" "number"
# =============================================================================
assert_json_type() {
    local response="$1"
    local field="$2"
    local expected_type="$3"
    local label="${4:-$field is $expected_type}"
    local body
    body=$(_get_body "$response")
    HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))

    if ! command -v jq >/dev/null 2>&1; then
        echo -e "  ${_AYELLOW}SKIP${_ANC} jq not installed ($label)"
        return
    fi

    local actual_type
    actual_type=$(printf '%s' "$body" | jq -r "$field | type" 2>/dev/null)

    if [ "$actual_type" = "$expected_type" ]; then
        HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
        echo -e "  ${_AGREEN}PASS${_ANC} $field type=$expected_type ${_ADIM}$label${_ANC}"
    else
        HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
        echo -e "  ${_ARED}FAIL${_ANC} $field type: expected=$expected_type actual=$actual_type ${_ADIM}$label${_ANC}"
    fi
}

# =============================================================================
# assert_json_not_has - Check a key does NOT appear in JSON body
# Used for @writeOnly/@internal field verification
# Usage: assert_json_not_has "$RESPONSE" "Password"
# =============================================================================
assert_json_not_has() {
    local response="$1"
    local key="$2"
    local label="${3:-$key not in response}"
    local body
    body=$(_get_body "$response")
    HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))

    if echo "$body" | grep -qi "\"$key\""; then
        HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
        echo -e "  ${_ARED}FAIL${_ANC} \"$key\" found in response (should be hidden) ${_ADIM}$label${_ANC}"
    else
        HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
        echo -e "  ${_AGREEN}PASS${_ANC} \"$key\" not in response ${_ADIM}$label${_ANC}"
    fi
}

# =============================================================================
# assert_json_has - Check a key DOES appear in JSON body
# Usage: assert_json_has "$RESPONSE" "Email"
# =============================================================================
assert_json_has() {
    local response="$1"
    local key="$2"
    local label="${3:-$key in response}"
    local body
    body=$(_get_body "$response")
    HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))

    if echo "$body" | grep -qi "\"$key\""; then
        HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
        echo -e "  ${_AGREEN}PASS${_ANC} \"$key\" in response ${_ADIM}$label${_ANC}"
    else
        HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
        echo -e "  ${_ARED}FAIL${_ANC} \"$key\" missing from response ${_ADIM}$label${_ANC}"
    fi
}

# =============================================================================
# assert_json_gt - Check numeric JSON field > value
# Usage: assert_json_gt "$RESPONSE" ".data.id" 0
# =============================================================================
assert_json_gt() {
    local response="$1"
    local field="$2"
    local min_val="$3"
    local label="${4:-$field > $min_val}"
    local body
    body=$(_get_body "$response")
    HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))

    if ! command -v jq >/dev/null 2>&1; then
        echo -e "  ${_AYELLOW}SKIP${_ANC} jq not installed ($label)"
        return
    fi

    local actual
    actual=$(printf '%s' "$body" | jq -r "$field" 2>/dev/null)

    if [ -n "$actual" ] && [ "$actual" != "null" ] && [ "$(echo "$actual > $min_val" | bc -l 2>/dev/null || echo 0)" = "1" ]; then
        HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
        echo -e "  ${_AGREEN}PASS${_ANC} $field=$actual > $min_val ${_ADIM}$label${_ANC}"
    else
        HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
        echo -e "  ${_ARED}FAIL${_ANC} $field=$actual not > $min_val ${_ADIM}$label${_ANC}"
    fi
}

# =============================================================================
# assert_json_array_min - Check JSON array has at least N elements
# Usage: assert_json_array_min "$RESPONSE" ".data" 1
# =============================================================================
assert_json_array_min() {
    local response="$1"
    local field="$2"
    local min_len="$3"
    local label="${4:-$field length >= $min_len}"
    local body
    body=$(_get_body "$response")
    HTTP_TESTS_TOTAL=$((HTTP_TESTS_TOTAL + 1))

    if ! command -v jq >/dev/null 2>&1; then
        echo -e "  ${_AYELLOW}SKIP${_ANC} jq not installed ($label)"
        return
    fi

    local actual_len
    actual_len=$(printf '%s' "$body" | jq "$field | length" 2>/dev/null)

    if [ -n "$actual_len" ] && [ "$actual_len" -ge "$min_len" ] 2>/dev/null; then
        HTTP_TESTS_PASSED=$((HTTP_TESTS_PASSED + 1))
        echo -e "  ${_AGREEN}PASS${_ANC} $field length=$actual_len >= $min_len ${_ADIM}$label${_ANC}"
    else
        HTTP_TESTS_FAILED=$((HTTP_TESTS_FAILED + 1))
        echo -e "  ${_ARED}FAIL${_ANC} $field length=$actual_len not >= $min_len ${_ADIM}$label${_ANC}"
    fi
}

# =============================================================================
# extract_json - Extract a JSON field value from response body
# Usage: TOKEN=$(extract_json "$RESPONSE" ".data.token")
# =============================================================================
extract_json() {
    local response="$1"
    local field="$2"
    local body
    body=$(_get_body "$response")
    printf '%s' "$body" | jq -r "$field" 2>/dev/null
}

# =============================================================================
# print_http_summary - Print test results summary
# Usage: print_http_summary
# =============================================================================
print_http_summary() {
    echo ""
    echo -e "${_ABOLD}================================================================${_ANC}"
    echo -e "${_ABOLD}  HTTP TEST SUMMARY${_ANC}"
    echo -e "${_ABOLD}================================================================${_ANC}"
    echo -e "  Total:   ${_ABOLD}$HTTP_TESTS_TOTAL${_ANC}"
    echo -e "  Passed:  ${_AGREEN}$HTTP_TESTS_PASSED${_ANC}"
    if [ "$HTTP_TESTS_FAILED" -gt 0 ]; then
        echo -e "  Failed:  ${_ARED}$HTTP_TESTS_FAILED${_ANC}"
    else
        echo -e "  Failed:  0"
    fi
    echo -e "${_ABOLD}================================================================${_ANC}"
    if [ "$HTTP_TESTS_FAILED" -eq 0 ]; then
        echo -e "  ${_AGREEN}All HTTP tests passed!${_ANC}"
        exit 0
    else
        echo -e "  ${_ARED}Some tests failed${_ANC}"
        exit 1
    fi
}
