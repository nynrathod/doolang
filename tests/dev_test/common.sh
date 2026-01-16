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
            export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$DOO_BUILD_ROOT/target-linux}"
        else
            export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target-linux}"
        fi
    fi
    export BIN="${BIN:-$CARGO_TARGET_DIR/release/doo}"
fi

# JWT secret for auth tests
if [ -z "${JWT_SECRET:-}" ]; then
    env_candidates=()
    if [ -n "${PROJECT_ENV_FILE:-}" ]; then
        env_candidates+=("$PROJECT_ENV_FILE")
    fi
    env_candidates+=("$PWD/.env")
    env_candidates+=("$PROJECT_ROOT/url-shortner/server/.env")

    for env_file in "${env_candidates[@]}"; do
        if [ -f "$env_file" ]; then
            jwt_line=$(grep -E '^JWT_SECRET=' "$env_file" | head -n 1 || true)
            if [ -n "$jwt_line" ]; then
                jwt_val=${jwt_line#JWT_SECRET=}
                jwt_val=${jwt_val%\"}
                jwt_val=${jwt_val#\"}
                jwt_val=${jwt_val%\'}
                jwt_val=${jwt_val#\'}
                if [ -n "$jwt_val" ]; then
                    export JWT_SECRET="$jwt_val"
                    break
                fi
            fi
        fi
    done
fi

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
    # We use stdbuf to ensure unbuffered output if possible
    "$BIN" run "$doo_file" --debug >server.log 2>&1 &
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
