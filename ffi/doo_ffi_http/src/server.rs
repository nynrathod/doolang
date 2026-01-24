//! HTTP Server
//! Hyper-based HTTP server with request handling and middleware execution.

use crate::types::*;
use crate::router::get_routes;
use crate::helpers::*;
use crate::error::*;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::Instant;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper::body::{Incoming, Bytes};
use http_body_util::{BodyExt, Full};
use tokio::net::TcpListener;
use hyper_util::rt::TokioIo;

static STARTUP_INSTANT: OnceLock<Instant> = OnceLock::new();
const VERSION: &str = "0.4.0";

/// Handle incoming HTTP request
async fn handle_request(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    
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
    
    // Execute handler (with middleware chain)
    let handler = route_entry.handler;
    drop(registry);
    
    let result = handler(doo_request);
    
    // Process result
    let response = unsafe {
        if result.is_null() {
            let err = internal_error("Handler returned null", &path);
            build_response(500, &err.to_json())
        } else {
            let res = &*result;
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
                let status = get_last_error_status();
                let body = if !res.value.is_null() {
                    c_to_string(res.value as *const i8)
                } else {
                    get_last_error_json()
                };
                build_response(if status > 0 { status } else { 500 }, &body)
            }
        }
    };
    
    // Log request
    let elapsed = start.elapsed();
    let status_code = response.status().as_u16();
    println!("[Doo] {} | {} | {:>3}ms | {} {}", 
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
    let status_code = StatusCode::from_u16(status as u16)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    
    Response::builder()
        .status(status_code)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap_or_else(|_| {
            Response::new(Full::new(Bytes::from("{\"error\":\"Response build failed\"}")))
        })
}

/// Start the server
pub fn start_server(host: &str, port: u16) -> Result<(), String> {
    let _ = STARTUP_INSTANT.set(Instant::now());
    
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| format!("Failed to create runtime: {}", e))?;
    
    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|e| format!("Invalid address: {}", e))?;
    
    runtime.block_on(async move {
        let listener = TcpListener::bind(addr).await
            .map_err(|e| format!("Failed to bind: {}", e))?;
        
        let boot_time = STARTUP_INSTANT.get()
            .map(|s| s.elapsed().as_millis())
            .unwrap_or(0);
        
        // Print banner
        println!("\x1b[36m  ____              ");
        println!(" |  _ \\  ___   ___  ");
        println!(" | | | |/ _ \\ / _ \\ ");
        println!(" | |_| | (_) | (_) |");
        println!(" |____/ \\___/ \\___/          Doo v{}\x1b[0m", VERSION);
        println!("-------------------------------------------");
        println!("• Boot Time:            {} ms", boot_time);
        println!("• Listening on:         http://{}:{}", addr.ip(), port);
        println!("-------------------------------------------");
        println!("🚀 Server Started\n");
        
        loop {
            let (stream, _) = listener.accept().await
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
