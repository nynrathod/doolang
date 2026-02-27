//! HTTP Server
//! Hyper-based HTTP server with request handling and middleware execution.
//!
//! Production-grade features:
//! - Graceful shutdown with SIGTERM/SIGINT handling (Docker/GKE/Fly.io)
//! - Connection limiting via Semaphore (prevents FD/memory exhaustion)
//! - Request body size limit (prevents OOM from malicious POSTs)
//! - Request timeout (prevents slow client starvation)
//! - K8s-ready health checks (/health, /ready, /live)
//! - Connection tracking for drain on shutdown
//!
//! ## Environment Variables
//!
//! | Variable              | Default | Description                        |
//! |-----------------------|---------|------------------------------------|
//! | `DOO_MAX_CONNECTIONS` | 10000   | Max concurrent TCP connections     |
//! | `DOO_REQUEST_TIMEOUT` | 30000   | Per-request timeout (ms)           |
//! | `DOO_MAX_BODY_SIZE`   | 1048576 | Max request body size (bytes, 1MB) |
//! | `DOO_NO_BANNER`       | unset   | Set to "1" to suppress banner      |

use crate::error::*;
use crate::helpers::*;
use crate::router::{freeze_routes, get_frozen_routes};
use crate::types::*;
use doo_ffi_core::ffi_debug;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

static STARTUP_INSTANT: OnceLock<Instant> = OnceLock::new();
const VERSION: &str = "0.4.0";

/// Get uptime in seconds (used by metrics module)
pub fn startup_uptime_secs() -> u64 {
    STARTUP_INSTANT
        .get()
        .map(|s| s.elapsed().as_secs())
        .unwrap_or(0)
}

/// Server is in draining state (shutting down, rejecting new requests)
static DRAINING: AtomicBool = AtomicBool::new(false);

/// Active in-flight request counter for drain tracking
static ACTIVE_REQUESTS: AtomicUsize = AtomicUsize::new(0);

/// Max body size (bytes). Default 1MB. Configurable via DOO_MAX_BODY_SIZE.
static MAX_BODY_SIZE: AtomicUsize = AtomicUsize::new(1_048_576);

/// Per-request timeout (ms). Default 30s. Configurable via DOO_REQUEST_TIMEOUT.
static REQUEST_TIMEOUT_MS: AtomicUsize = AtomicUsize::new(30_000);

/// Whether any WebSocket routes are registered — gates WS upgrade header scan.
/// Set once during server startup, read on every request.
static HAS_WS_ROUTES: AtomicBool = AtomicBool::new(false);

/// Static C strings for common empty values — zero per-request allocation.
/// Used by handle_request (to set) and free_doo_request (to skip freeing).
/// MUST be module-level so both functions share the same address.
static EMPTY_C_STR: &[u8] = b"\0";
static EMPTY_JSON_C_STR: &[u8] = b"{}\0";

// Static C strings for common HTTP methods — eliminates malloc+memcpy per request
static METHOD_GET_C: &[u8] = b"GET\0";
static METHOD_POST_C: &[u8] = b"POST\0";
static METHOD_PUT_C: &[u8] = b"PUT\0";
static METHOD_DELETE_C: &[u8] = b"DELETE\0";
static METHOD_PATCH_C: &[u8] = b"PATCH\0";
static METHOD_HEAD_C: &[u8] = b"HEAD\0";
static METHOD_OPTIONS_C: &[u8] = b"OPTIONS\0";

/// Check if a method C string pointer is one of the static method pointers.
/// Used in free_doo_request to avoid freeing static memory.
#[inline(always)]
fn is_static_method_ptr(ptr: *const c_char) -> bool {
    ptr == METHOD_GET_C.as_ptr() as *const c_char
        || ptr == METHOD_POST_C.as_ptr() as *const c_char
        || ptr == METHOD_PUT_C.as_ptr() as *const c_char
        || ptr == METHOD_DELETE_C.as_ptr() as *const c_char
        || ptr == METHOD_PATCH_C.as_ptr() as *const c_char
        || ptr == METHOD_HEAD_C.as_ptr() as *const c_char
        || ptr == METHOD_OPTIONS_C.as_ptr() as *const c_char
}

/// Read env var as usize with default
fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

// ============================================================================
// MIDDLEWARE EXECUTION - Single Source of Truth
// Uses raw pointer + length to avoid Vec cloning per request.
// Safe because middleware slices live in the frozen RouteRegistry ('static).
// ============================================================================

/// Execute middleware chain then handler
/// This is the centralized middleware execution point - no duplication.
/// CRITICAL: Wraps all user handler/middleware calls in catch_unwind
/// to prevent panics from crossing the FFI boundary (which is UB).
fn execute_middleware_chain(
    req: *const DooRequest,
    middleware: &[DooMiddlewareFn],
    handler: DooHandlerFn,
) -> *mut DooResult {
    // Wrap the entire chain in catch_unwind — any panic in user handler
    // or middleware is caught here instead of unwinding across FFI.
    let req_send = req as usize; // raw pointer → usize for UnwindSafe
    let handler_send = handler as usize;
    let mw_ptr = middleware.as_ptr() as usize;
    let mw_len = middleware.len();

    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let req = req_send as *const DooRequest;
        let handler: DooHandlerFn = unsafe { std::mem::transmute(handler_send) };
        let middleware_ptr: *const DooMiddlewareFn = mw_ptr as *const DooMiddlewareFn;

        if mw_len == 0 {
            // No middleware - call handler directly
            return handler(req);
        }

        // Build the next function chain using pointer-based context (no Vec clone)
        execute_middleware_at_index(req, middleware_ptr, mw_len, 0, handler)
    })) {
        Ok(result) => result,
        Err(payload) => {
            // Panic caught — return RFC 7807 error instead of UB
            doo_ffi_core::helpers::make_panic_err("HTTP", payload)
        }
    }
}

