//! HTTP Server
//! Hyper-based HTTP server with request handling and middleware execution.

use crate::error::*;
use crate::helpers::*;
use crate::router::get_routes;
use crate::types::*;
use doo_ffi_core::ffi_debug;
use std::collections::HashMap;
use std::net::SocketAddr;
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

    // Build the next function chain
    // Each middleware gets a "next" function that calls either the next middleware or the handler
    execute_middleware_at_index(req, middleware, 0, handler)
}

/// Execute middleware at given index, with next pointing to rest of chain
fn execute_middleware_at_index(
    req: *const DooRequest,
    middleware: &[DooMiddlewareFn],
    index: usize,
    handler: DooHandlerFn,
) -> *mut DooResult {
    if index >= middleware.len() {
        // All middleware executed, call the handler
        return handler(req);
    }

    let current_mw = middleware[index];

    // Create next function that continues the chain
    // We use a thread-local to pass the chain context since FFI can't capture closures
    MIDDLEWARE_CONTEXT.with(|ctx| {
        let mut ctx = ctx.borrow_mut();
        ctx.middleware = middleware.to_vec();
        ctx.current_index = index;
        ctx.handler = Some(handler);
    });

    // Store current request for next.call() to access
    set_current_request(req);

    // Call middleware with next function
    current_mw(req, middleware_next)
}

/// Thread-local context for middleware chain execution
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

#[derive(Default)]
struct MiddlewareContext {
    middleware: Vec<DooMiddlewareFn>,
    current_index: usize,
    handler: Option<DooHandlerFn>,
}

/// The "next" function passed to middleware
/// Continues execution to the next middleware or handler
extern "C" fn middleware_next(req: *const DooRequest) -> *mut DooResult {
    MIDDLEWARE_CONTEXT.with(|ctx| {
        let ctx = ctx.borrow();
        let next_index = ctx.current_index + 1;
        let middleware = ctx.middleware.clone();
        let handler = ctx.handler.expect("Handler must be set");
        drop(ctx);

        execute_middleware_at_index(req, &middleware, next_index, handler)
    })
}

