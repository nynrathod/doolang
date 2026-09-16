//! # HTTP Client — `Fetch(url, options?)` for outbound HTTP requests
//!
//! Provides the Fetch API for making HTTP requests from Doo programs.
//! Designed for DooCloud inter-service communication and general-purpose
//! HTTP client usage.
//!
//! ## Doo syntax (ObjectLit options — like CORS middleware)
//! ```doo
//! // Simple GET (no options)
//! let res = Fetch("https://api.example.com/data");
//!
//! // POST with full options
//! let res = Fetch("https://api.example.com/users", {
//!     method: "POST",
//!     body: "{\"name\":\"John\"}",
//!     timeout: 30,
//!     headers: ["Content-Type: application/json", "Authorization: Bearer tok"]
//! });
//! ```
//!
//! ## Response (JSON string)
//! ```json
//! { "status": 200, "body": "...", "ok": true, "headers": { ... } }
//! ```
//!
//! ## Performance
//! - Global `reqwest::Client` with connection pooling (TCP reuse, TLS session cache)
//! - Thread-isolated execution to avoid nested-runtime panics in HTTP handlers
//!
//! ## Runtime Safety
//! Fetch runs on a dedicated OS thread with its own Tokio runtime, avoiding
//! "Cannot start a runtime from within a runtime" panics when called from
//! inside HTTP route handlers.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::panic;
use std::sync::OnceLock;
use std::time::Duration;

use doo_ffi_core::ffi_debug;

use crate::helpers::{c_to_string, string_to_c};
use crate::framework::map_ops::{doo_map_get_str, parse_json_i64_or_default, parse_json_string_or_default};

// ============================================================================
// Configuration Constants
// ============================================================================

/// Default request timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Maximum response body size (10 MB) to prevent OOM.
const MAX_RESPONSE_BODY_BYTES: usize = 10 * 1024 * 1024;

// ============================================================================
// Global HTTP Client — connection pooling for performance
// ============================================================================

/// Global shared reqwest::Client — created once, reused across all Fetch calls.
/// Provides TCP connection pooling, TLS session caching, and DNS reuse.
static GLOBAL_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Get or create the global HTTP client with optimized pool settings.
pub(crate) fn get_client() -> &'static reqwest::Client {
    GLOBAL_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_max_idle_per_host(32)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

// ============================================================================
// doo_http_fetch — FFI entry point for outbound HTTP requests
// ============================================================================

/// Make an outbound HTTP request.
///
/// # Parameters
/// - `url`: The request URL (C string).
/// - `options`: Pointer to `HashMap<String, String>` from ObjectLit codegen,
///   or null for default GET with no headers and 30s timeout.
///
/// # Options map keys (all optional)
/// - `method`: HTTP method (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS)
/// - `body`: Request body string
/// - `timeout`: Timeout in seconds (1–300)
/// - `headers`: Comma-separated "Key: Value" pairs (from array codegen)
///
/// # Returns
/// `*const c_char` — JSON string with response details:
/// ```json
/// { "status": 200, "body": "...", "ok": true, "headers": { ... } }
/// ```
/// On error, returns a JSON error string (never null).
#[no_mangle]
pub extern "C" fn doo_http_fetch(url: *const c_char, options: *mut c_void) -> *const c_char {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        // Parse URL
        let url_str = if url.is_null() {
            return string_to_c(
                r#"{"status":0,"body":"","ok":false,"error":"fetch: url is null"}"#,
            );
        } else {
            c_to_string(url)
        };

        if url_str.is_empty() {
            return string_to_c(
                r#"{"status":0,"body":"","ok":false,"error":"fetch: url is empty"}"#,
            );
        }

        ffi_debug!("FETCH", "fetch({}, ...)", url_str);

        // Parse options from HashMap (built by codegen from ObjectLit)
        let opts = parse_options_from_map(options);

        ffi_debug!(
            "FETCH",
            "method={}, timeout={}s, headers={}, body_len={}",
            opts.method,
            opts.timeout_secs,
            opts.headers.len(),
            opts.body.as_ref().map_or(0, |b| b.len())
        );

        // Execute the request on a dedicated thread to avoid nested-runtime panics
        let fetch_result = execute_fetch_on_thread(url_str, opts);

        match fetch_result {
            Ok(response_json) => string_to_c(&response_json),
            Err(e) => {
                let escaped = e.replace('\\', "\\\\").replace('"', "\\\"");
                string_to_c(&format!(
                    r#"{{"status":0,"body":"","ok":false,"error":"{}"}}"#,
                    escaped
                ))
            }
        }
    }));

    match result {
        Ok(ptr) => ptr,
        Err(_) => {
            string_to_c(r#"{"status":0,"body":"","ok":false,"error":"fetch: internal panic"}"#)
        }
    }
}

// ============================================================================
// Options Parsing — reads from HashMap<String, String> (ObjectLit codegen)
// ============================================================================

/// Parsed fetch options.
struct FetchOptions {
    method: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    timeout_secs: u64,
}