/// Execute middleware at given index, with next pointing to rest of chain.
/// Uses raw pointer + length instead of cloning the middleware Vec.
fn execute_middleware_at_index(
    req: *const DooRequest,
    middleware_ptr: *const DooMiddlewareFn,
    middleware_len: usize,
    index: usize,
    handler: DooHandlerFn,
) -> *mut DooResult {
    if index >= middleware_len {
        // All middleware executed, call the handler
        return handler(req);
    }

    let current_mw = unsafe { *middleware_ptr.add(index) };

    // Store chain context in thread-local for the FFI callback
    MIDDLEWARE_CONTEXT.with(|ctx| {
        let mut ctx = ctx.borrow_mut();
        ctx.middleware_ptr = middleware_ptr;
        ctx.middleware_len = middleware_len;
        ctx.current_index = index;
        ctx.handler = Some(handler);
    });

    // Store current request for next.call() to access
    set_current_request(req);

    // Call middleware with next function
    current_mw(req, middleware_next)
}

// Thread-local context for middleware chain execution.
// Uses raw pointer + length instead of owned Vec to avoid allocation.
thread_local! {
    static MIDDLEWARE_CONTEXT: std::cell::RefCell<MiddlewareContext> =
        std::cell::RefCell::new(MiddlewareContext::default());

    /// Current request pointer for `next.call()` to access
    static CURRENT_REQUEST: std::cell::RefCell<*const DooRequest> =
        std::cell::RefCell::new(std::ptr::null());
}

/// Set the current request pointer for middleware chain
pub fn set_current_request(req: *const DooRequest) {
    CURRENT_REQUEST.with(|r| *r.borrow_mut() = req);
}

/// Get the current request pointer for `next.call()`
pub fn get_current_request() -> *const DooRequest {
    CURRENT_REQUEST.with(|r| *r.borrow())
}

/// Middleware chain context — pointer-based, zero allocation.
/// The middleware slice pointer is valid because it points into the frozen RouteRegistry
/// which has 'static lifetime.
struct MiddlewareContext {
    middleware_ptr: *const DooMiddlewareFn,
    middleware_len: usize,
    current_index: usize,
    handler: Option<DooHandlerFn>,
}

impl Default for MiddlewareContext {
    fn default() -> Self {
        Self {
            middleware_ptr: std::ptr::null(),
            middleware_len: 0,
            current_index: 0,
            handler: None,
        }
    }
}

// Safety: DooMiddlewareFn is a function pointer (Send + Sync)
unsafe impl Send for MiddlewareContext {}

/// The "next" function passed to middleware
/// Continues execution to the next middleware or handler — zero allocation.
/// SAFETY: Uses unwrap_or to avoid panicking across FFI on missing handler.
extern "C" fn middleware_next(req: *const DooRequest) -> *mut DooResult {
    // Wrap in catch_unwind to prevent panics from crossing FFI boundary
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MIDDLEWARE_CONTEXT.with(|ctx| {
            let ctx = ctx.borrow();
            let next_index = ctx.current_index + 1;
            let middleware_ptr = ctx.middleware_ptr;
            let middleware_len = ctx.middleware_len;
            // SAFETY: Use unwrap_or_else instead of .expect() — .expect() panics
            // across FFI = instant UB. If handler is None, return 500 error.
            let handler = match ctx.handler {
                Some(h) => h,
                None => {
                    drop(ctx);
                    return doo_ffi_core::DooResult::err_str(
                        500,
                        "Internal error: middleware handler not set",
                    )
                    .into_raw();
                }
            };
            drop(ctx);

            execute_middleware_at_index(req, middleware_ptr, middleware_len, next_index, handler)
        })
    })) {
        Ok(result) => result,
        Err(payload) => doo_ffi_core::helpers::make_panic_err("HTTP.middleware_next", payload),
    }
}