/// Handle incoming HTTP request
async fn handle_request(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();

    // Built-in health check endpoint for server readiness
    if path == "/health" && method == "GET" {
        return Ok(build_response(200, r#"{"status":"ok"}"#));
    }

    // Handle CORS preflight (OPTIONS) requests before route matching
    // When CORS is configured, OPTIONS requests should return 204 with CORS headers
    // NOTE: Check config first, then drop lock BEFORE calling build_response
    // (build_response also locks cors_config to add headers — avoid deadlock)
    if method == "OPTIONS" {
        let has_cors = crate::middleware::get_cors_config()
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false);
        if has_cors {
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
            // Extract the Sec-WebSocket-Key for the accept handshake
            let ws_key = req
                .headers()
                .get("sec-websocket-key")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            let accept_key =
                tokio_tungstenite::tungstenite::handshake::derive_accept_key(ws_key.as_bytes());

            let path_clone = path.clone();

            // Spawn task that awaits the HTTP → WS upgrade from hyper
            tokio::spawn(async move {
                eprintln!("[Doo] WS: Awaiting upgrade for {}", path_clone);
                match hyper::upgrade::on(req).await {
                    Ok(upgraded) => {
                        eprintln!(
                            "[Doo] WS: Upgrade OK for {}, creating WebSocket stream",
                            path_clone
                        );
                        let io = TokioIo::new(upgraded);
                        let ws_stream = tokio_tungstenite::WebSocketStream::from_raw_socket(
                            io,
                            tokio_tungstenite::tungstenite::protocol::Role::Server,
                            None,
                        )
                        .await;
                        eprintln!(
                            "[Doo] WS: Stream ready, starting handler for {}",
                            path_clone
                        );
                        crate::ws::upgrade::handle_ws_connection(ws_stream, &path_clone).await;
                        eprintln!("[Doo] WS: Connection handler finished for {}", path_clone);
                    }
                    Err(e) => {
                        eprintln!("[Doo] WebSocket upgrade failed for {}: {}", path_clone, e);
                    }
                }
            });

            // Return 101 Switching Protocols — hyper completes the upgrade
            println!(
                "[Doo] {} | 101 | WebSocket | {}",
                chrono::Local::now().format("%H:%M:%S"),
                path,
            );

            let response = Response::builder()
                .status(StatusCode::SWITCHING_PROTOCOLS)
                .header("Upgrade", "websocket")
                .header("Connection", "Upgrade")
                .header("Sec-WebSocket-Accept", accept_key)
                .body(Full::new(Bytes::new()))
                .unwrap();

            return Ok(response);
        } else {
            // WS upgrade requested but no WS route registered for this path
            let err = Rfc7807Error::route_not_found("WS", &path);
            return Ok(build_response(404, &err.to_json()));
        }
    }

    // Set thread-local request path for RFC 7807
    set_current_request_path(&path);
    clear_last_error();

    // Extract headers
    let mut headers_map: HashMap<String, String> = HashMap::new();
    for (name, value) in req.headers() {
        if let Ok(v) = value.to_str() {
            headers_map.insert(name.to_string().to_lowercase(), v.to_string());
        }
    }

    // Extract body
    let body_bytes = req.collect().await?.to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();

    // Parse query parameters
    let query_map = parse_query(&query);

    // Match route
    let routes = get_routes();
    let registry = routes.lock().unwrap();
    ffi_debug!("SERVER", "Route registry has {} routes", registry.count());

    let (route_entry, params) = match registry.match_route(&method, &path) {
        Some(r) => r,
        None => {
            // Check if path exists with other methods → 405 vs 404
            let allowed = registry.find_allowed_methods(&path);
            drop(registry);
            if !allowed.is_empty() {
                // 405 Method Not Allowed
                let err = Rfc7807Error::method_not_allowed(&method, &path, allowed);
                return Ok(build_response(405, &err.to_json()));
            }
            // 404 Not Found
            let err = Rfc7807Error::route_not_found(&method, &path);
            return Ok(build_response(404, &err.to_json()));
        }
    };

    // ========================================================================
    // Request validation (Content-Type and JSON body)
    // Only for methods that send a body (POST, PUT, PATCH)
    // ========================================================================
    let has_body = matches!(method.as_str(), "POST" | "PUT" | "PATCH");
    if has_body && !body_str.is_empty() {
        // Check Content-Type header
        let content_type = headers_map.get("content-type");
        match content_type {
            None => {
                // Missing Content-Type — still allow if body is valid JSON
                // Some clients don't set Content-Type for simple JSON
            }
            Some(ct) if !ct.contains("application/json") => {
                let err = Rfc7807Error::wrong_content_type(&path, ct);
                return Ok(build_response(400, &err.to_json()));
            }
            _ => {} // Valid Content-Type
        }

        // Validate JSON body
        if !body_str.is_empty() {
            if let Err(_) = serde_json::from_str::<serde_json::Value>(&body_str) {
                let err = Rfc7807Error::malformed_json(&path);
                return Ok(build_response(400, &err.to_json()));
            }
        }
    }

    // Build DooRequest
    let doo_request = unsafe {
        let req_ptr = libc::malloc(std::mem::size_of::<DooRequest>()) as *mut DooRequest;
        if req_ptr.is_null() {
            let err = internal_error("Memory allocation failed", &path);
            return Ok(build_response(500, &err.to_json()));
        }

        // Convert params HashMap to JSON string for codegen compatibility
        // The codegen uses doo_json_get_field to extract path params
        let params_json = serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());

        (*req_ptr).method = string_to_c(&method);
        (*req_ptr).path = string_to_c(&path);
        (*req_ptr).body = string_to_c(&body_str);
        (*req_ptr).headers = Box::into_raw(Box::new(headers_map)) as *mut std::ffi::c_void;
        (*req_ptr).params = string_to_c(&params_json) as *mut std::ffi::c_void;
        (*req_ptr).query = Box::into_raw(Box::new(query_map)) as *mut std::ffi::c_void;
        (*req_ptr).user_id = std::ptr::null();

        req_ptr
    };

    // Execute middleware chain then handler
    // Middleware support: each middleware can call next() or return early
    let handler = route_entry.handler;
    let middleware_chain: Vec<_> = route_entry.middleware.clone();
    drop(registry);

    // Clear any previous parse errors before calling handler
    doo_ffi_core::doo_json_clear_parse_error();

    let response = {
        // Execute middleware chain, then handler
        ffi_debug!("SERVER", "About to execute handler for {} {}", method, path);
        let result = execute_middleware_chain(doo_request, &middleware_chain, handler);
        ffi_debug!(
            "SERVER",
            "Handler returned, result is_null={}",
            result.is_null()
        );

        // Check for JSON parse errors first (e.g., type mismatches in arrays)
        // This catches errors that occur during struct field parsing
        if doo_ffi_core::doo_json_has_parse_error() {
            let status = doo_ffi_core::doo_json_get_parse_error_status();
            let error_json_ptr = doo_ffi_core::doo_json_get_parse_error_json();
            let body = c_to_string(error_json_ptr as *const i8);
            // Clear the error after reading
            doo_ffi_core::doo_json_clear_parse_error();
            build_response(status, &body)
        } else {
            // Process result
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
                        // Success - value is the response body string
                        let body = if res.value.is_null() {
                            ffi_debug!("SERVER", "Success but value is null, using {{}}");
                            "{}".to_string()
                        } else {
                            let body_str = c_to_string(res.value as *const i8);
                            ffi_debug!(
                                "SERVER",
                                "Success with body len={}: {}",
                                body_str.len(),
                                &body_str[..body_str.len().min(200)]
                            );
                            body_str
                        };
                        build_response(200, &body)
                    } else {
                        // Error - value is a pointer to error response struct {i32 status, ptr body, ptr content_type}
                        if !res.value.is_null() {
                            // Extract status and body from the error response struct
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

    // Log request
    let elapsed = start.elapsed();
    let status_code = response.status().as_u16();
    println!(
        "[Doo] {} | {} | {:>3}ms | {} {}",
        chrono::Local::now().format("%H:%M:%S"),
        status_code,
        elapsed.as_millis(),
        method,
        path
    );

    // Cleanup (TODO: proper memory management)

    Ok(response)
}

fn build_response(status: i32, body: &str) -> Response<Full<Bytes>> {
    let status_code =
        StatusCode::from_u16(status as u16).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let mut builder = Response::builder()
        .status(status_code)
        .header("Content-Type", CONTENT_TYPE_JSON);

    // Add CORS headers if CORS is configured (single source of truth via get_cors_config)
    if let Ok(guard) = crate::middleware::get_cors_config().lock() {
        if let Some(config) = guard.as_ref() {
            builder = builder.header("Access-Control-Allow-Origin", config.origins.join(", "));
            builder = builder.header("Access-Control-Allow-Methods", config.methods.join(", "));
            builder = builder.header("Access-Control-Allow-Headers", config.headers.join(", "));
            if config.credentials {
                builder = builder.header("Access-Control-Allow-Credentials", "true");
            }
            if let Some(max_age) = config.max_age {
                builder = builder.header("Access-Control-Max-Age", max_age.to_string());
            }
        }
    }

    builder
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap_or_else(|_| {
            Response::new(Full::new(Bytes::from(
                "{\"error\":\"Response build failed\"}",
            )))
        })
}

/// Start the server
pub fn start_server(host: &str, port: u16) -> Result<(), String> {
    let _ = STARTUP_INSTANT.set(Instant::now());

    // Use the global Tokio runtime from doo_ffi_runtime (single source of truth).
    // Falls back to creating a local runtime if the global one isn't initialized
    // (e.g., when HTTP server is started without async features in main).
    let local_runtime;
    let runtime: &tokio::runtime::Runtime = if doo_ffi_runtime::runtime::is_runtime_initialized() {
        doo_ffi_runtime::runtime::get_runtime()
    } else {
        // Initialize global runtime if not yet done
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

        // Count registered routes
        let routes = get_routes();
        let total_routes = routes.lock().map(|r| r.count()).unwrap_or(0);
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
