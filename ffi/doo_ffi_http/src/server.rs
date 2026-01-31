//! HTTP Server
//! Hyper-based HTTP server with request handling and middleware execution.

use crate::error::*;
use crate::helpers::*;
use crate::router::get_routes;
use crate::types::*;
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

    let (route_entry, params) = match registry.match_route(&method, &path) {
        Some(r) => r,
        None => {
            drop(registry);
            // 404 Not Found
            let err = not_found(&format!("No route for {} {}", method, path), &path);
            return Ok(build_response(404, &err.to_json()));
        }
    };

    // Build DooRequest
    let doo_request = unsafe {
        let req_ptr = libc::malloc(std::mem::size_of::<DooRequest>()) as *mut DooRequest;
        if req_ptr.is_null() {
            let err = internal_error("Memory allocation failed", &path);
            return Ok(build_response(500, &err.to_json()));
        }

        (*req_ptr).method = string_to_c(&method);
        (*req_ptr).path = string_to_c(&path);
        (*req_ptr).body = string_to_c(&body_str);
        (*req_ptr).headers = Box::into_raw(Box::new(headers_map)) as *mut std::ffi::c_void;
        (*req_ptr).params = Box::into_raw(Box::new(params.clone())) as *mut std::ffi::c_void;
        (*req_ptr).query = Box::into_raw(Box::new(query_map)) as *mut std::ffi::c_void;
        (*req_ptr).user_id = std::ptr::null();

        req_ptr
    };

    // Execute handler - all handlers have unified signature thanks to compiler wrappers
    let handler = route_entry.handler;
    drop(registry);

    // Clear any previous parse errors before calling handler
    doo_ffi_core::doo_json_clear_parse_error();

    let response = {
        let result = handler(doo_request);

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
                    let err = internal_error("Handler returned null", &path);
                    build_response(500, &err.to_json())
                } else {
                    let res = &*result;
                    if res.tag == 0 {
                        // Success - value is the response body string
                        let body = if res.value.is_null() {
                            "{}".to_string()
                        } else {
                            c_to_string(res.value as *const i8)
                        };
                        build_response(200, &body)
                    } else {
                        // Error
                        let status = get_last_error_status();
                        let body = if !res.value.is_null() {
                            c_to_string(res.value as *const i8)
                        } else {
                            get_last_error_json()
                        };
                        build_response(if status > 0 { status } else { 500 }, &body)
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

    Response::builder()
        .status(status_code)
        .header("Content-Type", "application/json")
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

    let runtime =
        tokio::runtime::Runtime::new().map_err(|e| format!("Failed to create runtime: {}", e))?;

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
                    .await;
            });
        }
    })
}
