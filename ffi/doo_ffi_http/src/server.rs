//! HTTP Server
//! Hyper-based HTTP server with request handling and middleware execution.
//!
//! Performance-critical path — every allocation and lock matters here.
//! Routes and CORS are frozen (lock-free) before the accept loop.

use crate::error::*;
use crate::helpers::*;
use crate::router::{freeze_routes, get_frozen_routes};
use crate::types::*;
use doo_ffi_core::ffi_debug;
use std::collections::HashMap;
use std::ffi::CString;
use std::net::SocketAddr;
use std::os::raw::c_char;
use std::sync::OnceLock;
use std::time::Instant;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

static STARTUP_INSTANT: OnceLock<Instant> = OnceLock::new();
const VERSION: &str = "0.4.0";

// ============================================================================
// MIDDLEWARE EXECUTION - Single Source of Truth
// Uses raw pointer + length to avoid Vec cloning per request.
// Safe because middleware slices live in the frozen RouteRegistry ('static).
// ============================================================================

/// Execute middleware chain then handler
/// This is the centralized middleware execution point - no duplication
fn execute_middleware_chain(
    req: *const DooRequest,
    middleware: &[DooMiddlewareFn],
    handler: DooHandlerFn,
) -> *mut DooResult {
    if middleware.is_empty() {
        // No middleware - call handler directly
        return handler(req);
    }

    // Build the next function chain using pointer-based context (no Vec clone)
    execute_middleware_at_index(req, middleware.as_ptr(), middleware.len(), 0, handler)
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

/// Thread-local context for middleware chain execution.
/// Uses raw pointer + length instead of owned Vec to avoid allocation.
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
/// Continues execution to the next middleware or handler — zero allocation
extern "C" fn middleware_next(req: *const DooRequest) -> *mut DooResult {
    MIDDLEWARE_CONTEXT.with(|ctx| {
        let ctx = ctx.borrow();
        let next_index = ctx.current_index + 1;
        let middleware_ptr = ctx.middleware_ptr;
        let middleware_len = ctx.middleware_len;
        let handler = ctx.handler.expect("Handler must be set");
        drop(ctx);

        execute_middleware_at_index(req, middleware_ptr, middleware_len, next_index, handler)
    })
}

/// Handle incoming HTTP request — hot path, every allocation matters
async fn handle_request(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    // Extract owned values before consuming req (method is 3-6 chars, negligible)
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query_raw = req.uri().query().unwrap_or("").to_string();

    // Built-in health check endpoint for server readiness (fast path)
    if path == "/health" && method == "GET" {
        return Ok(build_response(200, "{\"status\":\"ok\"}"));
    }

    // Handle CORS preflight (OPTIONS) requests before route matching
    // Uses frozen CORS — zero lock contention
    if method == "OPTIONS" {
        if crate::middleware::has_frozen_cors() {
            return Ok(build_response(204, ""));
        }
    }

    // ========================================================================
    // WebSocket Upgrade Detection
    // Must happen BEFORE body consumption — hyper::upgrade::on needs the request
    // ========================================================================
    let is_ws_upgrade = req
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
                        eprintln!("[Doo] WebSocket upgrade failed for {}: {}", path_clone, e);
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

    // Set thread-local request path for RFC 7807
    set_current_request_path(&path);
    clear_last_error();

    // ========================================================================
    // Route matching FIRST — fail fast before doing any expensive work.
    // Uses frozen registry, ZERO lock contention.
    // ========================================================================
    let registry = get_frozen_routes();

    let (route_entry, params) = match registry.match_route(&method, &path) {
        Some(r) => r,
        None => {
            // Check if path exists with other methods → 405 vs 404
            let allowed = registry.find_allowed_methods(&path);
            if !allowed.is_empty() {
                let err = Rfc7807Error::method_not_allowed(&method, &path, allowed);
                return Ok(build_response(405, &err.to_json()));
            }
            let err = Rfc7807Error::route_not_found(&method, &path);
            return Ok(build_response(404, &err.to_json()));
        }
    };

    // Get handler and middleware from frozen route entry
    let handler = route_entry.handler;
    let middleware = &route_entry.middleware;

    // ========================================================================
    // Extract headers into HashMap
    // ========================================================================
    let mut headers_map = HashMap::with_capacity(16);
    for (name, value) in req.headers() {
        if let Ok(v) = value.to_str() {
            headers_map.insert(name.to_string().to_lowercase(), v.to_string());
        }
    }

    // ========================================================================
    // Conditional body collection — skip for methods that don't send a body
    // ========================================================================
    let has_body_method = matches!(method.as_str(), "POST" | "PUT" | "PATCH");
    let body_str = if has_body_method {
        let body_bytes = req.collect().await?.to_bytes();
        if body_bytes.is_empty() {
            String::new()
        } else {
            match String::from_utf8(body_bytes.to_vec()) {
                Ok(s) => s,
                Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
            }
        }
    } else {
        // Consume the body stream (empty for GET/DELETE/HEAD)
        let _ = req.collect().await?;
        String::new()
    };

    // Parse query parameters
    let query_map: HashMap<String, String> = if !query_raw.is_empty() {
        parse_query(&query_raw)
    } else {
        HashMap::new()
    };

    // ========================================================================
    // Content-Type validation (POST/PUT/PATCH only)
    // ========================================================================
    if has_body_method && !body_str.is_empty() {
        if let Some(ct) = headers_map.get("content-type") {
            if !ct.contains("application/json") {
                let err = Rfc7807Error::wrong_content_type(&path, ct);
                return Ok(build_response(400, &err.to_json()));
            }
        }
    }

    // ========================================================================
    // Build DooRequest — direct heap allocation via libc::malloc
    // No stack intermediate, no memcpy — writes directly to heap pointer.
    // ========================================================================
    let params_json = if params.is_empty() {
        "{}".to_string()
    } else {
        serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string())
    };

    let doo_request = unsafe {
        let req_ptr = libc::malloc(std::mem::size_of::<DooRequest>()) as *mut DooRequest;
        (*req_ptr).method = string_to_c(&method);
        (*req_ptr).path = string_to_c(&path);
        (*req_ptr).body = string_to_c(&body_str);
        (*req_ptr).headers = Box::into_raw(Box::new(headers_map)) as *mut std::ffi::c_void;
        (*req_ptr).params = string_to_c(&params_json) as *mut std::ffi::c_void;
        (*req_ptr).query = Box::into_raw(Box::new(query_map)) as *mut std::ffi::c_void;
        (*req_ptr).user_id = std::ptr::null();
        req_ptr
    };

    // ========================================================================
    // Execute middleware chain + handler
    // ========================================================================
    doo_ffi_core::doo_json_clear_parse_error();

    let response = {
        ffi_debug!("SERVER", "About to execute handler for {} {}", method, path);
        let result = execute_middleware_chain(doo_request, middleware, handler);
        ffi_debug!(
            "SERVER",
            "Handler returned, result is_null={}",
            result.is_null()
        );

        // Check for JSON parse errors (type mismatches in arrays, etc.)
        if doo_ffi_core::doo_json_has_parse_error() {
            let status = doo_ffi_core::doo_json_get_parse_error_status();
            let error_json_ptr = doo_ffi_core::doo_json_get_parse_error_json();
            let body = c_to_string(error_json_ptr as *const i8);
            doo_ffi_core::doo_json_clear_parse_error();
            build_response(status, &body)
        } else {
            unsafe {
                if result.is_null() {
                    ffi_debug!("SERVER", "Handler returned null!");
                    let err = internal_error("Handler returned null", &path);
                    build_response(500, &err.to_json())
                } else {
                    let res = &*result;
                    ffi_debug!(
                        "SERVER",
                        "Result tag={}, value_is_null={}",
                        res.tag,
                        res.value.is_null()
                    );
                    if res.tag == 0 {
                        // Success
                        let body = if res.value.is_null() {
                            "{}".to_string()
                        } else {
                            c_to_string(res.value as *const i8)
                        };
                        build_response(200, &body)
                    } else {
                        // Error
                        if !res.value.is_null() {
                            let error_struct = res.value as *const i8;
                            let error_status = *(error_struct as *const i32);
                            let body_ptr =
                                *((error_struct as *const u8).add(8) as *const *const i8);
                            let body = if body_ptr.is_null() {
                                get_last_error_json()
                            } else {
                                c_to_string(body_ptr)
                            };
                            build_response(error_status, &body)
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

    // ========================================================================
    // Cleanup DooRequest — prevent memory leak
    // At high RPS, leaked requests cause OOM within minutes.
    // ========================================================================
    unsafe {
        free_doo_request(doo_request);
    }

    Ok(response)
}

/// Free all memory owned by a DooRequest.
/// CString fields freed via CString::from_raw, HashMap fields via Box::from_raw.
/// DooRequest struct freed via libc::free (allocated with libc::malloc).
unsafe fn free_doo_request(req: *mut DooRequest) {
    if req.is_null() {
        return;
    }
    // Free CString fields (allocated via string_to_c → CString::into_raw)
    if !(*req).method.is_null() {
        let _ = CString::from_raw((*req).method as *mut c_char);
    }
    if !(*req).path.is_null() {
        let _ = CString::from_raw((*req).path as *mut c_char);
    }
    if !(*req).body.is_null() {
        let _ = CString::from_raw((*req).body as *mut c_char);
    }
    if !(*req).user_id.is_null() {
        let _ = CString::from_raw((*req).user_id as *mut c_char);
    }
    // Free Box<HashMap> fields
    if !(*req).headers.is_null() {
        let _ = Box::from_raw((*req).headers as *mut HashMap<String, String>);
    }
    // Params is a CString (via string_to_c), cast to *mut c_void
    if !(*req).params.is_null() {
        let _ = CString::from_raw((*req).params as *mut c_char);
    }
    // Query is a Box<HashMap>
    if !(*req).query.is_null() {
        let _ = Box::from_raw((*req).query as *mut HashMap<String, String>);
    }
    // Free the struct itself (libc::malloc allocated)
    libc::free(req as *mut std::ffi::c_void);
}

/// Build HTTP response with frozen CORS headers (zero lock contention)
fn build_response(status: i32, body: &str) -> Response<Full<Bytes>> {
    let status_code =
        StatusCode::from_u16(status as u16).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let mut builder = Response::builder()
        .status(status_code)
        .header("Content-Type", CONTENT_TYPE_JSON);

    // Add CORS headers from frozen config — NO lock, NO contention
    if let Some(cors) = crate::middleware::get_frozen_cors() {
        builder = builder.header("Access-Control-Allow-Origin", cors.origin.as_str());
        builder = builder.header("Access-Control-Allow-Methods", cors.methods.as_str());
        builder = builder.header("Access-Control-Allow-Headers", cors.headers.as_str());
        if cors.credentials {
            builder = builder.header("Access-Control-Allow-Credentials", "true");
        }
        if let Some(ref max_age) = cors.max_age {
            builder = builder.header("Access-Control-Max-Age", max_age.as_str());
        }
    }

    // Zero-copy: Bytes::copy_from_slice avoids the body.to_string() allocation
    builder
        .body(Full::new(Bytes::copy_from_slice(body.as_bytes())))
        .unwrap_or_else(|_| {
            Response::new(Full::new(Bytes::from_static(
                b"{\"error\":\"Response build failed\"}",
            )))
        })
}

/// Start the server with frozen routes and CORS for maximum throughput
pub fn start_server(host: &str, port: u16) -> Result<(), String> {
    let _ = STARTUP_INSTANT.set(Instant::now());

    // ========================================================================
    // FREEZE: Convert mutable registries to lock-free immutable state.
    // After this point, all request-time access is zero-cost (no locks).
    // ========================================================================
    freeze_routes();
    crate::middleware::freeze_cors();

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

    runtime.block_on(async move {
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

        // Print banner
        println!("\x1b[36m  ____              ");
        println!(" |  _ \\  ___   ___  ");
        println!(" | | | |/ _ \\ / _ \\ ");
        println!(" | |_| | (_) | (_) |");
        println!(" |____/ \\___/ \\___/          Doo v{}\x1b[0m", VERSION);
        println!("-------------------------------------------");
        println!("Info Server Online");
        println!("-------------------------------------------");
        println!("• Boot Time:            {} ms", boot_time);
        println!("• Listening on:         http://{}:{}", addr.ip(), port);
        println!("• Handlers Loaded:      {}", total_routes);
        println!("• WebSocket Routes:     {}", ws_routes);
        println!("• Process ID:           {}", std::process::id());
        println!("-------------------------------------------");
        println!("🚀 Server Started on http://{}:{}\n", addr.ip(), port);

        loop {
            let (stream, _) = listener
                .accept()
                .await
                .map_err(|e| format!("Accept failed: {}", e))?;

            // TCP_NODELAY: disable Nagle's algorithm for lower latency
            let _ = stream.set_nodelay(true);

            let io = TokioIo::new(stream);

            tokio::spawn(async move {
                let _ = http1::Builder::new()
                    .serve_connection(io, service_fn(handle_request))
                    .with_upgrades()
                    .await;
            });
        }
    })
}