/// Handle incoming HTTP request — hot path, every allocation matters
async fn handle_request(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    // Zero-alloc method — 99.99% of requests use standard methods (static &str).
    // Only custom/exotic methods allocate (TRACE, CONNECT, etc.)
    let method: std::borrow::Cow<'static, str> = match *req.method() {
        hyper::Method::GET => std::borrow::Cow::Borrowed("GET"),
        hyper::Method::POST => std::borrow::Cow::Borrowed("POST"),
        hyper::Method::PUT => std::borrow::Cow::Borrowed("PUT"),
        hyper::Method::DELETE => std::borrow::Cow::Borrowed("DELETE"),
        hyper::Method::PATCH => std::borrow::Cow::Borrowed("PATCH"),
        hyper::Method::HEAD => std::borrow::Cow::Borrowed("HEAD"),
        hyper::Method::OPTIONS => std::borrow::Cow::Borrowed("OPTIONS"),
        _ => std::borrow::Cow::Owned(req.method().to_string()),
    };
    let path = req.uri().path().to_string();
    // Copy query string only if present — avoids allocating for empty queries
    let query_owned: Option<String> = req.uri().query().map(|q| q.to_string());

    // Start timing for metrics/logger (only if either enabled — branch-predicted fast path)
    let request_start =
        if crate::metrics::is_metrics_enabled() || crate::middleware::is_logger_enabled() {
            Some(Instant::now())
        } else {
            None
        };

    // Extract Origin header for CORS dynamic origin reflection (proper ownership, no RefCell).
    // This is read once and borrowed through the response lifecycle.
    let request_origin: String = req
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Built-in health check endpoints for container orchestration (fast path)
    // /health and /live — liveness probe: is the process alive?
    // /ready — readiness probe: is the server ready to accept traffic?
    if method == "GET" {
        match path.as_str() {
            "/health" | "/live" => {
                let uptime = STARTUP_INSTANT
                    .get()
                    .map(|s| s.elapsed().as_secs())
                    .unwrap_or(0);
                let body = format!(
                    "{{\"status\":\"ok\",\"version\":\"{}\",\"uptime_s\":{}}}",
                    VERSION, uptime
                );
                return Ok(build_response(200, &body));
            }
            "/ready" => {
                // During draining (shutdown), return 503 so LB stops sending traffic
                if DRAINING.load(Ordering::Relaxed) {
                    return Ok(build_response(
                        503,
                        "{\"status\":\"draining\",\"detail\":\"Server is shutting down\"}",
                    ));
                }
                let uptime = STARTUP_INSTANT
                    .get()
                    .map(|s| s.elapsed().as_secs())
                    .unwrap_or(0);
                let body = format!(
                    "{{\"status\":\"ready\",\"version\":\"{}\",\"uptime_s\":{}}}",
                    VERSION, uptime
                );
                return Ok(build_response(200, &body));
            }
            "/metrics" if crate::metrics::is_metrics_enabled() => {
                // Prometheus-compatible metrics endpoint (enabled via app.metrics())
                let body = crate::metrics::render_metrics();
                let mut response = build_response(200, &body);
                // Set Content-Type to text/plain for Prometheus scraper
                *response.headers_mut() = {
                    let mut headers = hyper::HeaderMap::new();
                    headers.insert(
                        hyper::header::CONTENT_TYPE,
                        "text/plain; version=0.0.4; charset=utf-8".parse().unwrap(),
                    );
                    headers
                };
                return Ok(response);
            }
            _ => {}
        }
    }

    // Handle CORS preflight (OPTIONS) requests before route matching
    // Uses frozen CORS — zero lock contention
    if method == "OPTIONS" {
        if crate::middleware::has_frozen_cors() {
            let mut response = build_response(204, "");
            apply_cors_headers(&mut response, &request_origin);
            return Ok(response);
        }
    }

    // ========================================================================
    // WebSocket Upgrade Detection
    // Must happen BEFORE body consumption — hyper::upgrade::on needs the request
    // Guarded by HAS_WS_ROUTES — skip header scan entirely when no WS routes
    // ========================================================================
    let is_ws_upgrade = HAS_WS_ROUTES.load(Ordering::Relaxed)
        && req
            .headers()
            .get("upgrade")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false);

    if is_ws_upgrade {
        if crate::ws::is_ws_route(&path) {
            let ws_key = req
                .headers()
                .get("sec-websocket-key")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            let accept_key =
                tokio_tungstenite::tungstenite::handshake::derive_accept_key(ws_key.as_bytes());

            let path_clone = path.clone();

            tokio::spawn(async move {
                match hyper::upgrade::on(req).await {
                    Ok(upgraded) => {
                        let io = TokioIo::new(upgraded);
                        let ws_stream = tokio_tungstenite::WebSocketStream::from_raw_socket(
                            io,
                            tokio_tungstenite::tungstenite::protocol::Role::Server,
                            None,
                        )
                        .await;
                        crate::ws::upgrade::handle_ws_connection(ws_stream, &path_clone).await;
                    }
                    Err(e) => {
                        doo_ffi_core::ffi_fatal!(
                            "WebSocket upgrade failed for {}: {}",
                            path_clone,
                            e
                        );
                    }
                }
            });

            let response = Response::builder()
                .status(StatusCode::SWITCHING_PROTOCOLS)
                .header("Upgrade", "websocket")
                .header("Connection", "Upgrade")
                .header("Sec-WebSocket-Accept", accept_key)
                .body(Full::new(Bytes::new()))
                .unwrap();

            return Ok(response);
        } else {
            let err = Rfc7807Error::route_not_found("WS", &path);
            return Ok(build_response(404, &err.to_json()));
        }
    }

    // ========================================================================
    // DEFERRED: Thread-local error tracking setup
    // set_current_request_path + clear_last_error are moved BELOW route matching.
    // For GET /json with no middleware (benchmark case), these are skipped entirely.
    // They're only needed when: (a) middleware/body methods call request.rs FFI
    // functions that use get_current_request_path(), or (b) error recovery reads
    // get_last_error_*. On success, neither is ever read.
    // ========================================================================

    // ========================================================================
    // ALL user-facing request processing — wrapped in a block so EVERY path
    // flows to a SINGLE generic logging point at the end.
    // Infrastructure paths (health/cors/ws) already returned above.
    // ========================================================================
    let response = 'user_request: {
        // Route matching FIRST — fail fast before doing any expensive work.
        // Uses frozen registry, ZERO lock contention.
        let registry = get_frozen_routes();

        let (route_entry, params) = match registry.match_route(&method, &path) {
            Some(r) => r,
            None => {
                // Check if path exists with other methods → 405 vs 404
                let allowed = registry.find_allowed_methods(&path);
                if !allowed.is_empty() {
                    let err = Rfc7807Error::method_not_allowed(&*method, &path, allowed);
                    break 'user_request build_response(405, &err.to_json());
                }
                let err = Rfc7807Error::route_not_found(&*method, &path);
                break 'user_request build_response(404, &err.to_json());
            }
        };

        // Get handler and middleware from frozen route entry
        let handler = route_entry.handler;
        let middleware = &route_entry.middleware;

        // Compute early: needed for both header and body decisions
        let has_body_method = matches!(&*method, "POST" | "PUT" | "PATCH");

        // ========================================================================
        // Set error tracking state ONLY when needed — deferred from hot path.
        // For GET /json with no middleware, this saves ~25-30ns per request.
        // Middleware routes call request.rs FFI functions that need get_current_request_path().
        // Body methods need it for content-length/type validation errors.
        // ========================================================================
        let needs_headers = !middleware.is_empty() || has_body_method || route_entry.needs_headers;
        if needs_headers {
            set_current_request_path(&path);
            clear_last_error();
        }

        // ========================================================================
        // Extract headers ONLY when needed — biggest single optimization.
        // For GET /json with no middleware (benchmark case), this saves 5-10
        // heap allocations per request (~130-160ns).
        // Headers are needed when: (a) middleware exists (JWT reads Authorization),
        // or (b) body methods need content-length/content-type validation.
        // ========================================================================
        let headers_map: Option<HashMap<String, String>> = if needs_headers {
            let header_count = req.headers().len();
            let mut map = HashMap::with_capacity(header_count);
            for (name, value) in req.headers() {
                if let Ok(v) = value.to_str() {
                    map.insert(name.as_str().to_owned(), v.to_owned());
                }
            }
            Some(map)
        } else {
            None
        };

        // ========================================================================
        // Conditional body collection — skip for methods that don't send a body
        // Enforces DOO_MAX_BODY_SIZE to prevent OOM from malicious POSTs
        // ========================================================================
        let body_str = if has_body_method {
            let max_body = MAX_BODY_SIZE.load(Ordering::Relaxed);
            // Check Content-Length header first for early rejection (before reading body)
            // headers_map is guaranteed Some when has_body_method is true
            if let Some(cl) = headers_map.as_ref().and_then(|m| m.get("content-length")) {
                if let Ok(len) = cl.parse::<usize>() {
                    if len > max_body {
                        let err = Rfc7807Error::new(
                            413,
                            format!(
                                "Request body size {} exceeds limit of {} bytes",
                                len, max_body
                            ),
                        )
                        .with_instance(&path);
                        break 'user_request build_response(413, &err.to_json());
                    }
                }
            }
            let body_bytes = match req.collect().await {
                Ok(b) => b.to_bytes(),
                Err(e) => return Err(e), // Transport error — not a user request error
            };
            if body_bytes.len() > max_body {
                let err = Rfc7807Error::new(
                    413,
                    format!(
                        "Request body size {} exceeds limit of {} bytes",
                        body_bytes.len(),
                        max_body
                    ),
                )
                .with_instance(&path);
                break 'user_request build_response(413, &err.to_json());
            }
            if body_bytes.is_empty() {
                String::new()
            } else {
                match String::from_utf8(body_bytes.to_vec()) {
                    Ok(s) => s,
                    Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
                }
            }
        } else {
            // GET/DELETE/HEAD — no body to consume.
            // Do NOT call req.collect() — it's an unnecessary async poll per request.
            String::new()
        };

        // Parse query parameters
        let query_map: HashMap<String, String> = match query_owned.as_deref() {
            Some(q) if !q.is_empty() => parse_query(q),
            _ => HashMap::new(), // HashMap::new() doesn't allocate until first insert
        };

        // ========================================================================
        // Content-Type validation (POST/PUT/PATCH only)
        // ========================================================================
        if has_body_method && !body_str.is_empty() {
            if let Some(ct) = headers_map.as_ref().and_then(|m| m.get("content-type")) {
                if !ct.contains("application/json") {
                    let err = Rfc7807Error::wrong_content_type(&path, ct);
                    break 'user_request build_response(400, &err.to_json());
                }
            }
        }

        // ========================================================================
        // Build DooRequest — direct heap allocation via libc::malloc
        // Uses static C strings for empty values to avoid per-request allocations.
        // ========================================================================
        let params_json_c: *const c_char;
        let params_owned: String; // keep alive for params_json_c
        if params.is_empty() {
            params_json_c = EMPTY_JSON_C_STR.as_ptr() as *const c_char;
            params_owned = String::new(); // unused, no alloc
        } else {
            // Direct JSON writer — avoids serde_json::to_string overhead (no Value tree)
            let mut buf = String::with_capacity(64);
            buf.push('{');
            for (i, (key, value)) in params.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                buf.push('"');
                buf.push_str(key);
                buf.push_str("\":");
                // Emit native JSON type so emit_parse works correctly for typed struct fields:
                //   Int/Float params → JSON number (unquoted) so atoi/atof works
                //   Bool params      → JSON boolean (true/false)
                //   Str params       → JSON string (quoted)
                let is_int = !value.is_empty()
                    && value
                        .bytes()
                        .enumerate()
                        .all(|(i, b)| b.is_ascii_digit() || (i == 0 && b == b'-'));
                let is_float = !is_int && !value.is_empty() && value.parse::<f64>().is_ok();
                let is_bool = value == "true" || value == "false";

                if is_int || is_float || is_bool {
                    // Emit as native JSON value — no quotes
                    buf.push_str(value);
                } else {
                    // Emit as JSON string — escape special characters
                    buf.push('"');
                    for b in value.bytes() {
                        match b {
                            b'"' => buf.push_str("\\\""),
                            b'\\' => buf.push_str("\\\\"),
                            _ => buf.push(b as char),
                        }
                    }
                    buf.push('"');
                }
            }
            buf.push('}');
            params_owned = buf;
            params_json_c = string_to_c(&params_owned);
        };

        let body_c = if body_str.is_empty() {
            EMPTY_C_STR.as_ptr() as *const c_char
        } else {
            string_to_c(&body_str)
        };

        // ========================================================================
        // Build DooRequest on the STACK — eliminates libc::malloc+free per request.
        // The struct lives on the async task's stack (heap-backed by Tokio, but reused).
        // SAFETY: The pointer is only used synchronously within execute_middleware_chain.
        // ========================================================================
        let mut doo_request_storage = std::mem::MaybeUninit::<DooRequest>::uninit();
        let doo_request: *mut DooRequest = doo_request_storage.as_mut_ptr();
        unsafe {
            // Static C strings for standard methods — no malloc/memcpy per request
            (*doo_request).method = match &*method {
                "GET" => METHOD_GET_C.as_ptr() as *const c_char,
                "POST" => METHOD_POST_C.as_ptr() as *const c_char,
                "PUT" => METHOD_PUT_C.as_ptr() as *const c_char,
                "DELETE" => METHOD_DELETE_C.as_ptr() as *const c_char,
                "PATCH" => METHOD_PATCH_C.as_ptr() as *const c_char,
                "HEAD" => METHOD_HEAD_C.as_ptr() as *const c_char,
                "OPTIONS" => METHOD_OPTIONS_C.as_ptr() as *const c_char,
                _ => string_to_c(&method), // exotic methods still allocate
            };
            (*doo_request).path = string_to_c(&path);
            (*doo_request).body = body_c;
            (*doo_request).headers = match headers_map {
                Some(map) => Box::into_raw(Box::new(map)) as *mut std::ffi::c_void,
                None => std::ptr::null_mut(),
            };
            (*doo_request).params = params_json_c as *mut std::ffi::c_void;
            // Skip Box allocation for empty query — null pointer means no query params
            (*doo_request).query = if query_map.is_empty() {
                std::ptr::null_mut()
            } else {
                Box::into_raw(Box::new(query_map)) as *mut std::ffi::c_void
            };
            (*doo_request).user_id = std::ptr::null();
        }

        // ========================================================================
        // Execute middleware chain + handler
        // ========================================================================
        doo_ffi_json::doo_json_clear_parse_error();

        let handler_response = {
            ffi_debug!("SERVER", "About to execute handler for {} {}", method, path);
            let result = execute_middleware_chain(doo_request, middleware, handler);
            ffi_debug!(
                "SERVER",
                "Handler returned, result is_null={}",
                result.is_null()
            );

            // Check for JSON parse errors (type mismatches in arrays, etc.)
            if doo_ffi_json::doo_json_has_parse_error() {
                let status = doo_ffi_json::doo_json_get_parse_error_status();
                let error_json_ptr = doo_ffi_json::doo_json_get_parse_error_json();
                let body = c_to_string(error_json_ptr as *const i8);
                doo_ffi_json::doo_json_clear_parse_error();
                build_response(status, &body)
            } else {
                unsafe {
                    if result.is_null() {
                        ffi_debug!("SERVER", "Handler returned null!");
                        let err = internal_error("Handler returned null", &path);
                        build_response(500, &err.to_json())
                    } else {
                        // CRITICAL: Read result using doo_ffi_core::DooResult layout
                        // { i64 tag, *mut c_void data } — matches codegen output exactly.
                        let res = &*(result as *const doo_ffi_core::DooResult);
                        ffi_debug!(
                            "SERVER",
                            "Result tag={}, data_is_null={}",
                            res.tag,
                            res.data.is_null()
                        );
                        if res.tag == 0 {
                            // Success — zero-copy: read C string as bytes, pass directly
                            if res.data.is_null() {
                                build_response_bytes(200, b"{}")
                            } else {
                                let cstr = std::ffi::CStr::from_ptr(res.data as *const i8);
                                build_response_bytes(200, cstr.to_bytes())
                            }
                        } else {
                            // Error — lazily set request path if not already set
                            if !needs_headers {
                                set_current_request_path(&path);
                            }
                            if !res.data.is_null() {
                                let error_struct = res.data as *const i8;
                                let error_status = *(error_struct as *const i32);
                                let body_ptr =
                                    *((error_struct as *const u8).add(8) as *const *const i8);
                                let body = if body_ptr.is_null() {
                                    get_last_error_json()
                                } else {
                                    c_to_string(body_ptr)
                                };
                                // 3xx redirect: body is raw URL → build Location header
                                if error_status >= 300 && error_status < 400 {
                                    build_redirect(error_status, &body)
                                } else {
                                    build_response(error_status, &body)
                                }
                            } else {
                                let status = get_last_error_status();
                                let body = get_last_error_json();
                                build_response(if status > 0 { status } else { 500 }, &body)
                            }
                        }
                    }
                }
            }
        };

        // Cleanup DooRequest FIELDS — the struct itself is stack-allocated.
        // At high RPS, leaked field memory causes OOM within minutes.
        unsafe {
            free_doo_request_fields(doo_request);
        }

        handler_response
    }; // end 'user_request block

    // ========================================================================
    // APPLY PENDING COOKIES — Auth sets cookies, server writes Set-Cookie headers.
    // This is the SINGLE point where all Set-Cookie headers are applied.
    // Any auth strategy (OAuth, JWT login, refresh) pushes cookies via
    // doo_ffi_core::cookies::set_response_cookie() — we collect them here.
    //
    // Two sources of cookies:
    // 1. Structured cookies (same DLL) — from code running in doo_ffi_http
    // 2. Raw cookie headers (cross-DLL bridge) — from doo_ffi_auth via doo_http_push_cookie
    // ========================================================================
    let mut response = response;
    let pending_cookies = doo_ffi_core::cookies::take_response_cookies();
    for cookie in &pending_cookies {
        if let Ok(val) = hyper::header::HeaderValue::from_str(&cookie.to_header_value()) {
            response
                .headers_mut()
                .append(hyper::header::SET_COOKIE, val);
        }
    }
    // Cross-DLL cookies: auth DLL pushes raw Set-Cookie header strings via FFI bridge
    let raw_cookies = doo_ffi_core::cookies::take_raw_cookies();
    for raw in &raw_cookies {
        if let Ok(val) = hyper::header::HeaderValue::from_str(raw) {
            response
                .headers_mut()
                .append(hyper::header::SET_COOKIE, val);
        }
    }

    // ========================================================================
    // APPLY CORS HEADERS — Dynamic origin reflection.
    // Single post-processing point: request_origin is borrowed, no RefCell.
    // ========================================================================
    apply_cors_headers(&mut response, &request_origin);

    // ========================================================================
    // SINGLE GENERIC logging point — reads status from the response itself.
    // No hardcoded status codes. Automatically classifies:
    //   < 400 → Info | 400–499 → Warn | >= 500 → Error
    // All user-facing paths (200, 404, 405, 413, 400, 500…) flow here.
    // Infrastructure paths (health, CORS, WS) already returned above.
    // ========================================================================
    if let Some(start) = request_start {
        let duration_us = start.elapsed().as_micros() as u64;
        let status = response.status().as_u16();
        if crate::metrics::is_metrics_enabled() {
            crate::metrics::record_request(&method, &path, status, duration_us);
        }
        if crate::middleware::is_logger_enabled() {
            crate::middleware::log_request(&method, &path, status, duration_us / 1000);
        }
    }

    Ok(response)
}