/// Parse options from a `HashMap<String, String>` pointer (ObjectLit codegen).
/// Returns sensible defaults for missing/null fields.
fn parse_options_from_map(options: *mut c_void) -> FetchOptions {
    let mut method = "GET".to_string();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut body: Option<String> = None;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;

    if options.is_null() {
        return FetchOptions {
            method,
            headers,
            body,
            timeout_secs,
        };
    }

    // Read method
    let method_ptr = doo_map_get_str(options, "method");
    if !method_ptr.is_null() {
        let m = parse_json_string_or_default(method_ptr, "GET");
        if !m.is_empty() {
            method = m.to_uppercase();
        }
    }

    // Read body
    let body_ptr = doo_map_get_str(options, "body");
    if !body_ptr.is_null() {
        let b = c_to_string(body_ptr);
        if !b.is_empty() {
            body = Some(b);
        }
    }

    // Read timeout (integer stored as string by codegen)
    let timeout_ptr = doo_map_get_str(options, "timeout");
    if !timeout_ptr.is_null() {
        let t = parse_json_i64_or_default(timeout_ptr, DEFAULT_TIMEOUT_SECS as i64);
        if t > 0 && t <= 300 {
            timeout_secs = t as u64;
        }
    }

    // Read headers — comma-separated "Key: Value" pairs (from array codegen)
    // Array ["Content-Type: application/json", "Authorization: Bearer tok"]
    // becomes "Content-Type: application/json,Authorization: Bearer tok" in the map
    let headers_ptr = doo_map_get_str(options, "headers");
    if !headers_ptr.is_null() {
        let h = c_to_string(headers_ptr);
        if !h.is_empty() {
            for pair in h.split(',') {
                let pair = pair.trim();
                if let Some(colon_pos) = pair.find(':') {
                    let key = pair[..colon_pos].trim().to_string();
                    let value = pair[colon_pos + 1..].trim().to_string();
                    if !key.is_empty() {
                        headers.push((key, value));
                    }
                }
            }
        }
    }

    FetchOptions {
        method,
        headers,
        body,
        timeout_secs,
    }
}

// ============================================================================
// Request Execution — dedicated thread + pooled client
// ============================================================================

/// Execute the HTTP request on a dedicated OS thread with its own Tokio runtime.
/// Uses the global pooled client for connection reuse.
fn execute_fetch_on_thread(url: String, opts: FetchOptions) -> Result<String, String> {
    // Get reference to the global pooled client before spawning the thread
    let client = get_client().clone();

    let handle = std::thread::spawn(move || execute_fetch_async(client, url, opts));

    match handle.join() {
        Ok(result) => result,
        Err(_) => Err("fetch: request thread panicked".to_string()),
    }
}

/// Async fetch implementation — runs inside the spawned thread's runtime.
/// Uses the provided (pooled) client for connection reuse.
fn execute_fetch_async(
    client: reqwest::Client,
    url: String,
    opts: FetchOptions,
) -> Result<String, String> {
    // Build a single-threaded Tokio runtime for this request
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("fetch: failed to create runtime: {}", e))?;

    rt.block_on(async move {
        // Build the request method
        let method = match opts.method.as_str() {
            "GET" => reqwest::Method::GET,
            "POST" => reqwest::Method::POST,
            "PUT" => reqwest::Method::PUT,
            "DELETE" => reqwest::Method::DELETE,
            "PATCH" => reqwest::Method::PATCH,
            "HEAD" => reqwest::Method::HEAD,
            "OPTIONS" => reqwest::Method::OPTIONS,
            other => reqwest::Method::from_bytes(other.as_bytes())
                .map_err(|_| format!("fetch: invalid HTTP method: {}", other))?,
        };

        // Build the request with per-request timeout
        let mut request = client
            .request(method, &url)
            .timeout(Duration::from_secs(opts.timeout_secs));

        // Add headers
        for (key, value) in &opts.headers {
            request = request.header(key.as_str(), value.as_str());
        }

        // Add body (for POST, PUT, PATCH)
        if let Some(ref body) = opts.body {
            request = request.body(body.clone());
        }

        // Send the request
        let response = request.send().await.map_err(|e| format_reqwest_error(&e))?;

        // Extract response data
        let status = response.status().as_u16();
        let ok = response.status().is_success();

        // Collect response headers
        let mut resp_headers = serde_json::Map::new();
        for (key, value) in response.headers().iter() {
            if let Ok(v) = value.to_str() {
                resp_headers.insert(
                    key.as_str().to_string(),
                    serde_json::Value::String(v.to_string()),
                );
            }
        }

        // Read body with size limit
        let body_bytes = response
            .bytes()
            .await
            .map_err(|e| format!("fetch: failed to read response body: {}", e))?;

        if body_bytes.len() > MAX_RESPONSE_BODY_BYTES {
            return Err(format!(
                "fetch: response body too large ({} bytes, max {})",
                body_bytes.len(),
                MAX_RESPONSE_BODY_BYTES
            ));
        }

        let body_str = String::from_utf8_lossy(&body_bytes).to_string();

        // Build response JSON
        let result = serde_json::json!({
            "status": status,
            "body": body_str,
            "ok": ok,
            "headers": resp_headers,
        });

        Ok(result.to_string())
    })
}

// ============================================================================
// Error Formatting
// ============================================================================

/// Format a reqwest error into a user-friendly message.
fn format_reqwest_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "fetch: request timed out".to_string()
    } else if e.is_connect() {
        format!("fetch: connection failed: {}", e)
    } else if e.is_redirect() {
        format!("fetch: too many redirects: {}", e)
    } else if e.is_builder() {
        format!("fetch: invalid request: {}", e)
    } else {
        format!("fetch: request failed: {}", e)
    }
}