/// Free all memory owned by a DooRequest.
/// String fields freed via doo_free (matching doo_alloc_string allocation).
/// HashMap fields freed via Box::from_raw.
/// DooRequest struct freed via libc::free (allocated with libc::malloc).
///
/// IMPORTANT: body and params may point to static C strings (EMPTY_C / EMPTY_JSON_C)
/// when they were empty — these must NOT be freed. We detect this by checking if
/// the pointer falls within the program's static data range (simple null/known-address check).
///
/// NOTE: Does NOT free the DooRequest struct itself — it's stack-allocated.
/// Only frees the heap-allocated fields (strings, boxes).
unsafe fn free_doo_request_fields(req: *mut DooRequest) {
    if req.is_null() {
        return;
    }

    // Use module-level static addresses for comparison
    let empty_ptr = EMPTY_C_STR.as_ptr() as *const c_char;
    let empty_json_ptr = EMPTY_JSON_C_STR.as_ptr() as *const c_char;

    // Free string fields (allocated via string_to_c → doo_alloc_string → libc::malloc)
    // Static method pointers (GET, POST, etc.) must NOT be freed
    if !(*req).method.is_null() && !is_static_method_ptr((*req).method) {
        doo_ffi_core::doo_free((*req).method as *mut u8);
    }
    if !(*req).path.is_null() {
        doo_ffi_core::doo_free((*req).path as *mut u8);
    }
    // body might be a static pointer — only free if heap-allocated
    if !(*req).body.is_null() && (*req).body != empty_ptr {
        doo_ffi_core::doo_free((*req).body as *mut u8);
    }
    if !(*req).user_id.is_null() {
        doo_ffi_core::doo_free((*req).user_id as *mut u8);
    }
    // Free Box<HashMap> fields
    if !(*req).headers.is_null() {
        let _ = Box::from_raw((*req).headers as *mut HashMap<String, String>);
    }
    // Params might be a static pointer — only free if heap-allocated
    if !(*req).params.is_null() && (*req).params != empty_json_ptr as *mut std::ffi::c_void {
        doo_ffi_core::doo_free((*req).params as *mut u8);
    }
    // Query is a Box<HashMap>
    if !(*req).query.is_null() {
        let _ = Box::from_raw((*req).query as *mut HashMap<String, String>);
    }
    // NOTE: Struct itself is NOT freed — it lives on the stack (MaybeUninit)
}

/// Apply CORS headers to a response using dynamic origin reflection.
/// Takes the request's Origin header as a borrowed &str (proper ownership, no RefCell).
/// Called at a single post-processing point in handle_request().
fn apply_cors_headers(response: &mut Response<Full<Bytes>>, request_origin: &str) {
    if let Some(cors) = crate::middleware::get_frozen_cors() {
        if let Some(origin_val) = cors.get_origin_for_request(request_origin) {
            let headers = response.headers_mut();
            headers.insert("Access-Control-Allow-Origin", origin_val);
            headers.insert("Access-Control-Allow-Methods", cors.methods.clone());
            headers.insert("Access-Control-Allow-Headers", cors.headers.clone());
            if cors.credentials {
                headers.insert(
                    "Access-Control-Allow-Credentials",
                    hyper::header::HeaderValue::from_static("true"),
                );
            }
            if let Some(ref max_age) = cors.max_age {
                headers.insert("Access-Control-Max-Age", max_age.clone());
            }
            // Vary: Origin — required when origin changes per request (not wildcard)
            if !cors.allow_all {
                headers.insert("Vary", hyper::header::HeaderValue::from_static("Origin"));
            }
        }
    }
}

/// Build HTTP response with frozen CORS headers (zero lock contention)
fn build_response(status: i32, body: &str) -> Response<Full<Bytes>> {
    build_response_bytes(status, body.as_bytes())
}

/// Build HTTP response from raw bytes — avoids intermediate String allocation.
/// Used on the hot path where we already have bytes (e.g. from CStr::to_bytes()).
fn build_response_bytes(status: i32, body: &[u8]) -> Response<Full<Bytes>> {
    build_response_bytes_typed(status, body, CONTENT_TYPE_JSON)
}

/// Build a 3xx redirect response with Location header.
/// Used by OAuth and any handler that returns a redirect via DooResult(tag>0, status=3xx).
fn build_redirect(status: i32, url: &str) -> Response<Full<Bytes>> {
    let status_code = StatusCode::from_u16(status as u16).unwrap_or(StatusCode::FOUND);

    let builder = Response::builder()
        .status(status_code)
        .header("Location", url);

    // CORS headers are NOT added here — they're applied once in handle_request()
    // via apply_cors_headers() with the request's Origin (proper ownership, no RefCell).

    builder
        .body(Full::new(Bytes::new()))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

/// Build a plaintext HTTP response — for benchmarks and non-JSON endpoints.
#[allow(dead_code)]
fn build_response_plaintext(status: i32, body: &[u8]) -> Response<Full<Bytes>> {
    build_response_bytes_typed(status, body, CONTENT_TYPE_PLAIN)
}

/// Build HTTP response with specified content type — the core builder.
/// Reused by both JSON and plaintext paths (zero code duplication).
fn build_response_bytes_typed(
    status: i32,
    body: &[u8],
    content_type: &str,
) -> Response<Full<Bytes>> {
    let status_code =
        StatusCode::from_u16(status as u16).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let builder = Response::builder()
        .status(status_code)
        .header("Content-Type", content_type);

    // CORS headers are NOT added here — they're applied once in handle_request()
    // via apply_cors_headers() with the request's Origin (proper ownership, no RefCell).

    builder
        .body(Full::new(Bytes::copy_from_slice(body)))
        .unwrap_or_else(|_| {
            Response::new(Full::new(Bytes::from_static(
                b"{\"error\":\"Response build failed\"}",
            )))
        })
}

/// Start the server with frozen routes and CORS for maximum throughput.
///
/// Production features:
/// - Graceful shutdown on SIGTERM/SIGINT (Docker, GKE, Fly.io)
/// - Connection limiting via Semaphore (prevents FD exhaustion)
/// - Per-request timeout
/// - Connection draining on shutdown
pub fn start_server(host: &str, port: u16) -> Result<(), String> {
    let _ = STARTUP_INSTANT.set(Instant::now());

    // Load configuration from environment variables
    let max_connections = env_usize("DOO_MAX_CONNECTIONS", 10_000);
    MAX_BODY_SIZE.store(env_usize("DOO_MAX_BODY_SIZE", 1_048_576), Ordering::Relaxed);
    REQUEST_TIMEOUT_MS.store(env_usize("DOO_REQUEST_TIMEOUT", 30_000), Ordering::Relaxed);

    // ========================================================================
    // FREEZE: Convert mutable registries to lock-free immutable state.
    // After this point, all request-time access is zero-cost (no locks).
    // ========================================================================
    freeze_routes();
    crate::middleware::freeze_cors();
    crate::middleware::freeze_logger();

    // Use the global Tokio runtime from doo_ffi_runtime (single source of truth).
    let local_runtime;
    let runtime: &tokio::runtime::Runtime = if doo_ffi_runtime::runtime::is_runtime_initialized() {
        doo_ffi_runtime::runtime::get_runtime()
    } else {
        doo_ffi_runtime::runtime::doo_runtime_init();
        if doo_ffi_runtime::runtime::is_runtime_initialized() {
            doo_ffi_runtime::runtime::get_runtime()
        } else {
            local_runtime = tokio::runtime::Runtime::new()
                .map_err(|e| format!("Failed to create runtime: {}", e))?;
            &local_runtime
        }
    };

    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|e| format!("Invalid address: {}", e))?;

    // Connection limiter — prevents FD exhaustion and OOM under load
    let conn_semaphore = std::sync::Arc::new(Semaphore::new(max_connections));

    runtime.block_on(async move {
        // On Linux, use SO_REUSEPORT for better multi-core distribution
        #[cfg(target_os = "linux")]
        let listener = {
            use socket2::{Domain, Protocol, Socket, Type};
            let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))
                .map_err(|e| format!("Failed to create socket: {}", e))?;
            socket.set_reuse_port(true).ok(); // Best-effort; not fatal if unsupported
            socket.set_reuse_address(true).ok();
            socket.set_nodelay(true).ok();
            socket.set_nonblocking(true)
                .map_err(|e| format!("Failed to set non-blocking: {}", e))?;
            socket.bind(&addr.into())
                .map_err(|e| format!("Failed to bind: {}", e))?;
            socket.listen(8192)
                .map_err(|e| format!("Failed to listen: {}", e))?;
            TcpListener::from_std(std::net::TcpListener::from(socket))
                .map_err(|e| format!("Failed to convert to tokio listener: {}", e))?
        };

        #[cfg(not(target_os = "linux"))]
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| format!("Failed to bind: {}", e))?;

        let boot_time = STARTUP_INSTANT
            .get()
            .map(|s| s.elapsed().as_millis())
            .unwrap_or(0);

        // Count registered routes from frozen registry
        let registry = get_frozen_routes();
        let total_routes = registry.count();
        let ws_routes = crate::ws::get_ws_registry().count();
        let has_ws_routes = ws_routes > 0;
        HAS_WS_ROUTES.store(has_ws_routes, Ordering::Relaxed);

        // Print banner (suppressible via DOO_NO_BANNER=1)
        let no_banner = std::env::var(doo_ffi_core::constants::ENV_DOO_NO_BANNER).map(|v| v == "1").unwrap_or(false);
        if !no_banner {
            // ANSI color: follows NO_COLOR standard (https://no-color.org/)
            use std::io::IsTerminal;
            let (cyan, rst) = if std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal() {
                ("\x1b[36m", "\x1b[0m")
            } else {
                ("", "")
            };
            println!("{}  ____              ", cyan);
            println!(" |  _ \\  ___   ___  ");
            println!(" | | | |/ _ \\ / _ \\ ");
            println!(" | |_| | (_) | (_) |");
            println!(" |____/ \\___/ \\___/          Doo v{}{}", VERSION, rst);
            println!("-------------------------------------------");
            println!("Info Server Online");
            println!("-------------------------------------------");
            println!("  Boot Time:            {} ms", boot_time);
            println!("  Listening on:         http://{}:{}", addr.ip(), port);
            println!("  Handlers Loaded:      {}", total_routes);
            if ws_routes > 0 {
                println!("  WebSocket Routes:     {}", ws_routes);
            }
            if crate::middleware::is_logger_enabled() {
                let cfg = crate::middleware::get_frozen_logger().unwrap();
                let mut levels = Vec::new();
                if cfg.info { levels.push("Info"); }
                if cfg.warn { levels.push("Warn"); }
                if cfg.error { levels.push("Error"); }
                println!("  Logger:               {}", levels.join(", "));
            }
            println!("  Process ID:           {}", std::process::id());
            // println!("  Max Connections:      {}", max_connections);
            // println!("  Max Body Size:        {} bytes", MAX_BODY_SIZE.load(Ordering::Relaxed));
            // println!("  Request Timeout:      {} ms", REQUEST_TIMEOUT_MS.load(Ordering::Relaxed));
            println!("-------------------------------------------");
        }
        // Always print the server started line (useful for container health checks)
        eprintln!("[Doo] Server started on http://{}:{} (pid={})", addr.ip(), port, std::process::id());

        // ====================================================================
        // Graceful shutdown signal handling
        // Listens for SIGTERM (Docker/GKE), SIGINT (Ctrl+C), and CancellationToken
        // ====================================================================
        let cancel_token = doo_ffi_runtime::runtime::get_cancel_token().clone();

        let shutdown_signal = async {
            // Platform-specific signal handling
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                let mut sigterm = signal(SignalKind::terminate())
                    .expect("Failed to register SIGTERM handler");
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        eprintln!("[Doo] Received SIGINT, shutting down gracefully...");
                    }
                    _ = sigterm.recv() => {
                        eprintln!("[Doo] Received SIGTERM, shutting down gracefully...");
                    }
                    _ = cancel_token.cancelled() => {
                        eprintln!("[Doo] Shutdown requested via runtime, shutting down...");
                    }
                }
            }
            #[cfg(not(unix))]
            {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        eprintln!("[Doo] Received Ctrl+C, shutting down gracefully...");
                    }
                    _ = cancel_token.cancelled() => {
                        eprintln!("[Doo] Shutdown requested via runtime, shutting down...");
                    }
                }
            }
        };

        // Pin the shutdown future
        tokio::pin!(shutdown_signal);

        // ====================================================================
        // Accept loop with graceful shutdown
        // ====================================================================
        loop {
            tokio::select! {
                biased; // Check shutdown first for faster drain

                _ = &mut shutdown_signal => {
                    // Shutdown signal received
                    DRAINING.store(true, Ordering::SeqCst);

                    // Signal runtime shutdown
                    doo_ffi_runtime::runtime::doo_runtime_shutdown();

                    // Wait for in-flight requests to drain (with timeout)
                    let timeout = Duration::from_millis(
                        doo_ffi_runtime::runtime::get_shutdown_timeout_ms()
                    );
                    let drain_start = Instant::now();

                    loop {
                        let active = ACTIVE_REQUESTS.load(Ordering::Relaxed);
                        if active == 0 {
                            eprintln!("[Doo] All connections drained, exiting.");
                            break;
                        }
                        if drain_start.elapsed() > timeout {
                            eprintln!(
                                "[Doo] Shutdown timeout reached with {} active requests, forcing exit.",
                                active
                            );
                            break;
                        }
                        eprintln!("[Doo] Draining... {} active requests remaining", active);
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }

                    return Ok(());
                }

                result = listener.accept() => {
                    let (stream, _) = result
                        .map_err(|e| format!("Accept failed: {}", e))?;

                    // TCP_NODELAY: disable Nagle's algorithm for lower latency
                    let _ = stream.set_nodelay(true);

                    // Acquire connection permit — backpressure under load
                    let permit = match conn_semaphore.clone().try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            // At connection limit — drop the TCP connection
                            // The client will get a connection reset, LB will retry
                            drop(stream);
                            continue;
                        }
                    };

                    let io = TokioIo::new(stream);

                    tokio::spawn(async move {
                        // Track active connection for graceful drain
                        ACTIVE_REQUESTS.fetch_add(1, Ordering::Relaxed);

                        let conn = http1::Builder::new()
                            .keep_alive(true)
                            .pipeline_flush(true)
                            .serve_connection(io, service_fn(handle_request));

                        // PERF: Only add .with_upgrades() when WebSocket routes exist.
                        // .with_upgrades() switches hyper to UpgradeableConnection which
                        // disables internal optimizations — ~20-30% RPS cost.
                        // Also: Do NOT wrap in tokio::time::timeout — it kills keep-alive
                        // connections after the timeout, causing socket read errors and
                        // thundering herd reconnects under load.
                        if has_ws_routes {
                            let _ = conn.with_upgrades().await;
                        } else {
                            let _ = conn.await;
                        }

                        // Release connection tracking
                        ACTIVE_REQUESTS.fetch_sub(1, Ordering::Relaxed);
                        // Release connection permit (Semaphore RAII)
                        drop(permit);
                    });
                }
            }
        }
    })
}
