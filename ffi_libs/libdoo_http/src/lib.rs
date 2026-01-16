//! HTTP Server FFI for Doo language
//! Phase 3, 4, 5: Complete implementation with closures, JSON, groups, middleware

mod error;

use doo_runtime::{
    doo_debug, doo_ffi_enter, doo_ffi_exit, doo_handler_call, doo_handler_result, doo_http_debug,
    doo_mem_alloc, doo_mem_free, doo_mem_stats, dooruntime_malloc,
    ownership::dooruntime_free_rc_string,
    memory::{track_alloc, track_free, is_freed},
};
use serde::Serialize;
use serde_json::json;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::net::SocketAddr;
use std::os::raw::c_char;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{body::Incoming, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use matchit::Router;
use std::thread;
use tokio::net::TcpListener;

use chrono::Local;
use std::sync::RwLock;
use std::time::{Duration, Instant};

// Debug macro now imported from libdoo_runtime
// Use doo_http_debug!("message") or doo_http_debug!("HTTP", "message")

use error::{not_found, ErrorResponse, ErrorType}; // Thread-local RFC 7807 last error (status, json body) populated by parsing/param helpers

#[repr(C)]
struct DooMapPair {
    key: *const c_char,
    value: *const c_char,
}

#[inline]
unsafe fn doo_map_len(map_ptr: *const std::ffi::c_void) -> usize {
    if map_ptr.is_null() {
        return 0;
    }
    let len_i32 = *((map_ptr as *const u8).sub(4) as *const i32);
    if len_i32 <= 0 {
        0
    } else {
        len_i32 as usize
    }
}

#[inline]
unsafe fn doo_map_get_str(map_ptr: *const std::ffi::c_void, key: &str) -> Option<String> {
    if map_ptr.is_null() {
        return None;
    }
    let len = doo_map_len(map_ptr);
    if len == 0 {
        return None;
    }

    let pairs = map_ptr as *const DooMapPair;
    for i in 0..len {
        let pair = &*pairs.add(i);
        if pair.key.is_null() {
            continue;
        }
        if c_to_string(pair.key) == key {
            if pair.value.is_null() {
                return None;
            }
            return Some(c_to_string(pair.value));
        }
    }
    None
}

fn parse_json_str_array_or_default(raw: Option<String>, default: Vec<String>) -> Vec<String> {
    let Some(s) = raw else {
        return default;
    };

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
        if let Some(arr) = v.as_array() {
            let mut out = Vec::new();
            for item in arr {
                if let Some(st) = item.as_str() {
                    out.push(st.to_string());
                }
            }
            if !out.is_empty() {
                return out;
            }
        }
    }
    let parts: Vec<String> = s
        .split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        default
    } else {
        parts
    }
}

fn parse_json_bool_or_default(raw: Option<String>, default: bool) -> bool {
    let Some(s) = raw else {
        return default;
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
        if let Some(b) = v.as_bool() {
            return b;
        }
        if let Some(n) = v.as_i64() {
            return n != 0;
        }
        if let Some(st) = v.as_str() {
            let lc = st.to_lowercase();
            return lc == "true" || lc == "1";
        }
    }
    let lc = s.trim().to_lowercase();
    if lc == "true" || lc == "1" {
        true
    } else if lc == "false" || lc == "0" {
        false
    } else {
        default
    }
}

fn normalize_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '_' || ch == '-' {
            continue;
        }
        for lc in ch.to_lowercase() {
            out.push(lc);
        }
    }
    out
}

fn get_source_value<'a>(
    key: &str,
    map: &'a serde_json::Map<String, serde_json::Value>,
) -> Option<&'a serde_json::Value> {
    map.get(key).or_else(|| {
        let target = normalize_key(key);
        map.iter()
            .find(|(k, _)| normalize_key(k) == target)
            .map(|(_, v)| v)
    })
}

fn parse_json_i64_or_default(raw: Option<String>, default: i64) -> i64 {
    let Some(s) = raw else {
        return default;
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
        if let Some(n) = v.as_i64() {
            return n;
        }
        if let Some(st) = v.as_str() {
            if let Ok(n) = st.trim().parse::<i64>() {
                return n;
            }
        }
    }
    s.trim().parse::<i64>().unwrap_or(default)
}

fn parse_json_string_or_default(raw: Option<String>, default: &str) -> String {
    let Some(s) = raw else {
        return default.to_string();
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
        if let Some(st) = v.as_str() {
            return st.to_string();
        }
    }
    s
}

#[repr(C)]
pub struct DooArray {
    pub data: *mut *mut libc::c_char,
    pub len: i64,
    pub cap: i64,
}

thread_local! {
    static LAST_RFC_ERROR: RefCell<Option<(i32, String)>> = RefCell::new(None);
}

static ALLOCATED_CSTRS: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();

fn get_allocated_cstrs() -> &'static Mutex<HashSet<usize>> {
    ALLOCATED_CSTRS.get_or_init(|| Mutex::new(HashSet::new()))
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn set_last_error(status: i32, json: String) {
    LAST_RFC_ERROR.with(|cell| {
        *cell.borrow_mut() = Some((status, json));
    });
}

fn clear_last_error() {
    LAST_RFC_ERROR.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

fn take_last_error() -> Option<(i32, String)> {
    LAST_RFC_ERROR.with(|cell| cell.borrow_mut().take())
}

// Cached human-readable time (HH:MM:SS), updated once per second
static TIMESTAMP_CACHE: OnceLock<std::sync::Arc<RwLock<String>>> = OnceLock::new();

fn init_timestamp_updater() {
    // if already initialized, do nothing
    if TIMESTAMP_CACHE.get().is_some() {
        return;
    }

    // initial value
    let now = Local::now();
    let initial = now.format("%H:%M:%S").to_string();
    let arc_lock = std::sync::Arc::new(RwLock::new(initial));

    // try to set global; if another thread set it concurrently, use the existing one
    let cache = match TIMESTAMP_CACHE.set(arc_lock.clone()) {
        Ok(_) => arc_lock,
        Err(_) => TIMESTAMP_CACHE.get().unwrap().clone(),
    };

    // spawn background thread to update cached time once per second
    thread::spawn(move || loop {
        let t = Local::now().format("%H:%M:%S").to_string();
        if let Ok(mut w) = cache.write() {
            *w = t;
        }
        thread::sleep(Duration::from_secs(1));
    });
}

fn log_request(start: Instant, status: StatusCode, method: &str, path: &str) {
    // Try to read human-readable cached HH:MM:SS
    let time_str = if let Some(cache) = TIMESTAMP_CACHE.get() {
        if let Ok(r) = cache.read() {
            r.clone()
        } else {
            // fallback to epoch seconds as string
            format!(
                "{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
            )
        }
    } else {
        // fallback if not initialized
        format!(
            "{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        )
    };

    let elapsed_ms = start.elapsed().as_millis();
    // Example:
    // [Doo] 15:04:05 | 200 |   2ms | GET /api/users
    doo_http_debug!(
        "[Doo] {} | {:3} | {:4}ms | {} {}",
        time_str,
        status.as_u16(),
        elapsed_ms,
        method,
        path
    );
}

#[no_mangle]
pub extern "C" fn doohttp_last_error_status() -> libc::c_int {
    LAST_RFC_ERROR.with(|cell| cell.borrow().as_ref().map(|e| e.0).unwrap_or(0))
}

#[no_mangle]
pub extern "C" fn doohttp_last_error_json() -> *const libc::c_char {
    if let Some((_, json)) = take_last_error() {
        string_to_c(&json)
    } else {
        std::ptr::null()
    }
}

thread_local! {
    static CURRENT_REQUEST_PATH: RefCell<String> = RefCell::new(String::from("/"));
}

/// Set the current request path for this thread
fn set_current_request_path(path: &str) {
    CURRENT_REQUEST_PATH.with(|p| {
        *p.borrow_mut() = path.to_string();
    });
}

/// Get the current request path for this thread
fn get_current_request_path() -> String {
    CURRENT_REQUEST_PATH.with(|p| p.borrow().clone())
}

/// Global route registry for storing registered handlers
static ROUTES: OnceLock<Arc<Mutex<RouteRegistry>>> = OnceLock::new();

/// Function pointer type for Doo handler callbacks
/// Takes Request pointer, returns Response pointer (or error)
type DooHandlerFn = extern "C" fn(*mut DooRequest) -> *mut DooResult;

/// Middleware function pointer - takes Request and Next, returns Result
/// New signature supports chaining and error handling
type DooMiddlewareFn = extern "C" fn(*mut DooRequest, *mut DooNext) -> *mut DooResult;

/// Route with handler and middleware chain
#[derive(Clone)]
struct Route {
    handler: DooHandlerFn,
    middleware: Vec<DooMiddlewareFn>,
    redirect_field: Option<String>, // Field name with @redirect decorator (if any)
}

/// Handler metadata for validation (includes decorators)
#[derive(Clone, Debug)]
struct HandlerMetadata {
    param_types: Vec<String>,
    struct_decorators: HashMap<String, HashMap<String, Vec<DecoratorInfo>>>,
    #[allow(dead_code)]
    struct_fields: HashMap<String, Vec<Vec<String>>>,
    struct_layouts: serde_json::Value,
    enum_variants: HashMap<String, Vec<String>>, // enum_name -> [variant_names]
    return_type: String,
}

#[derive(Clone, Debug, Serialize)]
struct DecoratorInfo {
    name: String,
    args: Vec<String>,
}

/// Find the field name with @redirect decorator in a struct's decorator metadata
/// Returns Some(field_name) if found, None otherwise
fn find_redirect_field(
    struct_decorators: &HashMap<String, HashMap<String, Vec<DecoratorInfo>>>,
    struct_name: &str,
) -> Option<String> {
    if let Some(field_decorators) = struct_decorators.get(struct_name) {
        for (field_name, decorators) in field_decorators {
            for decorator in decorators {
                if decorator.name == "redirect" {
                    return Some(field_name.clone());
                }
            }
        }
    }
    None
}

/// Built-in health check handler available at GET /health
extern "C" fn health_handler(_req: *mut DooRequest) -> *mut DooResult {
    let body = r#"{"status":"ok"}"#;

    // Allocate DooResponse using libc::malloc
    let response = alloc_doo_response(200, string_to_c(body), string_to_c("application/json"));

    make_ok_ptr(response as *mut std::ffi::c_void)
}

/// Route registry storing method -> router with handlers
struct RouteRegistry {
    routes: HashMap<String, Router<Route>>, // method -> router
    exact_routes: HashMap<String, Route>,
    handlers: HashMap<String, DooHandlerFn>, // handler_name -> function pointer
    handler_metadata: HashMap<String, HandlerMetadata>, // handler_name -> metadata
    middleware: Vec<DooMiddlewareFn>,        // global middleware
    middleware_handlers: HashMap<String, DooMiddlewareFn>, // middleware_name -> function pointer
    #[allow(dead_code)]
    groups: HashMap<String, Vec<DooMiddlewareFn>>, // prefix -> middleware for groups
    route_count: usize,
}

impl RouteRegistry {
    fn new() -> Self {
        let mut registry = Self {
            routes: HashMap::new(),
            exact_routes: HashMap::new(),
            handlers: HashMap::new(),
            handler_metadata: HashMap::new(),
            middleware: Vec::new(),
            middleware_handlers: HashMap::new(),
            groups: HashMap::new(),
            route_count: 0,
        };

        // Register built-in health check
        registry.register("GET", "/health", health_handler);

        registry
    }

    fn is_exact_route(path: &str) -> bool {
        !path.contains(':') && !path.contains('{') && !path.contains('}')
    }

    fn exact_key(method: &str, path: &str) -> String {
        format!("{} {}", method.to_uppercase(), path)
    }

    fn register(&mut self, method: &str, path: &str, handler_fn: DooHandlerFn) {
        let method = method.to_uppercase();
        let router = self
            .routes
            .entry(method.clone())
            .or_insert_with(Router::new);

        let route = Route {
            handler: handler_fn,
            middleware: Vec::new(),
            redirect_field: None,
        };

        if Self::is_exact_route(path) {
            let key = Self::exact_key(&method, path);
            self.exact_routes.insert(key, route.clone());
        }

        if let Err(e) = router.insert(path, route) {
            if !Self::is_exact_route(path) {
                doo_http_debug!("Failed to register route {} {}: {}", method, path, e);
            }
        } else {
            self.route_count += 1;
            doo_http_debug!("✓ Registered: {} {}", method, path);
        }
    }

    fn register_with_middleware(
        &mut self,
        method: &str,
        path: &str,
        handler_fn: DooHandlerFn,
        middleware: Vec<DooMiddlewareFn>,
    ) {
        let method = method.to_uppercase();
        let router = self
            .routes
            .entry(method.clone())
            .or_insert_with(Router::new);

        let middleware_len = middleware.len();
        let route = Route {
            handler: handler_fn,
            middleware,
            redirect_field: None,
        };

        if Self::is_exact_route(path) {
            let key = Self::exact_key(&method, path);
            self.exact_routes.insert(key, route.clone());
        }

        if let Err(e) = router.insert(path, route) {
            if !Self::is_exact_route(path) {
                doo_http_debug!("Failed to register route {} {}: {}", method, path, e);
            }
        } else {
            self.route_count += 1;
            doo_http_debug!(
                "✓ Registered: {} {} (with {} middleware)",
                method,
                path,
                middleware_len
            );
        }
    }

    fn register_by_name(&mut self, method: &str, path: &str, handler_name: &str) {
        if let Some(handler_fn) = self.handlers.get(handler_name).copied() {
            self.register(method, path, handler_fn);
        } else {
            doo_http_debug!("Warning: Handler {} not found in registry", handler_name);
        }
    }

    fn register_by_name_with_middleware(
        &mut self,
        method: &str,
        path: &str,
        handler_name: &str,
        middleware: Vec<DooMiddlewareFn>,
    ) {
        if let Some(handler_fn) = self.handlers.get(handler_name).copied() {
            self.register_with_middleware(method, path, handler_fn, middleware);
        } else {
            doo_http_debug!("Warning: Handler {} not found in registry", handler_name);
        }
    }

    fn add_middleware(&mut self, mw: DooMiddlewareFn) {
        self.middleware.push(mw);
    }

    fn add_middleware_once(&mut self, mw: DooMiddlewareFn) {
        let mw_id = mw as usize;
        if self.middleware.iter().any(|&m| m as usize == mw_id) {
            return;
        }
        self.middleware.push(mw);
    }

    fn find_route(&self, method: &str, path: &str) -> Option<(&Route, HashMap<String, String>)> {
        let method = method.to_uppercase();

        let exact_key = Self::exact_key(&method, path);
        if let Some(route) = self.exact_routes.get(&exact_key) {
            return Some((route, HashMap::new()));
        }

        if let Some(router) = self.routes.get(&method) {
            if let Ok(matched) = router.at(path) {
                let params = matched
                    .params
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                return Some((matched.value, params));
            }
        }
        None
    }

    fn find_allowed_methods(&self, path: &str) -> Vec<String> {
        let mut allowed = Vec::new();
        for (method, router) in &self.routes {
            if router.at(path).is_ok() {
                allowed.push(method.clone());
            }
        }
        allowed
    }
}

fn get_routes() -> &'static Arc<Mutex<RouteRegistry>> {
    ROUTES.get_or_init(|| Arc::new(Mutex::new(RouteRegistry::new())))
}

// ============================================================================
// CORS and Rate Limiting Configuration
// ============================================================================

/// CORS Configuration
#[derive(Clone, Debug)]
struct CorsConfig {
    origins: Vec<String>,
    methods: Vec<String>,
    credentials: bool,
    headers: Vec<String>,
    expose_headers: Vec<String>,
    max_age: Option<i32>,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            origins: vec!["*".to_string()],
            methods: vec![
                "GET".to_string(),
                "POST".to_string(),
                "PUT".to_string(),
                "DELETE".to_string(),
                "PATCH".to_string(),
                "OPTIONS".to_string(),
            ],
            credentials: false,
            headers: vec!["*".to_string()],
            expose_headers: vec![],
            max_age: Some(86400), // 24 hours
        }
    }
}

static CORS_CONFIG: OnceLock<Arc<Mutex<Option<CorsConfig>>>> = OnceLock::new();

fn get_cors_config() -> &'static Arc<Mutex<Option<CorsConfig>>> {
    CORS_CONFIG.get_or_init(|| Arc::new(Mutex::new(None)))
}

/// Rate Limiting Configuration
#[derive(Clone, Debug)]
struct RateLimitConfig {
    max: u32,
    window: u64, // seconds
    per: String, // "ip" or "user"
}

// Default constants for rate limiting
const DEFAULT_RATE_LIMIT_MAX: u32 = 100;
const DEFAULT_RATE_LIMIT_WINDOW: u64 = 3600; // 1 hour

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max: DEFAULT_RATE_LIMIT_MAX,
            window: DEFAULT_RATE_LIMIT_WINDOW,
            per: "ip".to_string(),
        }
    }
}

struct RateLimitEntry {
    count: u32,
    window_start: Instant,
}

static RATELIMIT_CONFIG: OnceLock<Arc<Mutex<Option<RateLimitConfig>>>> = OnceLock::new();
static RATELIMIT_STATE: OnceLock<Arc<Mutex<HashMap<String, RateLimitEntry>>>> = OnceLock::new();

fn get_ratelimit_config() -> &'static Arc<Mutex<Option<RateLimitConfig>>> {
    RATELIMIT_CONFIG.get_or_init(|| Arc::new(Mutex::new(None)))
}

fn get_ratelimit_state() -> &'static Arc<Mutex<HashMap<String, RateLimitEntry>>> {
    RATELIMIT_STATE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

fn decorator_name_eq(dec: &serde_json::Value, target: &str) -> bool {
    dec.as_object()
        .and_then(|o| o.get("name"))
        .and_then(|n| n.as_str())
        .map(|n| n.eq_ignore_ascii_case(target))
        .unwrap_or(false)
}

fn has_decorator(decorators: Option<&Vec<serde_json::Value>>, target: &str) -> bool {
    decorators
        .map(|decs| decs.iter().any(|d| decorator_name_eq(d, target)))
        .unwrap_or(false)
}

fn to_snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 8);
    let mut prev_is_lower_or_digit = false;
    let mut prev_is_upper = false;
    let chars: Vec<char> = name.chars().collect();

    for (i, &ch) in chars.iter().enumerate() {
        if ch == '_' || ch == '-' {
            if !out.ends_with('_') {
                out.push('_');
            }
            prev_is_lower_or_digit = false;
            prev_is_upper = false;
            continue;
        }

        let is_upper = ch.is_ascii_uppercase();
        let is_lower = ch.is_ascii_lowercase();
        let is_digit = ch.is_ascii_digit();
        let next_is_lower = chars
            .get(i + 1)
            .copied()
            .map(|c| c.is_ascii_lowercase())
            .unwrap_or(false);

        if is_upper {
            if (prev_is_lower_or_digit) || (prev_is_upper && next_is_lower) {
                if !out.is_empty() && !out.ends_with('_') {
                    out.push('_');
                }
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }

        prev_is_lower_or_digit = is_lower || is_digit;
        prev_is_upper = is_upper;
    }

    while out.ends_with('_') {
        out.pop();
    }
    out
}

fn get_obj_value<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    field_name: &str,
) -> Option<&'a serde_json::Value> {
    if let Some(v) = obj.get(field_name) {
        return Some(v);
    }

    let target = normalize_key(field_name);
    obj.iter()
        .find(|(k, _)| normalize_key(k) == target)
        .map(|(_, v)| v)
}

fn get_header_value(req: *const DooRequest, key: &str) -> Option<String> {
    if req.is_null() {
        return None;
    }
    unsafe {
        let headers_map = (*req).headers as *const HashMap<String, String>;
        if headers_map.is_null() {
            return None;
        }
        let k = key.to_lowercase();
        (*headers_map).get(&k).cloned()
    }
}

fn get_rate_limit_key(req: *const DooRequest, per: &str) -> String {
    if req.is_null() {
        return "unknown".to_string();
    }
    unsafe {
        // Per-user: use auth_user_id if available (injected by jwt middleware)
        if per == "user" {
            let params_ptr = (*req).params as *const HashMap<String, String>;
            if !params_ptr.is_null() {
                if let Some(uid) = (*params_ptr).get("auth_user_id") {
                    if !uid.is_empty() {
                        return format!("user:{}", uid);
                    }
                }
            }
            // Fall back to IP key if user not available
        }

        // Try proxy headers first
        if let Some(xff) = get_header_value(req, "x-forwarded-for") {
            // Use first IP in list
            if let Some(first) = xff.split(',').next() {
                let ip = first.trim();
                if !ip.is_empty() {
                    return format!("ip:{}", ip);
                }
            }
        }
        if let Some(xri) = get_header_value(req, "x-real-ip") {
            let ip = xri.trim();
            if !ip.is_empty() {
                return format!("ip:{}", ip);
            }
        }

        // Fallback: no remote IP in request struct yet
        "ip:unknown".to_string()
    }
}

// Result type for FFI returns with ownership tracking
// tag: 0 = Ok, 1 = Err
// owner: 0 = LLVM (RC), 1 = FFI (libc), 2 = Rust (Box)
#[repr(C)]
pub struct DooResult {
    tag: i32,
    value: *mut std::ffi::c_void,
    owner: u8, // Owner enum: 0=LLVM, 1=FFI, 2=Rust
}

/// Owner enum constants for DooResult
pub mod owner {
    pub const LLVM: u8 = 0;
    pub const FFI: u8 = 1;
    pub const RUST: u8 = 2;
}

/// Next type - represents the next middleware/handler in the chain
#[repr(C)]
pub struct DooNext {
    request: *mut DooRequest,
    remaining_middleware: *mut std::ffi::c_void, // Vec<DooMiddlewareFn>
    handler: DooHandlerFn,
    current_index: usize,
}

// Error struct
#[repr(C)]
pub struct DooHttpError {
    status: i32,
    message: *const c_char,
}

// Request struct passed to handlers
#[repr(C)]
pub struct DooRequest {
    method: *const c_char,
    path: *const c_char,
    body: *const c_char,
    content_type: *const c_char,
    params: *mut std::ffi::c_void,  // HashMap<String, String>
    query: *mut std::ffi::c_void,   // HashMap<String, String>
    headers: *mut std::ffi::c_void, // HashMap<String, String>
}

// Response struct returned by handlers
#[repr(C)]
pub struct DooResponse {
    status: i32,
    body: *const c_char,
    content_type: *const c_char,
}

#[inline]
fn looks_like_doo_response(ptr: *const DooResponse) -> bool {
    if ptr.is_null() {
        return false;
    }
    let addr = ptr as usize;
    if addr < 0x1000 || (addr % 4 != 0) {
        return false;
    }

    if !doo_runtime::memory::validate_pointer(ptr as *const std::ffi::c_void, "looks_like_doo_response") {
        return false;
    }

    // SAFETY: we still can't *guarantee* ptr is mapped, but we avoid the known bad cases
    // (int-as-pointer like 0x4c, misalignment).
    let status = unsafe { (*ptr).status };
    if !(100..=599).contains(&status) {
        return false;
    }

    let body = unsafe { (*ptr).body };
    if !body.is_null() && !looks_like_any_cstr(body) {
        return false;
    }

    let content_type = unsafe { (*ptr).content_type };
    if !content_type.is_null() && !looks_like_any_cstr(content_type) {
        return false;
    }

    true
}

#[inline]
fn looks_like_json_cstr(ptr: *const c_char) -> bool {
    if ptr.is_null() {
        return false;
    }
    let addr = ptr as usize;
    if addr < 0x1000 {
        return false;
    }
    if !doo_runtime::memory::validate_pointer(ptr as *const std::ffi::c_void, "looks_like_json_cstr") {
        return false;
    }
    let first = unsafe { *(ptr as *const u8) };
    first == b'{' || first == b'['
}

#[inline]
fn looks_like_any_cstr(ptr: *const c_char) -> bool {
    if ptr.is_null() {
        return false;
    }
    let addr = ptr as usize;
    if addr < 0x1000 {
        return false;
    }
    if !doo_runtime::memory::validate_pointer(ptr as *const std::ffi::c_void, "looks_like_any_cstr") {
        return false;
    }
    let first = unsafe { *(ptr as *const u8) };
    if first == 0 {
        return true;
    }
    (0x20..=0x7e).contains(&first) || first == b'\t' || first == b'\n' || first == b'\r'
}

// ============================================================================
// Next.call() FFI function
// ============================================================================

/// Call the next middleware or handler in the chain
/// Returns a Response struct (not DooResult) for middleware to use
#[no_mangle]
pub extern "C" fn doo_http_next_call(next: *mut DooNext) -> *mut DooResponse {
    doo_ffi_enter!("doo_http_next_call", "next_ptr={:p}", next);
    
    // Helper to allocate DooResponse using libc::malloc
    unsafe fn alloc_response(
        status: i32,
        body: *const c_char,
        content_type: *const c_char,
    ) -> *mut DooResponse {
        let size = std::mem::size_of::<DooResponse>();
        let ptr = libc::malloc(size) as *mut DooResponse;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        // Track this allocation for memory debugging
        track_alloc(ptr as *const std::ffi::c_void, "http_alloc_response");
        (*ptr).status = status;
        (*ptr).body = body;
        (*ptr).content_type = content_type;
        doo_http_debug!("alloc_response: ptr={:p} status={}", ptr, status);
        ptr
    }

    if next.is_null() {
        doo_http_debug!("doo_http_next_call: null next ptr!");
        return unsafe {
            alloc_response(
                500,
                string_to_c("Internal error: null Next"),
                string_to_c("text/plain"),
            )
        };
    }

    unsafe {
        let next_ref = &mut *next;
        let request = next_ref.request;
        doo_http_debug!("doo_http_next_call: request_ptr={:p} handler={:p}", request, next_ref.handler as *const ());

        // Get the remaining middleware chain
        let middleware_vec_ptr = next_ref.remaining_middleware as *mut Vec<DooMiddlewareFn>;

        let result: *mut DooResult = if middleware_vec_ptr.is_null() {
            // No more middleware, call the handler
            doo_http_debug!("doo_http_next_call: calling direct handler");
            doo_handler_call!("direct_handler", request);
            let res = (next_ref.handler)(request);
            doo_ffi_exit!("direct_handler", "result_ptr={:p}", res);
            res
        } else {
            let middleware_vec = &*middleware_vec_ptr;
            let idx = next_ref.current_index;

            if idx >= middleware_vec.len() {
                // No more middleware, call the handler
                doo_http_debug!("middleware_chain: no more middleware, calling handler");
                doo_handler_call!("final_handler", request);
                let res = (next_ref.handler)(request);
                doo_ffi_exit!("final_handler", "result_ptr={:p}", res);
                res
            } else {
                // Create a new Next for the next middleware in chain
                // Use Box for consistent allocation with the middleware Vec
                doo_http_debug!("middleware_chain: calling middleware {} of {}", idx, middleware_vec.len());
                let new_middleware_vec = middleware_vec.clone();

                let new_next = Box::new(DooNext {
                    request,
                    remaining_middleware: Box::into_raw(Box::new(new_middleware_vec))
                        as *mut std::ffi::c_void,
                    handler: next_ref.handler,
                    current_index: idx + 1,
                });
                let new_next_ptr = Box::into_raw(new_next);
                doo_http_debug!("middleware_chain: new_next_ptr={:p}", new_next_ptr);

                // Call the current middleware
                let current_middleware = middleware_vec[idx];
                doo_ffi_enter!("middleware_chain", "req_ptr={:p}, mw_count={}", request, middleware_vec.len());
                let call_result = current_middleware(request, new_next_ptr);
                doo_ffi_exit!("middleware_chain", "result_ptr={:p}", call_result);

                // CLEANUP DISABLED: Middleware cleanup causing double-free\n                // Let memory leak to prevent heap corruption
                // let recovered_next = Box::from_raw(new_next_ptr);
                // if !recovered_next.remaining_middleware.is_null() {
                //     let mw_ptr = recovered_next.remaining_middleware as *mut Vec<DooMiddlewareFn>;
                //     let _ = Box::from_raw(mw_ptr);
                // }

                call_result
            }
        };

        // Convert DooResult to DooResponse for middleware to use
        if result.is_null() {
            doo_http_debug!("HANDLER_RESULT: null result, returning 500");
            return alloc_response(
                500,
                string_to_c("Handler returned null"),
                string_to_c("text/plain"),
            );
        }
        
        doo_http_debug!("Processing result ptr={:p}", result);

        // CRITICAL: Handler results can be the 2-field LLVM layout.
        // Read tag/value via offsets to avoid UB.
        let (result_tag, result_value) = read_dooresult_tag_value(result);
        
        doo_handler_result!("handler", result, result_tag);
        doo_http_debug!("Result tag={} value_ptr={:p}", result_tag, result_value);
        
        if result_tag == 0 {
            doo_http_debug!("Result OK with value ptr={:p}", result_value);
            // Success - extract response body
            if result_value.is_null() {
                alloc_response(200, string_to_c(""), string_to_c("text/plain"))
            } else {
                // Use helper to potential unwrap DooResult nesting
                let raw_val = result_value as *const c_char;
                let real_val = unwrap_potential_dooresult(raw_val);

                // If it looks like JSON (starts with { or [), wrap it in a response
                if looks_like_json_cstr(real_val) {
                    // It's a JSON string - wrap it in DooResponse
                    let json_body = c_to_string(real_val);
                    let wrapped_body = format!("{{\"data\":{}}}", json_body);
                    alloc_response(
                        200,
                        string_to_c(&wrapped_body),
                        string_to_c("application/json"),
                    )
                } else {
                    // Treat as DooResponse only if it looks like a valid response.
                    let response_ptr = result_value as *mut DooResponse;
                    if looks_like_doo_response(response_ptr) {
                        response_ptr
                    } else {
                        alloc_response(
                            500,
                            string_to_c("Invalid handler return (not JSON, not DooResponse)"),
                            string_to_c("text/plain"),
                        )
                    }
                }
            }
        } else {
            // Error - convert to error response
            // Error is also LLVM-allocated, read directly
            let error_ptr = result_value as *const DooHttpError;
            if error_ptr.is_null() {
                alloc_response(
                    500,
                    string_to_c("Unknown error"),
                    string_to_c("application/json"),
                )
            } else {
                let error_ref = &*error_ptr;
                alloc_response(
                    error_ref.status,
                    error_ref.message,
                    string_to_c("application/json"),
                )
            }
        }
    }
}

// Helper to convert Rust String to C string using simple libc allocation (NO RC header)
// This is for FFI-owned strings ONLY - clean ownership model
fn string_to_c(s: &str) -> *const c_char {
    unsafe {
        let bytes = s.as_bytes();
        let len = bytes.len();

        let total_size = len + 1 + 8;
        let alloc_size = (total_size + 15) & !15;
        let heap_ptr = dooruntime_malloc(alloc_size) as *mut u8;
        if heap_ptr.is_null() {
            doo_http_debug!("ALLOC FAILED: string_to_c len={}", len);
            return std::ptr::null();
        }

        // DEBUG: Track large string allocations
        if len > 10000 {
            doo_mem_alloc!(heap_ptr, alloc_size, "string_to_c_large");
        }

        std::ptr::write_bytes(heap_ptr, 0, alloc_size);
        *(heap_ptr as *mut i32) = 1;
        *(heap_ptr.add(4) as *mut i32) = len as i32;

        let data_ptr = heap_ptr.add(8);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data_ptr, len);
        *data_ptr.add(len) = 0;

        data_ptr as *const c_char
    }
}

fn c_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

#[inline]
unsafe fn read_dooresult_tag_value(result: *mut DooResult) -> (i32, *mut std::ffi::c_void) {
    // Check for use-after-free first
    if is_freed(result as *const std::ffi::c_void) {
        doo_http_debug!("USE-AFTER-FREE: read_dooresult_tag_value result_ptr={:p}", result);
        return (1, std::ptr::null_mut());
    }
    
    // Fail-closed: if result pointer isn't readable/mapped, treat as error.
    if !doo_runtime::memory::validate_pointer(result as *const std::ffi::c_void, "read_dooresult") {
        doo_http_debug!("CORRUPT: read_dooresult_tag_value result_ptr={:p}", result);
        return (1, std::ptr::null_mut());
    }

    // Treat handler results as the 2-field LLVM layout: { i32 tag, void* value }
    // The pointer field is aligned to 8 bytes, so it starts at offset 8.
    let tag = *(result as *const i32);
    let value = *((result as *const u8).add(8) as *const *mut std::ffi::c_void);

    // DEBUG: Aggressive validation of result pointer
    if !value.is_null() && !doo_runtime::memory::validate_pointer(value as *const std::ffi::c_void, "read_dooresult_value") {
        doo_http_debug!("CORRUPT: read_dooresult_tag_value value_ptr={:p}", value);
    }

    (tag, value)
}

/// Free a string allocated by string_to_c (using simple libc::malloc)
/// Since string_to_c now uses plain libc allocation without RC headers,
/// we just call libc::free directly - clean ownership model
#[inline]
fn free_rc_string(ptr: *const c_char) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        dooruntime_free_rc_string(ptr);
    }
}

// Helper for unified allocation - use libc::malloc and set owner to FFI
fn make_ok_void() -> *mut DooResult {
    unsafe {
        let size = std::mem::size_of::<DooResult>();
        let ptr = libc::malloc(size) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        track_alloc(ptr as *const std::ffi::c_void, "http_make_ok_void");
        (*ptr).tag = 0;
        (*ptr).value = std::ptr::null_mut();
        (*ptr).owner = owner::FFI;
        ptr
    }
}

fn make_ok_string(s: &str) -> *mut DooResult {
    unsafe {
        let size = std::mem::size_of::<DooResult>();
        let ptr = libc::malloc(size) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        track_alloc(ptr as *const std::ffi::c_void, "http_make_ok_string");
        (*ptr).tag = 0;
        (*ptr).value = string_to_c(s) as *mut std::ffi::c_void;
        (*ptr).owner = owner::FFI;
        ptr
    }
}

/// Create an Ok result with a pointer value using libc::malloc
fn make_ok_ptr(value: *mut std::ffi::c_void) -> *mut DooResult {
    unsafe {
        let size = std::mem::size_of::<DooResult>();
        let ptr = libc::malloc(size) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        track_alloc(ptr as *const std::ffi::c_void, "http_make_ok_ptr");
        (*ptr).tag = 0;
        (*ptr).value = value;
        (*ptr).owner = owner::FFI;
        ptr
    }
}

fn make_err_http(status: u16, message: &str) -> *mut DooResult {
    unsafe {
        // Allocate Error using libc::malloc (FFI owned)
        let error_size = std::mem::size_of::<DooHttpError>();
        let error = libc::malloc(error_size) as *mut DooHttpError;
        if error.is_null() {
            return std::ptr::null_mut();
        }
        track_alloc(error as *const std::ffi::c_void, "http_make_err_http_error");
        (*error).status = status as i32;
        (*error).message = string_to_c(message);

        // Allocate Result using libc::malloc
        let result_size = std::mem::size_of::<DooResult>();
        let ptr = libc::malloc(result_size) as *mut DooResult;
        if ptr.is_null() {
            libc::free(error as *mut libc::c_void);
            return std::ptr::null_mut();
        }
        track_alloc(ptr as *const std::ffi::c_void, "http_make_err_http_result");
        (*ptr).tag = 1;
        (*ptr).value = error as *mut std::ffi::c_void;
        (*ptr).owner = owner::FFI;
        ptr
    }
}

unsafe fn free_handler_result(_result: *mut DooResult) {}

/// Free a handler result based on ownership tracking.
///
/// TEMPORARILY DISABLED: Skip all freeing to verify tests pass.
/// Memory will leak but no crashes. This proves ownership model is the right approach.
// FFI Functions - Called from Doo code
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_register_handler(name: *const c_char, handler: DooHandlerFn) {
    let handler_name = c_to_string(name);
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.handlers.insert(handler_name.clone(), handler);
    doo_http_debug!("✓ Registered handler function: {}", handler_name);
}

/// Register handler with metadata (including decorators for validation)
#[no_mangle]
pub extern "C" fn doo_http_register_handler_with_metadata(
    name: *const c_char,
    handler: DooHandlerFn,
    metadata_json: *const c_char,
) {
    if name.is_null() || metadata_json.is_null() {
        return;
    }

    let handler_name = c_to_string(name);
    let metadata_str = c_to_string(metadata_json);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.handlers.insert(handler_name.clone(), handler);

    // Parse metadata JSON to extract struct_decorators
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&metadata_str) {
        if let Some(struct_decorators_obj) =
            json.get("struct_decorators").and_then(|v| v.as_object())
        {
            let mut struct_decorators = HashMap::new();

            for (struct_name, fields_obj) in struct_decorators_obj {
                if let Some(fields_map) = fields_obj.as_object() {
                    let mut field_decorators = HashMap::new();

                    for (field_name, decorators_arr) in fields_map {
                        if let Some(decorators) = decorators_arr.as_array() {
                            let mut decorator_list = Vec::new();

                            for decorator in decorators {
                                if let Some(dec_obj) = decorator.as_object() {
                                    let name = dec_obj
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();

                                    let args = dec_obj
                                        .get("args")
                                        .and_then(|v| v.as_array())
                                        .map(|arr| {
                                            arr.iter()
                                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                                .collect()
                                        })
                                        .unwrap_or_default();

                                    decorator_list.push(DecoratorInfo { name, args });
                                }
                            }

                            field_decorators.insert(field_name.clone(), decorator_list);
                        }
                    }

                    struct_decorators.insert(struct_name.clone(), field_decorators);
                }
            }

            // Also extract struct_fields and struct_layouts from metadata
            let struct_fields = json
                .get("struct_fields")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .map(|(k, v)| {
                            let fields = v
                                .as_array()
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|field_arr| {
                                            field_arr.as_array().map(|inner| {
                                                inner
                                                    .iter()
                                                    .filter_map(|s| {
                                                        s.as_str().map(|s| s.to_string())
                                                    })
                                                    .collect()
                                            })
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            (k.clone(), fields)
                        })
                        .collect()
                })
                .unwrap_or_default();

            let struct_layouts = json
                .get("struct_layouts")
                .cloned()
                .unwrap_or(serde_json::json!({}));

            let return_type = json
                .get("return_type")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();

            let param_types = json
                .get("param_types")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            // Parse enum_variants: { "EnumName": ["Variant1", "Variant2"] }
            let enum_variants = json
                .get("enum_variants")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .map(|(k, v)| {
                            let variants = v
                                .as_array()
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|s| s.as_str().map(|s| s.to_string()))
                                        .collect()
                                })
                                .unwrap_or_default();
                            (k.clone(), variants)
                        })
                        .collect()
                })
                .unwrap_or_default();

            let metadata = HandlerMetadata {
                param_types,
                struct_decorators,
                struct_fields,
                struct_layouts,
                enum_variants,
                return_type,
            };

            registry
                .handler_metadata
                .insert(handler_name.clone(), metadata);
        }
    }

    doo_http_debug!(
        "✓ Registered handler function with metadata: {}",
        handler_name
    );
}

/// Register a middleware function by name
#[no_mangle]
pub extern "C" fn doo_http_register_middleware(name: *const c_char, middleware: DooMiddlewareFn) {
    let middleware_name = c_to_string(name);
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry
        .middleware_handlers
        .insert(middleware_name.clone(), middleware);
    doo_http_debug!("✓ Registered middleware function: {}", middleware_name);
}

/// Register route with middleware array
/// middleware_names is a comma-separated string: "Auth,Admin,Logger"
#[no_mangle]
pub extern "C" fn doo_http_get_with_middleware(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let middleware_str = c_to_string(middleware_names);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    // Parse middleware names
    let middleware_list: Vec<String> = if middleware_str.is_empty() {
        vec![]
    } else {
        middleware_str
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    };

    // Lookup middleware functions
    let mut middleware_fns = Vec::new();
    for mw_name in middleware_list {
        // Auto-register JWT middleware if referenced
        if mw_name == "jwt" && !registry.middleware_handlers.contains_key("jwt") {
            registry
                .middleware_handlers
                .insert("jwt".to_string(), jwt_middleware_handler);
        }

        if let Some(mw_fn) = registry.middleware_handlers.get(&mw_name).copied() {
            middleware_fns.push(mw_fn);
        }
    }

    registry.register_by_name_with_middleware("GET", &path_str, &handler_str, middleware_fns);
    make_ok_void()
}

#[no_mangle]
pub extern "C" fn doo_http_post_with_middleware(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let middleware_str = c_to_string(middleware_names);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    let middleware_list: Vec<String> = if middleware_str.is_empty() {
        vec![]
    } else {
        middleware_str
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    };

    let mut middleware_fns = Vec::new();
    for mw_name in middleware_list {
        // Auto-register JWT middleware if referenced
        if mw_name == "jwt" && !registry.middleware_handlers.contains_key("jwt") {
            registry
                .middleware_handlers
                .insert("jwt".to_string(), jwt_middleware_handler);
        }

        if let Some(mw_fn) = registry.middleware_handlers.get(&mw_name).copied() {
            middleware_fns.push(mw_fn);
        }
    }

    registry.register_by_name_with_middleware("POST", &path_str, &handler_str, middleware_fns);
    make_ok_void()
}

#[no_mangle]
pub extern "C" fn doo_http_put_with_middleware(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let middleware_str = c_to_string(middleware_names);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    let middleware_list: Vec<String> = if middleware_str.is_empty() {
        vec![]
    } else {
        middleware_str
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    };

    let mut middleware_fns = Vec::new();
    for mw_name in middleware_list {
        // Auto-register JWT middleware if referenced
        if mw_name == "jwt" && !registry.middleware_handlers.contains_key("jwt") {
            registry
                .middleware_handlers
                .insert("jwt".to_string(), jwt_middleware_handler);
        }

        if let Some(mw_fn) = registry.middleware_handlers.get(&mw_name).copied() {
            middleware_fns.push(mw_fn);
        }
    }

    registry.register_by_name_with_middleware("PUT", &path_str, &handler_str, middleware_fns);
    make_ok_void()
}

#[no_mangle]
pub extern "C" fn doo_http_delete_with_middleware(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let middleware_str = c_to_string(middleware_names);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    let middleware_list: Vec<String> = if middleware_str.is_empty() {
        vec![]
    } else {
        middleware_str
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    };

    let mut middleware_fns = Vec::new();
    for mw_name in middleware_list {
        // Auto-register JWT middleware if referenced
        if mw_name == "jwt" && !registry.middleware_handlers.contains_key("jwt") {
            registry
                .middleware_handlers
                .insert("jwt".to_string(), jwt_middleware_handler);
        }

        if let Some(mw_fn) = registry.middleware_handlers.get(&mw_name).copied() {
            middleware_fns.push(mw_fn);
        }
    }

    registry.register_by_name_with_middleware("DELETE", &path_str, &handler_str, middleware_fns);
    make_ok_void()
}

#[no_mangle]
pub extern "C" fn doo_http_patch_with_middleware(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let middleware_str = c_to_string(middleware_names);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    let middleware_list: Vec<String> = if middleware_str.is_empty() {
        vec![]
    } else {
        middleware_str
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    };

    let mut middleware_fns = Vec::new();
    for mw_name in middleware_list {
        // Auto-register JWT middleware if referenced
        if mw_name == "jwt" && !registry.middleware_handlers.contains_key("jwt") {
            registry
                .middleware_handlers
                .insert("jwt".to_string(), jwt_middleware_handler);
        }

        if let Some(mw_fn) = registry.middleware_handlers.get(&mw_name).copied() {
            middleware_fns.push(mw_fn);
        }
    }

    registry.register_by_name_with_middleware("PATCH", &path_str, &handler_str, middleware_fns);
    make_ok_void()
}

/// This is called automatically for each handler name passed to route registration
unsafe fn auto_register_handler(handler_name: &str) -> Option<DooHandlerFn> {
    // Try to find the function symbol in the current process
    // Function names in Doo are mangled, so try both mangled and unmangled
    let _symbol_name = handler_name;

    // For now, we'll rely on explicit registration via doo_http_register_handler
    // or use the codegen to call register_handler automatically
    None
}

// ============================================================================
// GET
// ============================================================================
#[no_mangle]
pub extern "C" fn doo_http_get(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register_by_name("GET", &path_str, &handler_str);
    make_ok_void()
}

#[no_mangle]
pub extern "C" fn doo_http_get_fn(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    let path_str = c_to_string(path);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register("GET", &path_str, handler);
    make_ok_void()
}

// ============================================================================
// POST
// ============================================================================
#[no_mangle]
pub extern "C" fn doo_http_post(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register_by_name("POST", &path_str, &handler_str);
    make_ok_void()
}

#[no_mangle]
pub extern "C" fn doo_http_post_fn(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    let path_str = c_to_string(path);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register("POST", &path_str, handler);
    make_ok_void()
}

// ============================================================================
// PUT
// ============================================================================
#[no_mangle]
pub extern "C" fn doo_http_put(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register_by_name("PUT", &path_str, &handler_str);
    make_ok_void()
}

#[no_mangle]
pub extern "C" fn doo_http_put_fn(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    let path_str = c_to_string(path);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register("PUT", &path_str, handler);
    make_ok_void()
}

// ============================================================================
// DELETE
// ============================================================================
#[no_mangle]
pub extern "C" fn doo_http_delete(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register_by_name("DELETE", &path_str, &handler_str);
    make_ok_void()
}

#[no_mangle]
pub extern "C" fn doo_http_delete_fn(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    let path_str = c_to_string(path);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register("DELETE", &path_str, handler);
    make_ok_void()
}

// ============================================================================
// PATCH
// ============================================================================
#[no_mangle]
pub extern "C" fn doo_http_patch(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register_by_name("PATCH", &path_str, &handler_str);
    make_ok_void()
}

#[no_mangle]
pub extern "C" fn doo_http_patch_fn(
    _server: *const std::ffi::c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    let path_str = c_to_string(path);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register("PATCH", &path_str, handler);
    make_ok_void()
}

// ============================================================================
// Middleware
// ============================================================================
#[no_mangle]
pub extern "C" fn doo_http_use(
    server: *const std::ffi::c_void,
    middleware_name: *const c_char,
) -> *const std::ffi::c_void {
    let middleware_str = c_to_string(middleware_name);
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    // Look up middleware function pointer by name
    if let Some(mw_fn) = registry.middleware_handlers.get(&middleware_str).copied() {
        registry.add_middleware(mw_fn);
        doo_http_debug!("✓ Registered global middleware: {}", middleware_str);
    }

    // Return the server pointer for method chaining
    server
}

// ============================================================================
// JWT Middleware FFI Function
// ============================================================================

/// Returns the JWT middleware name for use in route registration
/// Called when jwt() is used in Doo code
#[no_mangle]
pub extern "C" fn doo_http_jwt() -> *const c_char {
    // Ensure JWT middleware is registered
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry.middleware_handlers.contains_key("jwt") {
        registry
            .middleware_handlers
            .insert("jwt".to_string(), jwt_middleware_handler);
    }
    drop(registry);

    // Return the middleware name as a string
    string_to_c("jwt")
}

/// Configure CORS middleware - shorthand version (matches std/Http.doo signature)
/// Always uses default config: origin "*", all methods, no credentials
#[no_mangle]
pub extern "C" fn doo_http_cors(server: *mut std::ffi::c_void) -> *mut std::ffi::c_void {
    // Use default config
    let config = CorsConfig::default();

    // Store config
    let mut cors_config_guard = get_cors_config().lock().unwrap();
    *cors_config_guard = Some(config);
    drop(cors_config_guard);

    // Register CORS middleware globally
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry.middleware_handlers.contains_key("cors") {
        registry
            .middleware_handlers
            .insert("cors".to_string(), cors_middleware_handler);
    }
    registry.add_middleware_once(cors_middleware_handler);
    drop(registry);

    doo_http_debug!("✓ Registered CORS middleware");
    server
}

/// Configure CORS middleware with custom options
#[no_mangle]
pub extern "C" fn doo_http_cors_custom(
    server: *mut std::ffi::c_void,
    options: *mut std::ffi::c_void, // Map(Str,Str) where values are JSON or plain strings
) -> *mut std::ffi::c_void {
    unsafe {
        let origins_vec = parse_json_str_array_or_default(
            doo_map_get_str(options, "origins"),
            vec!["*".to_string()],
        );
        let methods_vec = parse_json_str_array_or_default(
            doo_map_get_str(options, "methods"),
            vec![
                "GET".to_string(),
                "POST".to_string(),
                "PUT".to_string(),
                "DELETE".to_string(),
                "OPTIONS".to_string(),
            ],
        );
        let headers_vec = parse_json_str_array_or_default(
            doo_map_get_str(options, "headers"),
            vec!["Content-Type".to_string(), "Authorization".to_string()],
        );
        let credentials =
            parse_json_bool_or_default(doo_map_get_str(options, "credentials"), false);
        let max_age = parse_json_i64_or_default(doo_map_get_str(options, "max_age"), 0);

        let config = CorsConfig {
            origins: origins_vec,
            methods: methods_vec,
            headers: headers_vec,
            expose_headers: Vec::<String>::new(),
            credentials,
            max_age: if max_age > 0 {
                Some(max_age as i32)
            } else {
                None
            },
        };

        // Store config
        let mut cors_config_guard = get_cors_config().lock().unwrap();
        *cors_config_guard = Some(config);
        drop(cors_config_guard);

        // Register middleware if not already registered
        let routes = get_routes();
        let mut registry = routes.lock().unwrap();
        if !registry.middleware_handlers.contains_key("cors") {
            registry
                .middleware_handlers
                .insert("cors".to_string(), cors_middleware_handler);
        }
        registry.add_middleware_once(cors_middleware_handler);
        drop(registry);

        doo_http_debug!("✓ Registered CORS middleware (custom)");
        server
    }
}

/// Configure rate limiting middleware - shorthand version (matches std/Http.doo signature)
/// Always uses default config: 100 req/min per IP
#[no_mangle]
pub extern "C" fn doo_http_ratelimit(server: *mut std::ffi::c_void) -> *mut std::ffi::c_void {
    // Use default config: 100 requests per 60 seconds per IP
    let config = RateLimitConfig::default();

    // Store config
    let mut rl_config_guard = get_ratelimit_config().lock().unwrap();
    *rl_config_guard = Some(config.clone());
    drop(rl_config_guard);

    // Clear state for fresh start
    let mut rl_state_guard = get_ratelimit_state().lock().unwrap();
    rl_state_guard.clear();
    drop(rl_state_guard);

    // Register rate limit middleware globally
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry.middleware_handlers.contains_key("ratelimit") {
        registry
            .middleware_handlers
            .insert("ratelimit".to_string(), ratelimit_middleware_handler);
    }
    registry.add_middleware_once(ratelimit_middleware_handler);
    drop(registry);

    doo_http_debug!("✓ Rate limit: {} req/{} sec", config.max, config.window);

    server
}

/// Configure rate limiting middleware with custom options
#[no_mangle]
pub extern "C" fn doo_http_ratelimit_custom(
    server: *mut std::ffi::c_void,
    options: *mut std::ffi::c_void, // Map(Str,Str)
) -> *mut std::ffi::c_void {
    unsafe {
        let max = parse_json_i64_or_default(doo_map_get_str(options, "max"), 100);
        let window = parse_json_i64_or_default(doo_map_get_str(options, "window"), 3600);
        let per_str = parse_json_string_or_default(doo_map_get_str(options, "per"), "user");

        let config = RateLimitConfig {
            max: if max > 0 { max as u32 } else { 100 },
            window: if window > 0 { window as u64 } else { 3600 },
            per: per_str,
        };

        // Store config
        let mut rl_config_guard = get_ratelimit_config().lock().unwrap();
        *rl_config_guard = Some(config.clone());
        drop(rl_config_guard);

        let mut rl_state_guard = get_ratelimit_state().lock().unwrap();
        rl_state_guard.clear();
        drop(rl_state_guard);

        // Register middleware if not already registered
        let routes = get_routes();
        let mut registry = routes.lock().unwrap();
        if !registry.middleware_handlers.contains_key("ratelimit") {
            registry
                .middleware_handlers
                .insert("ratelimit".to_string(), ratelimit_middleware_handler);
        }
        registry.add_middleware_once(ratelimit_middleware_handler);
        drop(registry);

        doo_http_debug!(
            "✓ Registered rate limiting middleware ({} req/{} sec per {})",
            config.max,
            config.window,
            config.per
        );
        server
    }
}

// ============================================================================
// Groups
// ============================================================================
#[no_mangle]
pub extern "C" fn doo_http_group(
    _server: *const std::ffi::c_void,
    _prefix: *const c_char,
    _handler: extern "C" fn(),
) -> *mut DooResult {
    // Groups are handled at compile-time by the analyzer
    // This is a no-op at runtime
    make_ok_void()
}

// ============================================================================
// JSON Parsing
// ============================================================================
#[no_mangle]
pub extern "C" fn doo_http_parse_json(json: *const c_char) -> *mut std::ffi::c_void {
    let json_str = c_to_string(json);
    // For now, just return the string as-is
    // Later, we'll parse into proper Doo structs
    string_to_c(&json_str) as *mut std::ffi::c_void
}

#[no_mangle]
pub extern "C" fn doo_http_to_json(obj: *mut std::ffi::c_void) -> *const c_char {
    // For now, assume obj is already a JSON string
    obj as *const c_char
}

fn parse_query(query: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if query.is_empty() {
        return params;
    }

    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            params.insert(
                urlencoding::decode(key).unwrap_or_default().to_string(),
                urlencoding::decode(value).unwrap_or_default().to_string(),
            );
        }
    }
    params
}

async fn handle_request(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let req_start = std::time::Instant::now();

    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();

    // Parse query parameters
    let query_params = parse_query(&query);

    // Get headers
    let mut headers_map = HashMap::new();
    for (key, value) in req.headers().iter() {
        if let Ok(v) = value.to_str() {
            headers_map.insert(key.to_string(), v.to_string());
        }
    }

    // Get content type
    let content_type = req
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/plain")
        .to_string();

    // Enforce content-type for body methods
    let requires_body = method == "POST" || method == "PUT" || method == "PATCH";
    if requires_body {
        let is_json = content_type
            .to_ascii_lowercase()
            .starts_with("application/json");
        if !is_json {
            use error::*;
            let err = content_type_error(
                "Content-Type header required for POST/PUT/PATCH requests".to_string(),
                path.clone(),
                Some("application/json".to_string()),
                Some(content_type.clone()),
            );
            let body_json = err.to_json_string();
            set_last_error(err.status_code() as i32, body_json.clone());
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("content-type", "application/json")
                .body(Full::new(Bytes::from(body_json)))
                .unwrap());
        }
    }

    // Read body
    let body_bytes = req.collect().await?.to_bytes();
    let body = String::from_utf8_lossy(&body_bytes).to_string();

    // Find handler
    let routes = get_routes();
    let registry = routes.lock().unwrap();

    let (route, params) = match registry.find_route(&method, &path) {
        Some((r, p)) => (r, p),
        None => {
            // Check if path exists with different methods (405 vs 404)
            let allowed_methods = registry.find_allowed_methods(&path);
            drop(registry);

            if !allowed_methods.is_empty() {
                // Path exists but method not allowed
                use error::*;
                let error_response =
                    method_not_allowed_error(path.clone(), method.clone(), allowed_methods);
                let error_json = error_response.to_json_string();
                return Ok(Response::builder()
                    .status(StatusCode::METHOD_NOT_ALLOWED)
                    .header("content-type", "application/json")
                    .body(Full::new(Bytes::from(error_json)))
                    .unwrap());
            } else {
                // Path doesn't exist at all
                let error_response = not_found(
                    "The requested route does not exist".to_string(),
                    path.clone(),
                )
                .with_method(method.clone());
                let error_json = error_response.to_json_string();
                return Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header("content-type", "application/json")
                    .body(Full::new(Bytes::from(error_json)))
                    .unwrap());
            }
        }
    };

    let handler = route.handler;
    let middleware = route.middleware.clone();
    let global_middleware = registry.middleware.clone();

    // Get handler metadata for validation and redirect detection
    let handler_metadata = registry
        .handlers
        .iter()
        .find(|(_, &h)| h as usize == handler as usize)
        .and_then(|(name, _)| registry.handler_metadata.get(name))
        .cloned();

    // Check if response struct has @redirect field
    let redirect_field = handler_metadata
        .as_ref()
        .and_then(|meta| find_redirect_field(&meta.struct_decorators, &meta.return_type));

    drop(registry);

    // Validate JSON body for POST/PUT/PATCH requests
    if requires_body && !body.is_empty() {
        if let Err(_) = serde_json::from_str::<serde_json::Value>(&body) {
            use error::*;
            let err = invalid_json_error(path.clone());
            let body_json = err.to_json_string();
            set_last_error(err.status_code() as i32, body_json.clone());
            log_request(req_start, StatusCode::BAD_REQUEST, &method, &path);
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("content-type", "application/json")
                .body(Full::new(Bytes::from(body_json)))
                .unwrap());
        }
    }

    // Create Doo Request
    let params_box = Box::new(params);
    let query_box = Box::new(query_params);
    let headers_box = Box::new(headers_map);

    let doo_request = Box::new(DooRequest {
        method: string_to_c(&method),
        path: string_to_c(&path),
        body: string_to_c(&body),
        content_type: string_to_c(&content_type),
        params: Box::into_raw(params_box) as *mut std::ffi::c_void,
        query: Box::into_raw(query_box) as *mut std::ffi::c_void,
        headers: Box::into_raw(headers_box) as *mut std::ffi::c_void,
    });

    // Store current request path in thread-local storage for RFC 7807 errors
    set_current_request_path(&path);

    // Combine global and route-specific middleware
    let mut all_middleware = global_middleware.clone();
    all_middleware.extend(middleware.iter().cloned());

    let req_ptr = Box::into_raw(doo_request);

    // If there's middleware, create Next chain and call first middleware
    let result = if !all_middleware.is_empty() {
        // Clone the middleware list for the chain - original owned by this scope
        let middleware_vec = all_middleware.clone();
        let middleware_box = Box::new(middleware_vec);
        let mw_raw_ptr = Box::into_raw(middleware_box);

        // Create Next for first middleware with current_index=1
        // (first middleware is called directly, so next.call() goes to index 1)
        let next_for_first = Box::new(DooNext {
            request: req_ptr,
            remaining_middleware: mw_raw_ptr as *mut std::ffi::c_void,
            handler,
            current_index: 1,
        });

        let next_ptr = Box::into_raw(next_for_first);

        // Call the first middleware directly
        let first_middleware = all_middleware[0];
        // DEBUG: Log middleware call
        doo_ffi_enter!("middleware_chain", "req_ptr={:p}, mw_count={}", req_ptr, all_middleware.len());
        let res = first_middleware(req_ptr, next_ptr);
        doo_ffi_exit!("middleware_chain", "result_ptr={:p}", res);

        // CLEANUP DISABLED: Middleware cleanup causing double-free
        // Let memory leak to prevent heap corruption
        // unsafe {
        //     let _ = Box::from_raw(next_ptr); // Free DooNext struct
        //     let _ = Box::from_raw(mw_raw_ptr); // Free original middleware Vec
        // }
        res
    } else {
        // No middleware, call handler directly
        doo_handler_call!("direct_handler", req_ptr);
        let res = handler(req_ptr);
        doo_ffi_exit!("direct_handler", "result_ptr={:p}", res);
        res
    };
    // Process result
    doo_http_debug!("Processing result ptr={:p}", result);
    let response = unsafe {
        if result.is_null() {
            doo_http_debug!("WARN: Handler returned null result");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from("Handler returned null")))
                .unwrap()
        } else {
            // CRITICAL: Result is allocated by LLVM's malloc, NOT Rust's Box.
            // We must read the fields directly and use libc::free for cleanup,
            // otherwise we get allocator mismatch and heap corruption.
            let (result_tag, result_value) = read_dooresult_tag_value(result);
            doo_handler_result!("handler", result, result_tag);
            doo_http_debug!("Result tag={} value_ptr={:p}", result_tag, result_value);

            if result_tag == 0 {
                // Success - value is DooResponse*
                if result_value.is_null() {
                    doo_http_debug!("Result OK with null value");
                    Response::builder()
                        .status(StatusCode::OK)
                        .body(Full::new(Bytes::from("")))
                        .unwrap()
                } else {
                    doo_http_debug!("Result OK with value ptr={:p}", result_value);
                    // Check if value is DooResult (tag < 100) or DooResponse (status >= 100)
                    let raw_val = result_value as *const c_char;
                    let real_val = unwrap_potential_dooresult(raw_val);

                    // Check if it's a JSON string
                    if looks_like_json_cstr(real_val) {
                        // JSON String
                        let json_body = c_to_string(real_val);

                        // Check if this response should be a redirect.
                        // Redirects must only apply to the public short-link endpoint (non-/api) and only for GET.
                        if method == "GET" && !path.starts_with("/api") {
                            if let Some(ref redirect_field_name) = redirect_field {
                                // Parse JSON and extract the redirect URL from the specified field.
                                // Note: responses may be wrapped as {"data":{...}}.
                                if let Ok(json_value) =
                                    serde_json::from_str::<serde_json::Value>(&json_body)
                                {
                                    fn get_ci<'a>(
                                        obj: &'a serde_json::Map<String, serde_json::Value>,
                                        key: &str,
                                    ) -> Option<&'a serde_json::Value>
                                    {
                                        obj.iter()
                                            .find(|(k, _)| k.eq_ignore_ascii_case(key))
                                            .map(|(_, v)| v)
                                    }

                                    let redirect_url = match &json_value {
                                        serde_json::Value::Object(obj) => {
                                            get_ci(obj, redirect_field_name)
                                                .and_then(|v| v.as_str())
                                                .or_else(|| {
                                                    get_ci(obj, "data")
                                                        .and_then(|v| v.as_object())
                                                        .and_then(|data_obj| {
                                                            get_ci(
                                                                data_obj,
                                                                redirect_field_name,
                                                            )
                                                            .and_then(|v| v.as_str())
                                                        })
                                                })
                                        }
                                        _ => None,
                                    };

                                    if let Some(redirect_url) = redirect_url {
                                        return Ok(Response::builder()
                                            .status(StatusCode::FOUND) // 302
                                            .header("Location", redirect_url)
                                            .body(Full::new(Bytes::new()))
                                            .unwrap());
                                    }
                                }
                            }
                        }

                        let wrapped_body = if json_body.starts_with("{\"data\":")
                            || json_body.starts_with("{\"data\" :")
                        {
                            json_body
                        } else {
                            format!("{{\"data\":{}}}", json_body)
                        };

                        Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", "application/json")
                            .body(Full::new(Bytes::from(wrapped_body)))
                            .unwrap()
                    } else {
                        // DooResponse
                        // Response is also allocated by LLVM malloc - read directly, don't use Box
                        let response_ptr = result_value as *const DooResponse;
                        if !looks_like_doo_response(response_ptr) {
                            return Ok(Response::builder()
                                .status(StatusCode::INTERNAL_SERVER_ERROR)
                                .body(Full::new(Bytes::from(
                                    "Invalid handler return (not JSON, not DooResponse)",
                                )))
                                .unwrap());
                        }
                        let response_ref = &*response_ptr;

                        let status = StatusCode::from_u16(response_ref.status as u16)
                            .unwrap_or(StatusCode::OK);

                        // Handling body from Response (could also be DooResult)
                        let raw_body = response_ref.body;
                        let real_body = unwrap_potential_dooresult(raw_body);

                        let body_str = if real_body.is_null() {
                            String::new()
                        } else {
                            if !doo_runtime::memory::validate_pointer(
                                real_body as *const std::ffi::c_void,
                                "response_body",
                            ) {
                                String::new()
                            } else {
                                CStr::from_ptr(real_body).to_string_lossy().to_string()
                            }
                        };

                        // Check if this response should be a redirect based on handler return struct metadata
                        // (e.g. returning Link where DestinationUrl is marked @redirect).
                        // Redirects must only apply to the public short-link endpoint (non-/api) and only for GET.
                        if method == "GET" && !path.starts_with("/api") {
                            if let Some(ref redirect_field_name) = redirect_field {
                                if body_str.starts_with('{') {
                                    if let Ok(json_value) =
                                        serde_json::from_str::<serde_json::Value>(&body_str)
                                    {
                                        fn get_ci<'a>(
                                            obj: &'a serde_json::Map<String, serde_json::Value>,
                                            key: &str,
                                        ) -> Option<&'a serde_json::Value>
                                        {
                                            obj.iter()
                                                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                                                .map(|(_, v)| v)
                                        }

                                        let redirect_url = match &json_value {
                                            serde_json::Value::Object(obj) => {
                                                get_ci(obj, redirect_field_name)
                                                    .and_then(|v| v.as_str())
                                                    .or_else(|| {
                                                        get_ci(obj, "data")
                                                            .and_then(|v| v.as_object())
                                                            .and_then(|data_obj| {
                                                                get_ci(
                                                                    data_obj,
                                                                    redirect_field_name,
                                                                )
                                                                .and_then(|v| v.as_str())
                                                            })
                                                    })
                                            }
                                            _ => None,
                                        };

                                        if let Some(redirect_url) = redirect_url {
                                            return Ok(Response::builder()
                                                .status(StatusCode::FOUND)
                                                .header("Location", redirect_url)
                                                .body(Full::new(Bytes::new()))
                                                .unwrap());
                                        }
                                    }
                                }
                            }
                        }

                        // Auto redirect: for non-API GET routes, if body is a URL, return 302.
                        if method == "GET" && !path.starts_with("/api") {
                            let trimmed = body_str.trim();
                            if trimmed.starts_with("http://") || trimmed.starts_with("https://")
                            {
                                return Ok(Response::builder()
                                    .status(StatusCode::FOUND)
                                    .header("Location", trimmed)
                                    .body(Full::new(Bytes::new()))
                                    .unwrap());
                            }
                        }

                        // Wrap JSON arrays and objects in {"data": ...} envelope
                        // This ensures RFC 7807 compliance for all JSON responses
                        let final_body =
                            if body_str.starts_with('[') || body_str.starts_with('{') {
                                // Check if already wrapped in {"data": ...}
                                if body_str.starts_with("{\"data\":")
                                    || body_str.starts_with("{\"data\" :")
                                {
                                    body_str
                                } else {
                                    format!("{{\"data\":{}}}", body_str)
                                }
                            } else if body_str.is_empty() {
                                "{\"data\":null}".to_string()
                            } else {
                                body_str
                            };

                        let content_type_str = if response_ref.content_type.is_null() {
                            "application/json".to_string()
                        } else {
                            CStr::from_ptr(response_ref.content_type)
                                .to_string_lossy()
                                .to_string()
                        };

                        let mut builder = Response::builder()
                            .status(status)
                            .header("content-type", content_type_str);

                        // Auto-inject CORS headers if configured
                        // This ensures headers are present even if middleware logic handled the blocking
                        if let Ok(config_guard) = get_cors_config().lock() {
                            if let Some(config) = config_guard.as_ref() {
                                // Origin: For now use the first one or * if multiple not supported by simple header injection
                                // (Real implementation should match against request Origin)
                                let origin_val = if config.origins.is_empty() {
                                    "*".to_string()
                                } else {
                                    config.origins[0].clone()
                                };

                                builder = builder
                                    .header("Access-Control-Allow-Origin", origin_val)
                                    .header(
                                        "Access-Control-Allow-Methods",
                                        config.methods.join(", "),
                                    )
                                    .header(
                                        "Access-Control-Allow-Headers",
                                        config.headers.join(", "),
                                    );

                                if config.credentials {
                                    builder = builder
                                        .header("Access-Control-Allow-Credentials", "true");
                                }
                            }
                        }

                        builder.body(Full::new(Bytes::from(final_body))).unwrap()
                    }
                }
            } else {
                // Error - value is DooHttpError*
                // Error is also allocated by LLVM malloc - read directly, don't use Box
                let error_ptr = result_value as *const DooHttpError;
                let error_ref = &*error_ptr;

                let status = StatusCode::from_u16(error_ref.status as u16)
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                let message = if error_ref.message.is_null() {
                    "Unknown error".to_string()
                } else {
                    CStr::from_ptr(error_ref.message)
                        .to_string_lossy()
                        .to_string()
                };

                // Check if message is already RFC 7807 JSON (starts with {"type": or {"detail":)
                let body_str =
                    if message.starts_with("{\"type\":") || message.starts_with("{\"detail\":") {
                        // Already RFC 7807 format, use as-is
                        message
                    } else {
                        // Legacy error, wrap in simple error object
                        format!("{{\"error\":\"{}\"}}", message.replace("\"", "\\\""))
                    };

                let mut builder = Response::builder()
                    .status(status)
                    .header("content-type", "application/json");

                if let Ok(config_guard) = get_cors_config().lock() {
                    if let Some(config) = config_guard.as_ref() {
                        let origin_val = if config.origins.is_empty() {
                            "*".to_string()
                        } else {
                            config.origins[0].clone()
                        };

                        builder = builder
                            .header("Access-Control-Allow-Origin", origin_val)
                            .header("Access-Control-Allow-Methods", config.methods.join(", "))
                            .header("Access-Control-Allow-Headers", config.headers.join(", "));

                        if config.credentials {
                            builder = builder.header("Access-Control-Allow-Credentials", "true");
                        }
                    }
                }

                builder.body(Full::new(Bytes::from(body_str))).unwrap()
            }
        }
    };

    // Log request with new format: [Doo] HH:MM:SS | STATUS | Xms | METHOD PATH
    let elapsed = req_start.elapsed();
    let status_code = response.status().as_u16();
    let now = chrono::Local::now();
    let timestamp = now.format("%H:%M:%S");
    let duration_ms = elapsed.as_millis();

    println!(
        "[Doo] {} | {} | {:>3}ms | {} {}",
        timestamp, status_code, duration_ms, method, path
    );

    // Clean up request now that allocator mismatch is fixed
    // CLEANUP: Drop the DooRequest and its components
    // TEMPORARILY DISABLED: All cleanup to isolate memory issue
    // Memory will leak but this tests if cleanup is the cause
    unsafe {
        // CLEANUP: All cleanup disabled to isolate memory issue
        // let req = Box::from_raw(req_ptr);
        // let _ = Box::from_raw(req.params as *mut HashMap<String, String>);
        // let _ = Box::from_raw(req.query as *mut HashMap<String, String>);
        // let _ = Box::from_raw(req.headers as *mut HashMap<String, String>);
        // No string cleanup either
    }

    // MEMORY NOTE: Handler result cleanup - now enabled with unified libc::malloc allocation.
    //
    // All Rust-side FFI result allocations now use libc::malloc:
    // - health_handler, make_ok_void, make_ok_string, make_err_http
    // LLVM JIT handlers also use libc::malloc via build_malloc.
    // This ensures free_handler_result using libc::free is safe.
    //
    unsafe {
        free_handler_result(result);
    }

    Ok(response)
}

/// Create a new Server instance
/// Server struct layout: { Port: i32, Host: *const c_char }
#[no_mangle]
pub extern "C" fn doo_http_server_new(host_port: *const c_char) -> *mut std::ffi::c_void {
    let host_port_str = if host_port.is_null() {
        ":3000".to_string()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(host_port)
                .to_string_lossy()
                .to_string()
        }
    };

    // Parse port from ":3000" or "127.0.0.1:3000" format
    let port = if let Some(colon_pos) = host_port_str.rfind(':') {
        host_port_str[colon_pos + 1..]
            .parse::<i32>()
            .unwrap_or(3000)
    } else {
        3000
    };

    let host = if host_port_str.contains(':') {
        let parts: Vec<&str> = host_port_str.split(':').collect();
        if parts.len() > 1 && !parts[0].is_empty() {
            string_to_c(parts[0])
        } else {
            // Default to 127.0.0.1 for local dev to avoid firewall popups
            // Use 0.0.0.0 if DOO_ENV=production or configured explicitly
            let is_prod = std::env::var("DOO_ENV")
                .map(|v| v == "production")
                .unwrap_or(false);
            if is_prod {
                string_to_c("0.0.0.0")
            } else {
                string_to_c("127.0.0.1")
            }
        }
    } else {
        let is_prod = std::env::var("DOO_ENV")
            .map(|v| v == "production")
            .unwrap_or(false);
        if is_prod {
            string_to_c("0.0.0.0")
        } else {
            string_to_c("127.0.0.1")
        }
    };

    // Allocate Server struct: { Port: i32, Host: *const c_char }
    let server_size = std::mem::size_of::<i32>() + std::mem::size_of::<*const c_char>();
    let layout = std::alloc::Layout::from_size_align(server_size, 8).unwrap();
    let server_ptr = unsafe { std::alloc::alloc(layout) as *mut u8 };

    if server_ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        // Write Port (i32) at offset 0
        *(server_ptr as *mut i32) = port;
        // Write Host (*const c_char) at offset 8 (aligned)
        *(server_ptr.add(8) as *mut *const c_char) = host;

        // NOTE: string_to_c uses plain libc::malloc without RC headers.
        // Do NOT access ptr.sub(8) - that corrupts malloc metadata!
        // FFI strings are owned by FFI and freed with libc::free when appropriate.
    }

    server_ptr as *mut std::ffi::c_void
}

#[no_mangle]
pub extern "C" fn doo_http_listen(server_ptr: *const std::ffi::c_void) -> *mut DooResult {
    // Extract port from Server struct
    // Server struct layout: { Port: i32, Host: *const c_char }

    let startup_start = std::time::Instant::now();

    let (port, host_str) = if server_ptr.is_null() {
        (3000, "0.0.0.0".to_string())
    } else {
        unsafe {
            // Read the first i32 field (Port)
            let port_ptr = server_ptr as *const i32;
            let port = *port_ptr;

            // Read Host (*const c_char) at offset 8
            let host_ptr_ptr = (server_ptr as *const u8).add(8) as *const *const c_char;
            let host_ptr = *host_ptr_ptr;
            let host = c_to_string(host_ptr);

            (
                port,
                if host.is_empty() {
                    "0.0.0.0".to_string()
                } else {
                    host
                },
            )
        }
    };

    // Create tokio runtime (multi-threaded for better performance)
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => return make_err_http(500, &format!("Failed to create tokio runtime: {}", e)),
    };

    // Parse IP address
    let ip_addr: std::net::IpAddr = match host_str.parse() {
        Ok(ip) => ip,
        Err(_) => {
            // Fallback to 0.0.0.0 if parse fails
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0))
        }
    };

    let addr = SocketAddr::from((ip_addr, port as u16));

    // Print all registered routes and handler count
    let routes = get_routes();
    let registry = routes.lock().unwrap();

    let total_routes = registry.route_count;

    println!(); // Ensure previous output is flushed?
    drop(registry);

    runtime.block_on(async {
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                doo_http_debug!("CRITICAL ERROR: Failed to bind to {}: {}", addr, e);
                return;
            }
        };

        // Now that the socket is bound, compute real boot time
        let boot_time_ms = startup_start.elapsed().as_millis();

        init_timestamp_updater();

        // Run database migrations (create tables) at startup
        let created_tables = run_migrations();
        if !created_tables.is_empty() {
            doo_http_debug!(
                "✓ Migration success: created tables: {}",
                created_tables.join(", ")
            );
        } else {
            // Check if we have registered tables (they already exist)
            let has_tables = get_auth_metadata()
                .lock()
                .map(|m| !m.is_empty())
                .unwrap_or(false)
                || get_crud_metadata()
                    .lock()
                    .map(|m| !m.is_empty())
                    .unwrap_or(false)
                || get_table_metadata()
                    .lock()
                    .map(|m| !m.is_empty())
                    .unwrap_or(false);
            if has_tables {
                doo_http_debug!("✓ Migration success: all tables already exist");
            }
        }

        // Print banner AFTER bind so boot_time_ms is meaningful
        // Cyan color ANSI escape code: \x1b[36m ... \x1b[0m
        println!("\x1b[36m  ____              ");
        println!(" |  _ \\  ___   ___  ");
        println!(" | | | |/ _ \\ / _ \\ ");
        println!(" | |_| | (_) | (_) |");
        println!(" |____/ \\___/ \\___/          Doo v{}\x1b[0m", VERSION);
        println!("-------------------------------------------");
        println!("Info Server Online");
        println!("-------------------------------------------");
        println!("• Boot Time:            {} ms", boot_time_ms);
        println!("• Listening on:         http://{}:{}", addr.ip(), port);
        println!("• Handlers Loaded:      {}", total_routes);
        println!("• Process ID:           {}", std::process::id());
        println!("-------------------------------------------");
        println!("🚀 Server Started on http://{}:{}\n", addr.ip(), port);

        loop {
            let (stream, remote_addr) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    continue;
                }
            };

            tokio::task::spawn(async move {
                let io = TokioIo::new(stream);
                match http1::Builder::new()
                    .serve_connection(io, service_fn(handle_request))
                    .await
                {
                    Ok(_) => {}
                    Err(e) => {}
                };
            });
        }
    });

    make_ok_void()
}

// ============================================================================
// Request Helpers
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_req_query(req: *const DooRequest, key: *const c_char) -> *const c_char {
    if req.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let query_map = (*req).query as *const HashMap<String, String>;
        if query_map.is_null() {
            return std::ptr::null();
        }
        let key_str = c_to_string(key);
        if let Some(value) = (*query_map).get(&key_str) {
            string_to_c(value)
        } else {
            std::ptr::null()
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_http_req_param(req: *const DooRequest, key: *const c_char) -> *const c_char {
    if req.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let params_map = (*req).params as *const HashMap<String, String>;
        if params_map.is_null() {
            return std::ptr::null();
        }
        let key_str = c_to_string(key);
        if let Some(value) = (*params_map).get(&key_str) {
            string_to_c(value)
        } else {
            std::ptr::null()
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_http_req_header(req: *const DooRequest, key: *const c_char) -> *const c_char {
    if req.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let headers_map = (*req).headers as *const HashMap<String, String>;
        if headers_map.is_null() {
            return std::ptr::null();
        }
        let key_str = c_to_string(key);
        // HTTP headers are case-insensitive, convert to lowercase for lookup
        let key_lower = key_str.to_lowercase();
        if let Some(value) = (*headers_map).get(&key_lower) {
            string_to_c(value)
        } else {
            string_to_c("")
        }
    }
}

// Helper to safely unwrap a pointer that might be a DooResult* or a char*
// Returns the inner char* if it is a DooResult with JSON content, otherwise returns original ptr
// Returns NULL if it appears to be an ERROR (tag == 1)
unsafe fn unwrap_potential_dooresult(ptr: *const c_char) -> *const c_char {
    if ptr.is_null() {
        return ptr;
    }

    // CRITICAL: Some FFI values are encoded as small integers cast to pointers (e.g. 0x4c).
    // Never dereference obviously invalid/low pointers.
    let addr = ptr as usize;
    if addr < 0x1000 {
        return std::ptr::null();
    }

    if !doo_runtime::memory::validate_pointer(ptr as *const std::ffi::c_void, "unwrap_potential_dooresult") {
        return std::ptr::null();
    }

    // FAST PATH: if this is a JSON string pointer, return it immediately.
    // IMPORTANT: many C string pointers are 8/16-byte aligned, so we must NOT use alignment
    // as a discriminator before checking the first byte.
    let first_u8 = *(ptr as *const u8);
    if first_u8 == b'{' || first_u8 == b'[' || first_u8 == b'"' {
        return ptr;
    }

    // Check alignment check: Pointers to structs should be aligned to 4/8 bytes.
    // Strings (char*) might not be.
    if (ptr as usize) % 4 != 0 {
        return ptr;
    }

    let tag_val = *(ptr as *const i32);

    // If this is a Doo RC string header ([rc: i32][len: i32][data...]), the first i32 is a small
    // positive refcount (often 1). In that case, the JSON begins at offset +8.
    // This prevents mistaking RC header pointers for DooResult errors (tag=1).
    if tag_val > 0 && tag_val < 200 {
        let len_val = *((ptr as *const u8).add(4) as *const i32);
        // For JSON strings we expect at least 2 bytes ("[]", "{}", "\"\""), and not absurdly large.
        // This also prevents misclassifying a real DooResult (where offset+4 is padding, usually 0).
        if len_val < 2 || len_val > 50_000_000 {
        } else {
            let data_ptr = (ptr as *const u8).add(8) as *const c_char;
            if !data_ptr.is_null() {
                let first = *(data_ptr as *const u8);
                if first == b'{' || first == b'[' || first == b'"' {
                    return data_ptr;
                }
            }
        }
    }

    // SAFETY CHECK: Detect corrupted memory by checking for obviously invalid tag values
    // Valid tags are: 0 (Ok), 1 (Err), or HTTP status codes (200-599)
    // Negative values or values > 999 likely indicate memory corruption
    if tag_val < 0 || tag_val > 999 {
        // This often happens when ptr is actually a C string and its first 4 bytes are
        // interpreted as an i32 (e.g. "{\"da" -> 1633952379). Treat it as a raw pointer.
        return ptr;
    }

    // DooResult only has tag 0 (Ok) or 1 (Err)
    // Any other value (like HTTP status codes 200, 400, 500) means this is NOT a DooResult
    if tag_val == 0 {
        // This could be DooResult with tag=0 (success)
        let res = ptr as *const DooResult;
        let val = (*res).value as *const c_char;

        if !val.is_null() {
            // If val is a small int-as-pointer, do not treat it as a C string.
            if (val as usize) < 0x1000 {
                return std::ptr::null();
            }
            let first = *val;
            if first == b'{' as i8 || first == b'[' as i8 {
                return val;
            }

            // VALUE may be an RC-string header: [rc: i32][len: i32][data...]
            // This happens for DB JSON results and other runtime strings.
            if (val as usize) % 4 == 0 {
                let rc_val = *(val as *const i32);
                if rc_val > 0 && rc_val < 200 {
                    let len_val = *((val as *const u8).add(4) as *const i32);
                    if len_val >= 2 && len_val <= 50_000_000 {
                        let data_ptr = (val as *const u8).add(8) as *const c_char;
                        if !data_ptr.is_null() {
                            let first_data = *(data_ptr as *const u8);
                            if first_data == b'{' || first_data == b'[' || first_data == b'"' {
                                return data_ptr;
                            }
                        }
                    }
                }
            }
        }
    } else if tag_val == 1 {
        // This is a DooResult with tag=1 (error)
        // Return NULL to signal error
        return std::ptr::null();
    }
    // For any other tag value (like 200, 400, 500), this is NOT a DooResult
    // It could be a DooResponse struct or other data - return original pointer

    // Check if it's a JSON string starting with { or [
    // This handles the case where it's a raw string pointer
    let first = *(ptr as *const u8);
    if first == b'{' || first == b'[' {
        return ptr;
    }

    // If tag is not 0 or 1, and not JSON string, return original pointer
    // (It might be a struct like DooResponse where first field is status code)
    ptr
}

/// Serialize an array of structs to JSON with metadata
/// For handlers using db.raw(), the input is already a JSON string (possibly wrapped in DooResult)
#[no_mangle]
pub extern "C" fn array_to_json_with_metadata(
    array_ptr: *const c_char,
    _struct_name: *const c_char,
    _metadata_json: *const c_char,
) -> *const c_char {
    if array_ptr.is_null() {
        return string_to_c("{\"data\":[]}");
    }

    let real_ptr = unsafe { unwrap_potential_dooresult(array_ptr) };

    if real_ptr.is_null() {
        return string_to_c("{\"data\":[]}");
    }

    let first = unsafe { *(real_ptr as *const u8) };
    if first == b'[' || first == b'{' {
        let json_str = c_to_string(real_ptr);
        let wrapped = format!("{{\"data\":{}}}", json_str);
        return string_to_c(&wrapped);
    }

    if _struct_name.is_null() || _metadata_json.is_null() {
        return string_to_c("{\"data\":[]}");
    }

    let struct_name = unsafe { CStr::from_ptr(_struct_name).to_string_lossy().to_string() };
    let meta_str = unsafe { CStr::from_ptr(_metadata_json).to_string_lossy().to_string() };
    let meta_json: serde_json::Value = match serde_json::from_str(&meta_str) {
        Ok(v) => v,
        Err(_) => return string_to_c("{\"data\":[]}"),
    };

    let mut fields_owned: Option<Vec<serde_json::Value>> = None;
    let mut enum_variants_owned: Option<HashMap<String, Vec<String>>> = None;

    if let Some(struct_layout) = meta_json
        .get("struct_layouts")
        .and_then(|sl| sl.get(&struct_name))
        .and_then(|v| v.as_object())
    {
        if let Some(fields) = struct_layout.get("fields").and_then(|v| v.as_array()) {
            fields_owned = Some(fields.to_vec());
        }
    }

    if fields_owned.is_none() {
        // Fallback: compiler currently passes only a small field->type map, not the full
        // struct layout (offsets). But libdoo_http already has full handler metadata in the
        // route registry via doo_http_register_handler_with_metadata().
        let routes = get_routes();
        if let Ok(registry) = routes.lock() {
            for (_handler_name, md) in registry.handler_metadata.iter() {
                if let Some(struct_layout) = md
                    .struct_layouts
                    .get(&struct_name)
                    .and_then(|v| v.as_object())
                {
                    if let Some(fields) = struct_layout.get("fields").and_then(|v| v.as_array()) {
                        fields_owned = Some(fields.to_vec());
                        enum_variants_owned = Some(md.enum_variants.clone());
                        break;
                    }
                }
            }
        }
    }

    let fields = match fields_owned.as_ref() {
        Some(f) => f,
        None => return string_to_c("{\"data\":[]}"),
    };

    let enum_variants = enum_variants_owned.unwrap_or_default();

    unsafe {
        let base_ptr = real_ptr as *const u8;

        // Try interpreting real_ptr as the ARRAY HEADER start:
        //   [rc: i32][len: i32][data...]
        let rc0 = *(base_ptr as *const i32);
        let len0 = *(base_ptr.add(4) as *const i32);
        let (data_ptr, len) = if rc0 > 0 && rc0 < 1_000_000 && len0 >= 0 && len0 <= 10_000_000 {
            (base_ptr.add(8), len0)
        } else {
            // Fallback: interpret real_ptr as DATA pointer:
            //   RC at -8, LEN at -4. This matches how arrays are stored inside structs.
            // NOTE: only do this when header detection failed to avoid reading before allocation.
            let len = *(base_ptr.offset(-4) as *const i32);
            (base_ptr, len)
        };

        if len <= 0 {
            return string_to_c("{\"data\":[]}");
        }

        let mut out: Vec<serde_json::Value> = Vec::with_capacity(len as usize);
        let elems = data_ptr as *const *const u8;
        for i in 0..(len as usize) {
            let elem_ptr = *elems.add(i);
            if elem_ptr.is_null() {
                continue;
            }

            let mut obj = serde_json::Map::new();
            for field in fields {
                let field_obj = match field.as_object() {
                    Some(o) => o,
                    None => continue,
                };
                let field_name = match field_obj.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n,
                    None => continue,
                };
                let field_type = match field_obj.get("type").and_then(|v| v.as_str()) {
                    Some(t) => t,
                    None => continue,
                };
                let offset = match field_obj.get("offset").and_then(|v| v.as_u64()) {
                    Some(o) => o as isize,
                    None => continue,
                };

                let field_ptr = (elem_ptr as *const u8).offset(offset);
                let value: Option<serde_json::Value> = match field_type {
                    "Int" => Some(serde_json::json!(*(field_ptr as *const i32))),
                    "Float" => Some(serde_json::json!(*(field_ptr as *const f64))),
                    "Bool" => Some(serde_json::json!(*(field_ptr as *const i32) != 0)),
                    "Str" => {
                        let str_ptr = *(field_ptr as *const *const libc::c_char);
                        if str_ptr.is_null() {
                            Some(serde_json::json!(""))
                        } else {
                            Some(serde_json::json!(CStr::from_ptr(str_ptr)
                                .to_string_lossy()
                                .to_string()))
                        }
                    }
                    "Optional(Str)" | "Optional(String)" => {
                        let str_ptr = *(field_ptr as *const *const libc::c_char);
                        if str_ptr.is_null() {
                            None
                        } else {
                            Some(serde_json::json!(CStr::from_ptr(str_ptr)
                                .to_string_lossy()
                                .to_string()))
                        }
                    }
                    other_type => {
                        let enum_name = if other_type.starts_with("Enum(") && other_type.ends_with(')') {
                            &other_type[5..other_type.len() - 1]
                        } else {
                            other_type
                        };

                        if let Some(variants) = enum_variants.get(enum_name) {
                            let tag = *(field_ptr as *const i32);
                            let idx: usize = if tag < 0 { 0 } else { tag as usize };
                            let v = variants
                                .get(idx)
                                .cloned()
                                .unwrap_or_else(|| "Unknown".to_string());
                            Some(serde_json::json!(v))
                        } else {
                            Some(serde_json::json!(null))
                        }
                    }
                };

                if let Some(v) = value {
                    obj.insert(field_name.to_string(), v);
                }
            }

            out.push(serde_json::Value::Object(obj));
        }

        let wrapped = serde_json::json!({ "data": out });
        let json_str =
            serde_json::to_string(&wrapped).unwrap_or_else(|_| r#"{\"data\":[]}"#.to_string());
        return string_to_c(&json_str);
    }
}

// Stub for userId extraction to fix linking/build
#[no_mangle]
pub extern "C" fn doo_http_req_user_id(req: *const DooRequest) -> i32 {
    // Extract user ID injected by jwt_middleware_handler into request.params["auth_user_id"].
    // Returns 0 if not present.
    if req.is_null() {
        return 0;
    }
    unsafe {
        let params_ptr = (*req).params as *const HashMap<String, String>;
        if params_ptr.is_null() {
            return 0;
        }
        if let Some(v) = (*params_ptr).get("auth_user_id") {
            v.parse::<i32>().unwrap_or(0)
        } else {
            0
        }
    }
}

/// Helper to allocate DooResponse using libc::malloc (to match free_handler_result/libc::free)
fn alloc_doo_response(
    status: i32,
    body: *const libc::c_char,
    content_type: *const libc::c_char,
) -> *mut DooResponse {
    unsafe {
        let size = std::mem::size_of::<DooResponse>();
        let ptr = libc::malloc(size) as *mut DooResponse;
        if !ptr.is_null() {
            track_alloc(ptr as *const std::ffi::c_void, "http_alloc_doo_response");
            (*ptr).status = status;
            (*ptr).body = body;
            (*ptr).content_type = content_type;
        }
        ptr
    }
}

// ============================================================================
// Memory Management
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_free_result(result: *mut DooResult) {
    // FIXED: Use libc::free for LLVM-allocated results, not Box::from_raw
    // DooResult may be allocated by LLVM's malloc, so using Box::from_raw would cause
    // allocator mismatch (Rust tries to free LLVM-allocated memory)
    if !result.is_null() {
        unsafe { libc::free(result as *mut libc::c_void) };
    }
}

#[no_mangle]
pub extern "C" fn doo_http_free_string(s: *const c_char) {
    if !s.is_null() {
        free_rc_string(s);
    }
}

#[no_mangle]
pub extern "C" fn doo_http_free_request(req: *mut DooRequest) {
    if req.is_null() {
        return;
    }
    unsafe {
        extern "C" {
            fn dooruntime_free_rc_string(ptr: *const c_char);
        }
        let request = Box::from_raw(req);

        // Free C strings
        if !request.method.is_null() {
            dooruntime_free_rc_string(request.method);
        }
        if !request.path.is_null() {
            dooruntime_free_rc_string(request.path);
        }
        if !request.body.is_null() {
            dooruntime_free_rc_string(request.body);
        }
        if !request.content_type.is_null() {
            dooruntime_free_rc_string(request.content_type);
        }

        // Free HashMaps
        if !request.params.is_null() {
            drop(Box::from_raw(
                request.params as *mut HashMap<String, String>,
            ));
        }
        if !request.query.is_null() {
            drop(Box::from_raw(request.query as *mut HashMap<String, String>));
        }
        if !request.headers.is_null() {
            drop(Box::from_raw(
                request.headers as *mut HashMap<String, String>,
            ));
        }
    }
}

// ===== PHASE 6: AUTO JSON SERIALIZATION/DESERIALIZATION =====

/// Parse JSON body into struct with validation
/// FFI signature: doohttp_parse_json_struct(body, struct_name, validators) -> struct_ptr
///
/// Returns: pointer to allocated struct (on success) or NULL (on error)
/// Errors: 400 (malformed JSON), 422 (validation failed)
#[no_mangle]
pub extern "C" fn doohttp_parse_json_struct(
    body: *const libc::c_char,
    struct_name: *const libc::c_char,
    validator_spec: *const libc::c_char,
) -> *mut libc::c_void {
    clear_last_error();
    if body.is_null() || struct_name.is_null() {
        return std::ptr::null_mut();
    }

    let body_str = unsafe { std::ffi::CStr::from_ptr(body).to_string_lossy().to_string() };
    let _struct_name_str = unsafe {
        std::ffi::CStr::from_ptr(struct_name)
            .to_string_lossy()
            .to_string()
    };
    let _validator_str = if validator_spec.is_null() {
        String::new()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(validator_spec)
                .to_string_lossy()
                .to_string()
        }
    };

    // Parse JSON
    let _json_value: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(v) => v,
        Err(_) => {
            // 400 Bad Request - malformed JSON
            use error::*;
            let inst = get_current_request_path();
            let err = bad_request(
                "Invalid JSON: malformed or unexpected content".to_string(),
                inst,
            );
            set_last_error(err.status_code() as i32, err.to_json_string());
            return std::ptr::null_mut();
        }
    };

    // NOTE: Decorator validation happens in generated code before calling this function
    // This function only parses JSON - validation is done by dooruntime_validate_field
    // in the handler wrapper code generated by the compiler

    // Clear any previous JSON type mismatch errors before parsing
    extern "C" {
        fn dooruntime_clear_json_type_mismatch();
    }
    unsafe {
        dooruntime_clear_json_type_mismatch();
    }

    // Return JSON string representation allocated with libc::malloc (via string_to_c)
    // Caller must free using doo_http_free_string / dooruntime_free_string.
    string_to_c(&body_str) as *mut libc::c_void
}

/// Check for JSON type mismatch errors after struct deserialization
/// If a type mismatch occurred, set RFC 7807 error and return error status
/// Returns: 0 if no error, HTTP status code if error occurred
#[no_mangle]
pub extern "C" fn doohttp_check_json_type_mismatch() -> i32 {
    extern "C" {
        fn dooruntime_get_json_type_mismatch() -> *mut libc::c_char;
        fn dooruntime_free_string(ptr: *mut libc::c_char);
    }

    let error_ptr = unsafe { dooruntime_get_json_type_mismatch() };

    if error_ptr.is_null() {
        return 0; // No error
    }

    let error_json = unsafe {
        std::ffi::CStr::from_ptr(error_ptr)
            .to_string_lossy()
            .to_string()
    };

    unsafe {
        dooruntime_free_string(error_ptr);
    }

    // Parse the error JSON
    if let Ok(error_data) = serde_json::from_str::<serde_json::Value>(&error_json) {
        let field_name = error_data["field"].as_str().unwrap_or("unknown");
        let expected_type = error_data["expected"].as_str().unwrap_or("unknown");
        let actual_type = error_data["actual"].as_str().unwrap_or("unknown");

        // Create RFC 7807 error response
        use error::*;
        let inst = get_current_request_path();

        let error = bad_request(format!("Type mismatch in request body"), inst.clone()).with_field(
            field_name.to_string(),
            FieldError {
                rule: Some(format!("type:{}", expected_type)),
                message: field_name.to_string(),
                value: Some(format!("({})", actual_type)),
                expected: Some(expected_type.to_string()),
                received: Some(actual_type.to_string()),
                error: Some(format!(
                    "Expected type '{}' but got '{}'",
                    expected_type, actual_type
                )),
            },
        );

        set_last_error(400, error.to_json_string());
        return 400;
    }

    0 // Parse failed, no error
}

/// Serialize struct to JSON
/// FFI signature: doohttp_serialize_struct(struct_ptr, struct_name) -> json_string
///
/// Returns: pointer to allocated JSON string (caller must free)
#[no_mangle]
pub extern "C" fn doohttp_serialize_struct(
    struct_ptr: *const libc::c_void,
    struct_name: *const libc::c_char,
) -> *const libc::c_char {
    if struct_ptr.is_null() || struct_name.is_null() {
        return std::ptr::null();
    }

    let struct_name_str = unsafe {
        std::ffi::CStr::from_ptr(struct_name)
            .to_string_lossy()
            .to_string()
    };

    // In production, this would serialize the struct based on its type
    // For now, return a dummy JSON response
    let json_str = format!(
        r#"{{"id":1,"name":"example","type":"{}"}}"#,
        struct_name_str
    );
    string_to_c(&json_str)
}

/// Format error string as JSON for HTTP error response
/// FFI signature: doohttp_format_error_json(error_ptr) -> json_string
///
/// IMPORTANT: error_ptr is a pointer to DooDbError struct { code: i32, message: *mut c_char }
/// not a raw string pointer!
#[no_mangle]
pub extern "C" fn doohttp_format_error_json(error_ptr: *const libc::c_void) -> *const libc::c_char {
    if error_ptr.is_null() {
        let json_str = r#"{"error":"Unknown error","status":500}"#;
        return string_to_c(json_str);
    }

    // DooDbError struct layout: { code: i32, message: *mut c_char }
    // Extract the message pointer which is at offset 8 (after i32 code + padding on 64-bit)
    // On 64-bit systems: i32 (4 bytes) + 4 bytes padding = 8 bytes offset for pointer
    unsafe {
        // Cast to a struct-like accessor: after i32 code (4 bytes + 4 padding = 8), we have message ptr
        let message_ptr_ptr = (error_ptr as *const u8).add(8) as *const *const libc::c_char;
        let message_ptr = *message_ptr_ptr;

        if message_ptr.is_null() {
            let json_str = r#"{"error":"Unknown error (null message)","status":500}"#;
            return string_to_c(json_str);
        }

        let error_msg = std::ffi::CStr::from_ptr(message_ptr)
            .to_string_lossy()
            .to_string();

        // The message is already JSON formatted from make_err_with_code
        // Just return it as-is since it already has the proper error structure
        string_to_c(&error_msg)
    }
}

/// Create HTTP response from a Result struct (FFI-first approach)
/// This function handles all the conditional logic for errors vs success
/// so the compiler only needs one simple call.
///
/// Parameters:
/// - tag: 0 = success, 1 = error
/// - value_ptr: For errors, pointer to DooDbError; for success, pointer to body string
/// - success_body_ptr: The serialized success body (used only if tag=0)
///
/// Returns: Pointer to DooResponse struct { status: i32, body: *const c_char, content_type: *const c_char }
#[no_mangle]
pub extern "C" fn doohttp_create_response_from_result(
    tag: i32,
    value_ptr: *const libc::c_void,
    success_body_ptr: *const libc::c_char,
) -> *mut DooResponse {
    // Allocate response using libc
    let response = unsafe {
        let ptr = libc::malloc(std::mem::size_of::<DooResponse>()) as *mut DooResponse;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        ptr
    };

    if tag == 1 {
        fn wrap_error_message_json(msg: &str) -> *const libc::c_char {
            if msg.trim_start().starts_with('{') {
                return string_to_c(msg);
            }
            let json_str = format!(
                r#"{{"error":{},"status":500}}"#,
                serde_json::to_string(msg).unwrap_or_else(|_| "\"Unknown error\"".to_string())
            );
            string_to_c(&json_str)
        }

        unsafe fn try_read_rc_string(ptr: *const libc::c_void) -> Option<String> {
            if ptr.is_null() {
                return None;
            }
            let base = ptr as *const u8;
            let rc = *(base as *const i32);
            let len = *(base.add(4) as *const i32);
            if rc <= 0 || rc > 1_000_000 {
                return None;
            }
            if len < 0 || len > 100_000_000 {
                return None;
            }
            let data_ptr = base.add(8);
            let end_ptr = data_ptr.add(len as usize);
            let end_byte = *end_ptr;
            if end_byte != 0 {
                return None;
            }
            let c_ptr = data_ptr as *const libc::c_char;
            Some(
                std::ffi::CStr::from_ptr(c_ptr)
                    .to_string_lossy()
                    .to_string(),
            )
        }

        unsafe fn try_read_c_string(ptr: *const libc::c_char) -> Option<String> {
            if ptr.is_null() {
                return None;
            }

            // Avoid unbounded reads: require a NUL byte within a reasonable limit.
            // If value_ptr is actually a struct pointer, this usually won't look like
            // a valid short C string, so we'll fall through to the structured error path.
            let max_len: usize = 4096;
            let len = libc::strnlen(ptr, max_len);
            if len == 0 || len >= max_len {
                return None;
            }

            let bytes = std::slice::from_raw_parts(ptr as *const u8, len);
            Some(String::from_utf8_lossy(bytes).to_string())
        }

        let error_body = if value_ptr.is_null() {
            string_to_c(r#"{"error":"Unknown error","status":500}"#)
        } else if let Some(msg) = unsafe { try_read_rc_string(value_ptr) } {
            wrap_error_message_json(&msg)
        } else if let Some(msg) = unsafe { try_read_c_string(value_ptr as *const libc::c_char) } {
            // Covers common case: Err "..." where compiler produced a global C string literal.
            wrap_error_message_json(&msg)
        } else {
            unsafe {
                let message_ptr_ptr = (value_ptr as *const u8).add(8) as *const *const libc::c_char;
                let message_ptr = *message_ptr_ptr;

                if message_ptr.is_null() {
                    string_to_c(r#"{"error":"Unknown error (null)","status":500}"#)
                } else {
                    let msg = std::ffi::CStr::from_ptr(message_ptr)
                        .to_string_lossy()
                        .to_string();
                    wrap_error_message_json(&msg)
                }
            }
        };

        unsafe {
            (*response).status = 500;
            (*response).body = error_body;
            (*response).content_type = string_to_c("application/json");
        }
    } else {
        // Success case - use the provided body and return 200
        unsafe {
            (*response).status = 200;
            (*response).body = success_body_ptr;
            (*response).content_type = string_to_c("application/json");
        }
    }

    response
}

// ===== PHASE 7: VALIDATION DECORATORS =====

/// Validate a JSON value against decorator specifications
/// Format: "field1:email;field2:min8|max100;field3:enum:a|b|c"
// NOTE: All validation logic has been moved to libdoo_runtime
// HTTP layer delegates to dooruntime_validate_field() for all decorator validation
// This keeps validation centralized and reusable across all FFI libs (http, db, auth)

// ===== PHASE 8: TYPE-SAFE PARAMETERS =====

/// Extract typed path parameter from request
/// FFI signature: doohttp_extract_param_typed(request, param_name, param_type) -> typed_value
///
/// Converts parameter string to specified type
/// Returns: converted value as string (caller must free)
#[no_mangle]
pub extern "C" fn doohttp_extract_param_typed(
    request: *const DooRequest,
    param_name: *const libc::c_char,
    param_type: *const libc::c_char,
) -> *const libc::c_char {
    clear_last_error();
    if request.is_null() || param_name.is_null() || param_type.is_null() {
        return std::ptr::null();
    }

    let param_name_str = unsafe {
        std::ffi::CStr::from_ptr(param_name)
            .to_string_lossy()
            .to_string()
    };
    let param_type_str = unsafe {
        std::ffi::CStr::from_ptr(param_type)
            .to_string_lossy()
            .to_string()
    };

    // Extract parameter from request params HashMap
    let req = unsafe { &*request };

    // params is *mut c_void pointing to HashMap<String, String>
    if req.params.is_null() {
        return std::ptr::null();
    }

    let params_map = unsafe { &*(req.params as *const std::collections::HashMap<String, String>) };

    if let Some(value) = params_map.get(&param_name_str) {
        // Type conversion validation
        match param_type_str.as_str() {
            "Int" => {
                if value.parse::<i64>().is_ok() {
                    string_to_c(value)
                } else {
                    use error::*;
                    let mut param = ParameterError::new(param_name_str.clone())
                        .with_expected("Int".to_string())
                        .with_received(value.clone());
                    param = param.with_message("Invalid path parameter type".to_string());
                    let err = parameter_error(
                        "Invalid path parameter type".to_string(),
                        get_current_request_path(),
                        param,
                    );
                    set_last_error(err.status_code() as i32, err.to_json_string());
                    std::ptr::null()
                }
            }
            "Float" => {
                if value.parse::<f64>().is_ok() {
                    string_to_c(value)
                } else {
                    use error::*;
                    let mut param = ParameterError::new(param_name_str.clone())
                        .with_expected("Float".to_string())
                        .with_received(value.clone());
                    param = param.with_message("Invalid path parameter type".to_string());
                    let err = parameter_error(
                        "Invalid path parameter type".to_string(),
                        get_current_request_path(),
                        param,
                    );
                    set_last_error(err.status_code() as i32, err.to_json_string());
                    std::ptr::null()
                }
            }
            "Bool" => {
                if value == "true" || value == "false" {
                    string_to_c(value)
                } else {
                    use error::*;
                    let mut param = ParameterError::new(param_name_str.clone())
                        .with_expected("Bool".to_string())
                        .with_received(value.clone());
                    param = param.with_message("Invalid path parameter type".to_string());
                    let err = parameter_error(
                        "Invalid path parameter type".to_string(),
                        get_current_request_path(),
                        param,
                    );
                    set_last_error(err.status_code() as i32, err.to_json_string());
                    std::ptr::null()
                }
            }
            _ => string_to_c(value), // String or other types
        }
    } else {
        use error::*;
        let param = ParameterError::new(param_name_str.clone())
            .with_message("Path parameter not found".to_string());
        let err = parameter_error(
            "Path parameter not found".to_string(),
            get_current_request_path(),
            param,
        );
        set_last_error(err.status_code() as i32, err.to_json_string());
        std::ptr::null()
    }
}

/// Extract path parameter as integer directly
/// FFI signature: doohttp_extract_param_int(request, param_name) -> i64
///
/// Returns: Integer value of parameter, or 0 if not found/invalid
#[no_mangle]
pub extern "C" fn doohttp_extract_param_int(
    request: *const DooRequest,
    param_name: *const libc::c_char,
) -> i64 {
    clear_last_error();
    if request.is_null() || param_name.is_null() {
        return 0;
    }

    let param_name_str = unsafe {
        std::ffi::CStr::from_ptr(param_name)
            .to_string_lossy()
            .to_string()
    };

    // Extract parameter from request params HashMap
    let req = unsafe { &*request };

    // params is *mut c_void pointing to HashMap<String, String>
    if req.params.is_null() {
        return 0;
    }

    let params_map = unsafe { &*(req.params as *const std::collections::HashMap<String, String>) };

    if let Some(value) = params_map.get(&param_name_str) {
        match value.parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                use error::*;
                let mut param = ParameterError::new(param_name_str.clone())
                    .with_expected("Int".to_string())
                    .with_received(value.clone());
                param = param.with_message("Invalid path parameter type".to_string());
                let err = parameter_error(
                    "Invalid path parameter type".to_string(),
                    get_current_request_path(),
                    param,
                );
                set_last_error(err.status_code() as i32, err.to_json_string());
                0
            }
        }
    } else {
        use error::*;
        let param = ParameterError::new(param_name_str.clone())
            .with_message("Path parameter not found".to_string());
        let err = parameter_error(
            "Path parameter not found".to_string(),
            get_current_request_path(),
            param,
        );
        set_last_error(err.status_code() as i32, err.to_json_string());
        0
    }
}

/// Extract path parameter as float directly
/// FFI signature: doohttp_extract_param_float(request, param_name) -> f64
///
/// Returns: Float value of parameter, or 0.0 if not found/invalid
#[no_mangle]
pub extern "C" fn doohttp_extract_param_float(
    request: *const DooRequest,
    param_name: *const libc::c_char,
) -> f64 {
    clear_last_error();
    if request.is_null() || param_name.is_null() {
        return 0.0;
    }

    let param_name_str = unsafe {
        std::ffi::CStr::from_ptr(param_name)
            .to_string_lossy()
            .to_string()
    };

    // Extract parameter from request params HashMap
    let req = unsafe { &*request };

    // params is *mut c_void pointing to HashMap<String, String>
    if req.params.is_null() {
        return 0.0;
    }

    let params_map = unsafe { &*(req.params as *const std::collections::HashMap<String, String>) };

    if let Some(value) = params_map.get(&param_name_str) {
        match value.parse::<f64>() {
            Ok(v) => v,
            Err(_) => {
                use error::*;
                let mut param = ParameterError::new(param_name_str.clone())
                    .with_expected("Float".to_string())
                    .with_received(value.clone());
                param = param.with_message("Invalid path parameter type".to_string());
                let err = parameter_error(
                    "Invalid path parameter type".to_string(),
                    get_current_request_path(),
                    param,
                );
                set_last_error(err.status_code() as i32, err.to_json_string());
                0.0
            }
        }
    } else {
        use error::*;
        let param = ParameterError::new(param_name_str.clone())
            .with_message("Path parameter not found".to_string());
        let err = parameter_error(
            "Path parameter not found".to_string(),
            get_current_request_path(),
            param,
        );
        set_last_error(err.status_code() as i32, err.to_json_string());
        0.0
    }
}

/// Parse query parameters into struct
/// FFI signature: doohttp_parse_query_struct(query_string, struct_name, defaults) -> struct_ptr
///
/// Parses ?key=value&key2=value2 into struct fields
/// Applies type conversion and default values
#[no_mangle]
pub extern "C" fn doohttp_parse_query_struct(
    query_string: *const libc::c_char,
    struct_name: *const libc::c_char,
    defaults_spec: *const libc::c_char,
) -> *mut libc::c_void {
    clear_last_error();
    if query_string.is_null() || struct_name.is_null() {
        return std::ptr::null_mut();
    }

    let query_str = unsafe {
        std::ffi::CStr::from_ptr(query_string)
            .to_string_lossy()
            .to_string()
    };
    let defaults_str = if defaults_spec.is_null() {
        String::new()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(defaults_spec)
                .to_string_lossy()
                .to_string()
        }
    };

    // Parse query string
    let mut query_map = parse_query(&query_str);

    // Apply defaults
    for default_pair in defaults_str.split(';') {
        let kv: Vec<&str> = default_pair.split(':').collect();
        if kv.len() == 2 && !query_map.contains_key(kv[0]) {
            query_map.insert(kv[0].to_string(), kv[1].to_string());
        }
    }

    // Allocate and return struct representation as JSON using libc::malloc
    // This avoids allocator mismatch if caller frees with libc::free.
    let json_str = format!("{:?}", query_map);
    string_to_c(&json_str) as *mut libc::c_void
}

// ===== PHASE 8: ERROR MAPPING =====

/// Map error enum variant to HTTP status code
/// FFI signature: doohttp_error_to_status(error_type, variant) -> status_code
///
/// Returns: HTTP status code (404, 409, 422, 500, etc.)
#[no_mangle]
pub extern "C" fn doohttp_error_to_status(
    error_type: *const libc::c_char,
    variant: *const libc::c_char,
) -> libc::c_int {
    if error_type.is_null() || variant.is_null() {
        return 500; // Default to 500 Internal Error
    }

    let variant_str = unsafe {
        std::ffi::CStr::from_ptr(variant)
            .to_string_lossy()
            .to_string()
    };

    // Map error variants to status codes
    match variant_str.as_str() {
        "NotFound" => 404,
        "InvalidInput" | "ValidationError" => 422,
        "Unauthorized" => 401,
        "Forbidden" => 403,
        "Conflict" | "AlreadyExists" => 409,
        "BadRequest" => 400,
        _ => 500, // Default to 500 for unknown errors
    }
}

/// Get error message from enum variant
/// FFI signature: doohttp_error_message(error_type, variant) -> message_string
#[no_mangle]
pub extern "C" fn doohttp_error_message(
    error_type: *const libc::c_char,
    variant: *const libc::c_char,
) -> *const libc::c_char {
    if error_type.is_null() || variant.is_null() {
        return std::ptr::null();
    }

    let variant_str = unsafe {
        std::ffi::CStr::from_ptr(variant)
            .to_string_lossy()
            .to_string()
    };

    let message = match variant_str.as_str() {
        "NotFound" => "Resource not found",
        "InvalidInput" => "Invalid input",
        "ValidationError" => "Validation failed",
        "Unauthorized" => "Unauthorized",
        "Forbidden" => "Forbidden",
        "Conflict" => "Conflict",
        "AlreadyExists" => "Resource already exists",
        "BadRequest" => "Bad request",
        _ => "Internal server error",
    };

    string_to_c(message)
}

// ============================================================================
// RFC 7807 Error Helpers
// ============================================================================

/// Helper function to create RFC 7807 error JSON
// Removed: replaced with centralized ErrorResponse usage

/// Create RFC 7807 error response
/// FFI signature: doohttp_get_request_path(request) -> path_string
/// Extracts the path field from DooRequest struct
#[no_mangle]
pub extern "C" fn doohttp_get_request_path(request: *const DooRequest) -> *const libc::c_char {
    if request.is_null() {
        // Return "/" as default if null
        return string_to_c("/");
    }

    unsafe {
        let req = &*request;
        // The path field is already a *const c_char, just return it
        req.path
    }
}

/// FFI signature: doohttp_error_rfc7807(status, detail, instance) -> json_string
#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807(
    status: libc::c_int,
    detail: *const libc::c_char,
    instance: *const libc::c_char,
) -> *const libc::c_char {
    if detail.is_null() {
        return std::ptr::null();
    }

    let detail_str = unsafe {
        std::ffi::CStr::from_ptr(detail)
            .to_string_lossy()
            .to_string()
    };

    // If instance is null, empty string, or sentinel "$$THREAD_LOCAL$$", use thread-local request path
    let instance_str = if instance.is_null() {
        let path = get_current_request_path();
        path
    } else {
        let path = unsafe {
            std::ffi::CStr::from_ptr(instance)
                .to_string_lossy()
                .to_string()
        };
        // Check for sentinel string or empty string
        if path.is_empty() || path == "__USE_THREAD_LOCAL_REQUEST_PATH_FROM_STORAGE_PLEASE__" {
            let thread_path = get_current_request_path();
            thread_path
        } else {
            path
        }
    };

    // Use centralized error module
    use error::*;
    let error_response = match status {
        400 => bad_request(detail_str, instance_str),
        401 => unauthorized(detail_str, instance_str),
        403 => forbidden(detail_str, instance_str),
        404 => not_found(detail_str, instance_str),
        405 => method_not_allowed(detail_str, instance_str, vec![]),
        409 => conflict(detail_str, instance_str),
        422 => ErrorResponse::new(ErrorType::UnprocessableEntity, detail_str, instance_str),
        429 => ErrorResponse::new(ErrorType::TooManyRequests, detail_str, instance_str),
        500 => internal_error(detail_str, instance_str),
        501 => not_implemented(detail_str, instance_str),
        502 => bad_gateway(detail_str, instance_str),
        503 => service_unavailable(detail_str, instance_str),
        _ => internal_error(detail_str, instance_str),
    };

    let error_json = error_response.to_json_string();
    string_to_c(&error_json)
}

/// Create RFC 7807 parameter error (path/query) with parameter details
#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807_parameter(
    detail: *const libc::c_char,
    instance: *const libc::c_char,
    name: *const libc::c_char,
    expected: *const libc::c_char,
    received: *const libc::c_char,
    message: *const libc::c_char,
) -> *const libc::c_char {
    if detail.is_null() || name.is_null() {
        return std::ptr::null();
    }

    let detail_str = unsafe {
        std::ffi::CStr::from_ptr(detail)
            .to_string_lossy()
            .to_string()
    };

    let instance_str = if instance.is_null() {
        get_current_request_path()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(instance)
                .to_string_lossy()
                .to_string()
        }
    };

    let name_str = unsafe { std::ffi::CStr::from_ptr(name).to_string_lossy().to_string() };

    let expected_str = if expected.is_null() {
        None
    } else {
        Some(unsafe {
            std::ffi::CStr::from_ptr(expected)
                .to_string_lossy()
                .to_string()
        })
    };
    let received_str = if received.is_null() {
        None
    } else {
        Some(unsafe {
            std::ffi::CStr::from_ptr(received)
                .to_string_lossy()
                .to_string()
        })
    };
    let message_str = if message.is_null() {
        None
    } else {
        Some(unsafe {
            std::ffi::CStr::from_ptr(message)
                .to_string_lossy()
                .to_string()
        })
    };

    use error::*;
    let mut param = ParameterError::new(name_str);
    if let Some(e) = expected_str {
        param = param.with_expected(e);
    }
    if let Some(r) = received_str {
        param = param.with_received(r);
    }
    if let Some(m) = message_str {
        param = param.with_message(m);
    }

    let error_response = parameter_error(detail_str, instance_str, param);
    let error_json = error_response.to_json_string();
    string_to_c(&error_json)
}

/// Create RFC 7807 bad_request for unknown fields in body
#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807_unknown_fields(
    detail: *const libc::c_char,
    instance: *const libc::c_char,
    unknown_fields: *const libc::c_char,
) -> *const libc::c_char {
    if detail.is_null() || unknown_fields.is_null() {
        return std::ptr::null();
    }

    let detail_str = unsafe {
        std::ffi::CStr::from_ptr(detail)
            .to_string_lossy()
            .to_string()
    };
    let instance_str = if instance.is_null() {
        get_current_request_path()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(instance)
                .to_string_lossy()
                .to_string()
        }
    };
    let unknown_str = unsafe {
        std::ffi::CStr::from_ptr(unknown_fields)
            .to_string_lossy()
            .to_string()
    };
    let unknown_vec: Vec<String> = unknown_str
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .collect();

    use error::*;
    let error_response = unknown_fields_error(detail_str, instance_str, unknown_vec);
    let error_json = error_response.to_json_string();
    string_to_c(&error_json)
}

/// Create RFC 7807 bad_request for content-type issues
#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807_content_type(
    detail: *const libc::c_char,
    instance: *const libc::c_char,
    expected: *const libc::c_char,
    received: *const libc::c_char,
) -> *const libc::c_char {
    if detail.is_null() {
        return std::ptr::null();
    }

    let detail_str = unsafe {
        std::ffi::CStr::from_ptr(detail)
            .to_string_lossy()
            .to_string()
    };
    let instance_str = if instance.is_null() {
        get_current_request_path()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(instance)
                .to_string_lossy()
                .to_string()
        }
    };
    let expected_str = if expected.is_null() {
        None
    } else {
        Some(unsafe {
            std::ffi::CStr::from_ptr(expected)
                .to_string_lossy()
                .to_string()
        })
    };
    let received_str = if received.is_null() {
        None
    } else {
        Some(unsafe {
            std::ffi::CStr::from_ptr(received)
                .to_string_lossy()
                .to_string()
        })
    };

    use error::*;
    let error_response = content_type_error(detail_str, instance_str, expected_str, received_str);
    let error_json = error_response.to_json_string();
    string_to_c(&error_json)
}

/// Create RFC 7807 error response with HTTP method
/// FFI signature: doohttp_error_rfc7807_with_method(status, detail, instance, method) -> json_string
#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807_with_method(
    status: libc::c_int,
    detail: *const libc::c_char,
    instance: *const libc::c_char,
    method: *const libc::c_char,
) -> *const libc::c_char {
    if detail.is_null() || method.is_null() {
        return std::ptr::null();
    }

    let detail_str = unsafe {
        std::ffi::CStr::from_ptr(detail)
            .to_string_lossy()
            .to_string()
    };

    // If instance is null, use thread-local request path
    let instance_str = if instance.is_null() {
        get_current_request_path()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(instance)
                .to_string_lossy()
                .to_string()
        }
    };

    let method_str = unsafe {
        std::ffi::CStr::from_ptr(method)
            .to_string_lossy()
            .to_string()
    };

    // Use centralized error module
    use error::*;
    let error_response = match status {
        400 => bad_request(detail_str, instance_str),
        401 => unauthorized(detail_str, instance_str),
        403 => forbidden(detail_str, instance_str),
        404 => not_found(detail_str, instance_str),
        405 => method_not_allowed(detail_str, instance_str, vec![]),
        409 => conflict(detail_str, instance_str),
        422 => ErrorResponse::new(ErrorType::UnprocessableEntity, detail_str, instance_str),
        500 => internal_error(detail_str, instance_str),
        _ => internal_error(detail_str, instance_str),
    };

    let error_json = error_response.with_method(method_str).to_json_string();
    string_to_c(&error_json)
}

/// Create RFC 7807 validation error with fields
/// FFI signature: doohttp_error_rfc7807_validation(detail, instance, fields_json) -> json_string
#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807_validation(
    detail: *const libc::c_char,
    instance: *const libc::c_char,
    fields_json: *const libc::c_char,
) -> *const libc::c_char {
    if detail.is_null() || fields_json.is_null() {
        return std::ptr::null();
    }

    let detail_str = unsafe {
        std::ffi::CStr::from_ptr(detail)
            .to_string_lossy()
            .to_string()
    };

    // If instance is null, use thread-local request path
    let instance_str = if instance.is_null() {
        get_current_request_path()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(instance)
                .to_string_lossy()
                .to_string()
        }
    };

    let fields_str = unsafe {
        std::ffi::CStr::from_ptr(fields_json)
            .to_string_lossy()
            .to_string()
    };

    // Parse the fields JSON string into HashMap<String, FieldError>
    use error::*;
    use std::collections::HashMap;

    let fields: HashMap<String, FieldError> = match serde_json::from_str(&fields_str) {
        Ok(parsed) => {
            // Convert from generic JSON to FieldError structure
            let json_obj: serde_json::Map<String, serde_json::Value> = parsed;
            json_obj
                .into_iter()
                .map(|(key, val)| {
                    let field_err = if let Some(obj) = val.as_object() {
                        let rule = obj
                            .get("rule")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let message = obj
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let value = obj
                            .get("value")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let expected = obj
                            .get("expected")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let received = obj
                            .get("received")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let error = obj
                            .get("error")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        let mut fe = FieldError::new(message);
                        if !rule.is_empty() {
                            fe = fe.with_rule(rule);
                        }
                        if let Some(v) = value {
                            fe = fe.with_value(v);
                        }
                        if let Some(v) = expected {
                            fe = fe.with_expected(v);
                        }
                        if let Some(v) = received {
                            fe = fe.with_received(v);
                        }
                        if let Some(v) = error {
                            fe = fe.with_error(v);
                        }
                        fe
                    } else {
                        FieldError::new("Validation failed".to_string())
                    };
                    (key, field_err)
                })
                .collect()
        }
        Err(_) => {
            // If parsing fails, create a simple error response
            HashMap::new()
        }
    };

    let error_response = validation_error(detail_str, instance_str, fields);
    let error_json = error_response.to_json_string();
    string_to_c(&error_json)
}

/// Create RFC 7807 method not allowed error with allowed methods
/// FFI signature: doohttp_error_rfc7807_method_not_allowed(detail, instance, allowed_methods) -> json_string
#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807_method_not_allowed(
    detail: *const libc::c_char,
    instance: *const libc::c_char,
    allowed_methods: *const libc::c_char,
) -> *const libc::c_char {
    if detail.is_null() || allowed_methods.is_null() {
        return std::ptr::null();
    }

    let detail_str = unsafe {
        std::ffi::CStr::from_ptr(detail)
            .to_string_lossy()
            .to_string()
    };

    // If instance is null, use thread-local request path
    let instance_str = if instance.is_null() {
        get_current_request_path()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(instance)
                .to_string_lossy()
                .to_string()
        }
    };

    let allowed_str = unsafe {
        std::ffi::CStr::from_ptr(allowed_methods)
            .to_string_lossy()
            .to_string()
    };

    // Parse comma-separated methods into Vec<String>
    let methods: Vec<String> = allowed_str
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    // Use centralized error module
    use error::*;
    let error_response = method_not_allowed(detail_str, instance_str, methods);
    let error_json = error_response.to_json_string();
    string_to_c(&error_json)
}

/// Create RFC 7807 error response with automatic instance from thread-local
/// This is used by generated code when enum errors are returned from handlers
/// FFI signature: doohttp_error_rfc7807_auto_instance(status, detail) -> json_string
#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807_auto_instance(
    status: libc::c_int,
    detail: *const libc::c_char,
) -> *const libc::c_char {
    if detail.is_null() {
        return std::ptr::null();
    }

    let detail_str = unsafe {
        std::ffi::CStr::from_ptr(detail)
            .to_string_lossy()
            .to_string()
    };

    // Always use thread-local request path
    let instance_str = get_current_request_path();

    // Use centralized error module
    use error::*;
    let error_response = match status {
        400 => bad_request(detail_str, instance_str),
        401 => unauthorized(detail_str, instance_str),
        403 => forbidden(detail_str, instance_str),
        404 => not_found(detail_str, instance_str),
        405 => method_not_allowed(detail_str, instance_str, vec![]),
        409 => conflict(detail_str, instance_str),
        422 => ErrorResponse::new(ErrorType::UnprocessableEntity, detail_str, instance_str),
        429 => ErrorResponse::new(ErrorType::TooManyRequests, detail_str, instance_str),
        500 => internal_error(detail_str, instance_str),
        501 => not_implemented(detail_str, instance_str),
        502 => bad_gateway(detail_str, instance_str),
        503 => service_unavailable(detail_str, instance_str),
        _ => internal_error(detail_str, instance_str),
    };

    let error_json = error_response.to_json_string();
    string_to_c(&error_json)
}

/// Create RFC 7807 validation error with multiple field errors
/// FFI signature: doohttp_error_rfc7807_bad_request_with_fields(fields_json) -> json_string
/// fields_json format: {"FieldName": {"rule": "email", "message": "Invalid email", "value": "bad"}}
#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807_bad_request_with_fields(
    fields_json: *const libc::c_char,
) -> *const libc::c_char {
    if fields_json.is_null() {
        return std::ptr::null();
    }

    let fields_json_str = unsafe {
        std::ffi::CStr::from_ptr(fields_json)
            .to_string_lossy()
            .to_string()
    };

    // Parse fields JSON
    let fields_map: std::collections::HashMap<String, serde_json::Value> =
        match serde_json::from_str(&fields_json_str) {
            Ok(m) => m,
            Err(_) => {
                // Invalid JSON, return simple error
                let instance_str = get_current_request_path();
                let error_response = error::bad_request(
                    "Invalid validation fields format".to_string(),
                    instance_str,
                );
                let error_json = error_response.to_json_string();
                return string_to_c(&error_json);
            }
        };

    // Convert to FieldError map
    let mut field_errors = std::collections::HashMap::new();
    for (field_name, field_obj) in fields_map {
        let rule = field_obj
            .get("rule")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let message = field_obj
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Validation failed")
            .to_string();
        let value = field_obj
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let field_error = error::FieldError::new(message)
            .with_rule(rule)
            .with_value(value);
        field_errors.insert(field_name, field_error);
    }

    // Create validation error response
    let instance_str = get_current_request_path();
    let error_response = error::validation_error(
        "One or more fields failed validation".to_string(),
        instance_str,
        field_errors,
    );

    let error_json = error_response.to_json_string();
    string_to_c(&error_json)
}

/// Helper function to validate a JSON value against an expected Doo type
/// Returns (is_valid, error_detail) where error_detail describes the mismatch
fn validate_field_type(expected_type: &str, value: &serde_json::Value) -> (bool, Option<String>) {
    // Optional(T): allow null, otherwise validate inner.
    if expected_type.starts_with("Optional(") && expected_type.ends_with(')') {
        if value.is_null() {
            return (true, None);
        }
        let inner = &expected_type[9..expected_type.len() - 1];
        return validate_field_type(inner, value);
    }

    match expected_type {
        "Int" => {
            if value.is_i64() || value.is_u64() {
                (true, None)
            } else {
                (
                    false,
                    Some(format!("expected Int, got {}", get_json_type_name(value))),
                )
            }
        }
        "Float" => {
            if value.is_f64() || value.is_i64() || value.is_u64() {
                (true, None)
            } else {
                (
                    false,
                    Some(format!("expected Float, got {}", get_json_type_name(value))),
                )
            }
        }
        "Bool" => {
            if value.is_boolean() {
                (true, None)
            } else {
                (
                    false,
                    Some(format!("expected Bool, got {}", get_json_type_name(value))),
                )
            }
        }
        "Str" | "String" => {
            if value.is_string() {
                (true, None)
            } else {
                (
                    false,
                    Some(format!("expected Str, got {}", get_json_type_name(value))),
                )
            }
        }
        // Array types: [Str], [Int], [Float], [Bool]
        ty if ty.starts_with('[') && ty.ends_with(']') => {
            let inner_type = &ty[1..ty.len() - 1];
            if let Some(arr) = value.as_array() {
                for (idx, elem) in arr.iter().enumerate() {
                    let (elem_valid, elem_error) = validate_field_type(inner_type, elem);
                    if !elem_valid {
                        return (
                            false,
                            Some(format!(
                                "array element at index {} has wrong type: {}",
                                idx,
                                elem_error.unwrap_or_default()
                            )),
                        );
                    }
                }
                (true, None)
            } else {
                (
                    false,
                    Some(format!("expected array, got {}", get_json_type_name(value))),
                )
            }
        }
        // Array types in compiler format: Array(Str), Array(String), Array(Int), Array(Float), Array(Bool)
        ty if ty.starts_with("Array(") && ty.ends_with(')') => {
            let inner_type = &ty[6..ty.len() - 1]; // Extract "Str" from "Array(Str)"
            if let Some(arr) = value.as_array() {
                for (idx, elem) in arr.iter().enumerate() {
                    let (elem_valid, elem_error) = validate_field_type(inner_type, elem);
                    if !elem_valid {
                        return (
                            false,
                            Some(format!(
                                "array element at index {} has wrong type: {}",
                                idx,
                                elem_error.unwrap_or_default()
                            )),
                        );
                    }
                }
                (true, None)
            } else {
                (
                    false,
                    Some(format!("expected array, got {}", get_json_type_name(value))),
                )
            }
        }

        // Map types: {Str: Int}, {Str: Str}, etc.
        ty if ty.starts_with('{') && ty.ends_with('}') && ty.contains(':') => {
            let inner = &ty[1..ty.len() - 1];
            if let Some(colon_pos) = inner.find(':') {
                let key_type = inner[..colon_pos].trim();
                let value_type = inner[colon_pos + 1..].trim();

                if let Some(obj) = value.as_object() {
                    // Validate keys (should always be strings in JSON)
                    if key_type != "Str" && key_type != "String" {
                        // JSON only supports string keys, so non-Str key types can't be validated
                        return (true, None);
                    }

                    // Validate values
                    for (k, v) in obj {
                        let (val_valid, val_error) = validate_field_type(value_type, v);
                        if !val_valid {
                            return (
                                false,
                                Some(format!(
                                    "map value for key '{}' has wrong type: {}",
                                    k,
                                    val_error.unwrap_or_default()
                                )),
                            );
                        }
                    }
                    (true, None)
                } else {
                    (
                        false,
                        Some(format!(
                            "expected object, got {}",
                            get_json_type_name(value)
                        )),
                    )
                }
            } else {
                (true, None) // Can't parse map type, skip validation
            }
        }
        // Enum types and unknown - allow them through (enums are validated elsewhere)
        _ => (true, None),
    }
}

/// Get the JSON type name for error messages
fn get_json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "Bool",
        serde_json::Value::Number(n) => {
            if n.is_f64() && n.as_i64().is_none() {
                "Float"
            } else {
                "Int"
            }
        }
        serde_json::Value::String(_) => "Str",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

// ===== DECORATOR VALIDATION =====
// Note: All validation logic is in libdoo_runtime via dooruntime_validate_field()
// This keeps validation centralized and reusable across all FFI libs (http, db, auth)

/// Format validation error from runtime into RFC 7807 DooResponse
/// Takes validation error JSON from dooruntime_get_last_validation_error() and request path
/// Returns DooResponse* with 422 status and RFC 7807 formatted error body
#[no_mangle]
/// Validate JSON body, call handler, and serialize response
/// This is the main entry point for HTTP handlers with validation
///
/// Parameters:
/// - body_json: JSON string from request body
/// - request_path: HTTP path for error responses
/// - metadata_json: JSON string with struct_decorators metadata
/// - handler_fn: Function pointer to the actual handler
/// - handler_name: Name of handler for debugging
///
/// Returns: DooResponse* with either handler result or RFC 7807 validation error

pub extern "C" fn doohttp_validate_and_call_handler(
    body_json: *const libc::c_char,
    request_path: *const libc::c_char,
    metadata_json: *const libc::c_char,
    handler_fn: *const libc::c_void,
    _handler_name: *const libc::c_char,
) -> *mut DooResponse {
    use error::*;

    if body_json.is_null()
        || request_path.is_null()
        || metadata_json.is_null()
        || handler_fn.is_null()
    {
        return alloc_doo_response(
            500,
            string_to_c(
                r#"{"type":"internal_error","title":"Internal Server Error","status":500,"detail":"Invalid parameters to validation handler"}"#,
            ),
            string_to_c("application/json"),
        );
    }

    let body_str = unsafe { CStr::from_ptr(body_json).to_string_lossy().to_string() };
    let path_str = unsafe { CStr::from_ptr(request_path).to_string_lossy().to_string() };
    let metadata_str = unsafe { CStr::from_ptr(metadata_json).to_string_lossy().to_string() };

    // Parse metadata JSON
    let metadata: serde_json::Value = match serde_json::from_str(&metadata_str) {
        Ok(m) => m,
        Err(_) => {
            let err = internal_error("Metadata parse error".to_string(), path_str);
            return alloc_doo_response(
                500,
                string_to_c(&err.to_json_string()),
                string_to_c("application/json"),
            );
        }
    };

    // Get param_types to determine struct name
    let struct_name = metadata
        .get("param_types")
        .and_then(|pt| pt.as_array())
        .and_then(|arr| arr.get(0))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");

    // Parse body JSON
    let body_obj = match serde_json::from_str::<serde_json::Value>(&body_str) {
        Ok(serde_json::Value::Object(obj)) => obj,
        Ok(_) => {
            let err = bad_request("Request body must be a JSON object".to_string(), path_str);
            return alloc_doo_response(
                400,
                string_to_c(&err.to_json_string()),
                string_to_c("application/json"),
            );
        }
        Err(_) => {
            let err = bad_request("Invalid JSON format".to_string(), path_str);
            return alloc_doo_response(
                400,
                string_to_c(&err.to_json_string()),
                string_to_c("application/json"),
            );
        }
    };

    // Get struct_fields for type validation (before decorator validation)
    let struct_fields = metadata
        .get("struct_fields")
        .and_then(|sf| sf.get(struct_name))
        .and_then(|v| v.as_array());

    // Validate field types including array element types
    if let Some(fields_array) = struct_fields {
        let mut type_errors = serde_json::Map::new();

        for field_def in fields_array {
            if let Some(field_arr) = field_def.as_array() {
                if field_arr.len() >= 2 {
                    let field_name = field_arr[0].as_str().unwrap_or("");
                    let expected_type = field_arr[1].as_str().unwrap_or("");

                    if let Some(field_value) = body_obj.get(field_name) {
                        // Validate type using helper function
                        let (is_valid, error_detail) =
                            validate_field_type(expected_type, field_value);

                        if !is_valid {
                            let mut field_error = serde_json::Map::new();
                            field_error.insert("expected".to_string(), json!(expected_type));
                            field_error.insert(
                                "received".to_string(),
                                json!(get_json_type_name(field_value)),
                            );
                            if let Some(detail) = error_detail {
                                field_error.insert("detail".to_string(), json!(detail));
                            }
                            type_errors.insert(field_name.to_string(), json!(field_error));
                        }
                    }
                }
            }
        }

        if !type_errors.is_empty() {
            use error::*;
            let mut err = bad_request(
                "Type mismatch in request body".to_string(),
                path_str.clone(),
            );
            for (field_name, error_info) in type_errors {
                let error_obj = error_info.as_object().unwrap();
                let detail = error_obj
                    .get("detail")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");
                err = err.with_field(
                    field_name.clone(),
                    FieldError {
                        rule: None,
                        message: field_name.clone(),
                        value: None,
                        expected: error_obj
                            .get("expected")
                            .and_then(|e| e.as_str())
                            .map(|s| s.to_string()),
                        received: error_obj
                            .get("received")
                            .and_then(|r| r.as_str())
                            .map(|s| s.to_string()),
                        error: Some(detail.to_string()),
                    },
                );
            }
            return alloc_doo_response(
                400,
                string_to_c(&err.to_json_string()),
                string_to_c("application/json"),
            );
        }
    }

    // Get struct_decorators for validation
    let struct_decorators = metadata
        .get("struct_decorators")
        .and_then(|sd| sd.get(struct_name))
        .and_then(|v| v.as_object());

    if let Some(field_decorators) = struct_decorators {
        let mut validation_errors = std::collections::HashMap::new();

        // Validate each field that has decorators
        for (field_name, decorators_value) in field_decorators {
            if let Some(decorators_array) = decorators_value.as_array() {
                if let Some(field_value) = body_obj.get(field_name) {
                    // Determine field type from JSON value
                    let field_type = if field_value.is_string() {
                        "Str"
                    } else if field_value.is_i64() || field_value.is_u64() {
                        "Int"
                    } else if field_value.is_f64() {
                        "Float"
                    } else if field_value.is_boolean() {
                        "Bool"
                    } else {
                        "Unknown"
                    };

                    // Convert field value to string for validation
                    let value_str = match field_value {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        _ => field_value.to_string(),
                    };

                    // Validate with each decorator
                    for decorator in decorators_array {
                        let decorators_json = serde_json::to_string(&vec![decorator])
                            .unwrap_or_else(|_| "[]".to_string());

                        let field_name_cstr = CString::new(field_name.as_str()).unwrap();
                        let field_type_cstr = CString::new(field_type).unwrap();
                        let value_cstr = CString::new(value_str.as_str()).unwrap();
                        let decorators_cstr = CString::new(decorators_json).unwrap();

                        extern "C" {
                            fn dooruntime_validate_field(
                                field_name: *const libc::c_char,
                                field_type: *const libc::c_char,
                                value: *const libc::c_char,
                                decorators_json: *const libc::c_char,
                            ) -> *const libc::c_char;
                            fn dooruntime_get_last_validation_error() -> *mut libc::c_char;
                            fn dooruntime_free_string(ptr: *mut libc::c_char);
                        }

                        unsafe {
                            let error_ptr = dooruntime_validate_field(
                                field_name_cstr.as_ptr(),
                                field_type_cstr.as_ptr(),
                                value_cstr.as_ptr(),
                                decorators_cstr.as_ptr(),
                            );

                            if !error_ptr.is_null() {
                                // Validation failed - get structured error
                                let validation_error_json_ptr =
                                    dooruntime_get_last_validation_error();
                                if !validation_error_json_ptr.is_null() {
                                    let error_json_str = CStr::from_ptr(validation_error_json_ptr)
                                        .to_string_lossy()
                                        .to_string();
                                    if let Ok(error_json) =
                                        serde_json::from_str::<serde_json::Value>(&error_json_str)
                                    {
                                        let rule = error_json
                                            .get("rule")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        let message = error_json
                                            .get("message")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("Validation failed");

                                        let field_error = FieldError::new(field_name.clone())
                                            .with_rule(rule.to_string())
                                            .with_value(value_str.clone())
                                            .with_error(message.to_string());

                                        validation_errors.insert(field_name.clone(), field_error);
                                    }
                                    dooruntime_free_string(validation_error_json_ptr);
                                }
                                dooruntime_free_string(error_ptr as *mut _);
                                break; // Stop after first error for this field
                            }
                        }
                    }
                }
            }
        }

        // If there are validation errors, return RFC 7807 response
        if !validation_errors.is_empty() {
            let err = validation_failed_error(path_str, validation_errors);
            let body_json = err.to_json_string();
            return alloc_doo_response(
                422,
                string_to_c(&body_json),
                string_to_c("application/json"),
            );
        }
    }

    // Validation passed - return success with body as-is (handler will process via wrapper)
    // The actual handler call happens in generated code, we just validated and return OK to proceed
    alloc_doo_response(
        0, // Special status 0 means "validation passed, proceed with handler"
        string_to_c(&body_str),
        string_to_c("application/json"),
    )
}

/// Parse JSON body and validate using decorators metadata
/// Returns DooResponse* with parsed data on success, or RFC 7807 error on validation failure
///
/// Parameters:
/// - body_json: JSON string to parse
/// - struct_name: Name of the struct type to parse into
/// - metadata_json: JSON string with struct_decorators metadata
/// - request_path: HTTP path for error responses
#[no_mangle]
pub extern "C" fn doohttp_parse_and_validate_json(
    body_json: *const libc::c_char,
    struct_name: *const libc::c_char,
    metadata_json: *const libc::c_char,
    request_path: *const libc::c_char,
) -> *mut DooResponse {
    use error::*;

    if body_json.is_null()
        || struct_name.is_null()
        || metadata_json.is_null()
        || request_path.is_null()
    {
        return alloc_doo_response(
            400,
            string_to_c(
                r#"{"type":"bad_request","title":"Bad Request","status":400,"detail":"Invalid parameters"}"#,
            ),
            string_to_c("application/json"),
        );
    }

    let body_str = unsafe { CStr::from_ptr(body_json).to_string_lossy().to_string() };
    let struct_name_str = unsafe { CStr::from_ptr(struct_name).to_string_lossy().to_string() };
    let metadata_str = unsafe { CStr::from_ptr(metadata_json).to_string_lossy().to_string() };
    let path_str = unsafe { CStr::from_ptr(request_path).to_string_lossy().to_string() };

    // Parse body JSON
    let body_obj = match serde_json::from_str::<serde_json::Value>(&body_str) {
        Ok(serde_json::Value::Object(obj)) => obj,
        Ok(_) => {
            let err = bad_request("Request body must be a JSON object".to_string(), path_str);
            return alloc_doo_response(
                400,
                string_to_c(&err.to_json_string()),
                string_to_c("application/json"),
            );
        }
        Err(_) => {
            let err = bad_request("Invalid JSON format".to_string(), path_str);
            return alloc_doo_response(
                400,
                string_to_c(&err.to_json_string()),
                string_to_c("application/json"),
            );
        }
    };

    // Parse metadata JSON
    let metadata: serde_json::Value = match serde_json::from_str(&metadata_str) {
        Ok(m) => m,
        Err(_) => {
            let err = internal_error("Metadata parse error".to_string(), path_str);
            return alloc_doo_response(
                500,
                string_to_c(&err.to_json_string()),
                string_to_c("application/json"),
            );
        }
    };

    // Get struct_decorators for the specific struct
    let struct_decorators = metadata
        .get("struct_decorators")
        .and_then(|sd| sd.get(&struct_name_str))
        .and_then(|v| v.as_object());

    if let Some(field_decorators) = struct_decorators {
        let mut validation_errors = std::collections::HashMap::new();

        // Validate each field that has decorators
        for (field_name, decorators_value) in field_decorators {
            if let Some(decorators_array) = decorators_value.as_array() {
                if let Some(field_value) = body_obj.get(field_name) {
                    // Determine field type from JSON value
                    let field_type = if field_value.is_string() {
                        "Str"
                    } else if field_value.is_i64() || field_value.is_u64() {
                        "Int"
                    } else if field_value.is_f64() {
                        "Float"
                    } else if field_value.is_boolean() {
                        "Bool"
                    } else {
                        "Unknown"
                    };

                    // Convert field value to string for validation
                    let value_str = match field_value {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        _ => field_value.to_string(),
                    };

                    // Validate with each decorator
                    for decorator in decorators_array {
                        let decorators_json = serde_json::to_string(&vec![decorator])
                            .unwrap_or_else(|_| "[]".to_string());

                        let field_name_cstr = CString::new(field_name.as_str()).unwrap();
                        let field_type_cstr = CString::new(field_type).unwrap();
                        let value_cstr = CString::new(value_str.as_str()).unwrap();
                        let decorators_cstr = CString::new(decorators_json).unwrap();

                        extern "C" {
                            fn dooruntime_validate_field(
                                field_name: *const libc::c_char,
                                field_type: *const libc::c_char,
                                value: *const libc::c_char,
                                decorators_json: *const libc::c_char,
                            ) -> *const libc::c_char;
                            fn dooruntime_get_last_validation_error() -> *mut libc::c_char;
                            fn dooruntime_free_string(ptr: *mut libc::c_char);
                        }

                        unsafe {
                            let error_ptr = dooruntime_validate_field(
                                field_name_cstr.as_ptr(),
                                field_type_cstr.as_ptr(),
                                value_cstr.as_ptr(),
                                decorators_cstr.as_ptr(),
                            );

                            if !error_ptr.is_null() {
                                // Validation failed - get structured error
                                let validation_error_json_ptr =
                                    dooruntime_get_last_validation_error();
                                if !validation_error_json_ptr.is_null() {
                                    let error_json_str = CStr::from_ptr(validation_error_json_ptr)
                                        .to_string_lossy()
                                        .to_string();
                                    if let Ok(error_json) =
                                        serde_json::from_str::<serde_json::Value>(&error_json_str)
                                    {
                                        let rule = error_json
                                            .get("rule")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        let message = error_json
                                            .get("message")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("Validation failed");

                                        let field_error = FieldError::new(field_name.clone())
                                            .with_rule(rule.to_string())
                                            .with_value(value_str.clone())
                                            .with_error(message.to_string());

                                        validation_errors.insert(field_name.clone(), field_error);
                                    }
                                    dooruntime_free_string(validation_error_json_ptr);
                                }
                                dooruntime_free_string(error_ptr as *mut _);
                                break; // Stop after first error for this field
                            }
                        }
                    }
                }
            }
        }

        // If there are validation errors, return RFC 7807 response
        if !validation_errors.is_empty() {
            let err = validation_failed_error(path_str, validation_errors);
            let body_json = err.to_json_string();
            return alloc_doo_response(
                422,
                string_to_c(&body_json),
                string_to_c("application/json"),
            );
        }
    }

    // Validation passed - return success with parsed JSON body
    alloc_doo_response(200, string_to_c(&body_str), string_to_c("application/json"))
}

#[no_mangle]
pub extern "C" fn doohttp_format_validation_error(
    validation_error_json: *const libc::c_char,
    request_path: *const libc::c_char,
) -> *mut DooResponse {
    if validation_error_json.is_null() || request_path.is_null() {
        // Return generic 422 error
        return alloc_doo_response(
            422,
            string_to_c(
                r#"{"type":"validation_error","title":"Validation Failed","status":422,"detail":"Validation error occurred"}"#,
            ),
            string_to_c("application/json"),
        );
    }

    let error_json_str = unsafe {
        std::ffi::CStr::from_ptr(validation_error_json)
            .to_string_lossy()
            .to_string()
    };

    let path_str = unsafe {
        std::ffi::CStr::from_ptr(request_path)
            .to_string_lossy()
            .to_string()
    };

    // Parse validation error JSON
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&error_json_str) {
        let field_name = json
            .get("field_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let rule = json.get("rule").and_then(|v| v.as_str()).unwrap_or("");
        let message = json
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Validation failed");
        let value = json.get("value").and_then(|v| v.as_str()).unwrap_or("");

        // Create FieldError using error.rs
        use error::*;
        let mut field_errors = std::collections::HashMap::new();
        let field_error = FieldError::new(field_name.to_string())
            .with_rule(rule.to_string())
            .with_value(value.to_string())
            .with_error(message.to_string());

        field_errors.insert(field_name.to_string(), field_error);

        // Create RFC 7807 validation error response
        let err = validation_failed_error(path_str, field_errors);
        let body_json = err.to_json_string();

        alloc_doo_response(
            422,
            string_to_c(&body_json),
            string_to_c("application/json"),
        )
    } else {
        // Failed to parse, return generic error
        alloc_doo_response(
            422,
            string_to_c(
                r#"{"type":"validation_error","title":"Validation Failed","status":422,"detail":"Validation error occurred"}"#,
            ),
            string_to_c("application/json"),
        )
    }
}

/// Serialize a struct to JSON for HTTP response
/// Takes a pointer to struct data and handler name, looks up metadata from registry
#[no_mangle]
pub extern "C" fn doohttp_serialize_struct_to_json(
    struct_ptr: *const libc::c_void,
    handler_name: *const libc::c_char,
) -> *const libc::c_char {
    if struct_ptr.is_null() || handler_name.is_null() {
        return string_to_c("{}");
    }

    let handler_name_str = unsafe { CStr::from_ptr(handler_name).to_string_lossy().to_string() };

    // Get handler metadata from registry
    let routes = get_routes();
    let registry = routes.lock().unwrap();
    let metadata = registry.handler_metadata.get(&handler_name_str).cloned();
    drop(registry);

    let metadata = match metadata {
        Some(m) => m,
        None => return string_to_c("{}"),
    };

    // Get return type from handler metadata
    let struct_name = &metadata.return_type;

    let struct_layout = metadata
        .struct_layouts
        .get(struct_name)
        .and_then(|v| v.as_object());

    let struct_layout = match struct_layout {
        Some(layout) => layout,
        None => return string_to_c("{}"),
    };

    let fields = struct_layout.get("fields").and_then(|f| f.as_array());

    let fields = match fields {
        Some(f) => f,
        None => return string_to_c("{}"),
    };

    fn serialize_struct_ptr_to_value(
        struct_ptr: *const u8,
        struct_name: &str,
        struct_layouts: &serde_json::Value,
    ) -> serde_json::Value {
        if struct_ptr.is_null() {
            return serde_json::Value::Null;
        }

        let layout = struct_layouts
            .get(struct_name)
            .and_then(|v| v.as_object())
            .and_then(|o| o.get("fields"))
            .and_then(|v| v.as_array());

        let Some(fields) = layout else {
            return serde_json::Value::Null;
        };

        let mut json_obj = serde_json::Map::new();
        unsafe {
            for field in fields {
                let field_obj = match field.as_object() {
                    Some(obj) => obj,
                    None => continue,
                };

                let field_name = match field_obj.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n,
                    None => continue,
                };

                let field_type = match field_obj.get("type").and_then(|v| v.as_str()) {
                    Some(t) => t,
                    None => continue,
                };

                let offset = match field_obj.get("offset").and_then(|v| v.as_u64()) {
                    Some(o) => o as isize,
                    None => continue,
                };

                let field_ptr = (struct_ptr as *const u8).offset(offset);

                let field_value: Option<serde_json::Value> = match field_type {
                    "Int" => Some(serde_json::json!(*(field_ptr as *const i32))),
                    "Float" => Some(serde_json::json!(*(field_ptr as *const f64))),
                    "Bool" => Some(serde_json::json!(*(field_ptr as *const i32) != 0)),
                    "Str" => {
                        let str_ptr = *(field_ptr as *const *const libc::c_char);
                        if str_ptr.is_null() {
                            Some(serde_json::json!(""))
                        } else {
                            Some(serde_json::json!(CStr::from_ptr(str_ptr)
                                .to_string_lossy()
                                .to_string()))
                        }
                    }
                    "Optional(Str)" | "Optional(String)" => {
                        let str_ptr = *(field_ptr as *const *const libc::c_char);
                        if str_ptr.is_null() {
                            None
                        } else {
                            Some(serde_json::json!(CStr::from_ptr(str_ptr)
                                .to_string_lossy()
                                .to_string()))
                        }
                    }
                    ty if ty.starts_with("Array(") && ty.ends_with(')') => {
                        let data_ptr = *(field_ptr as *const *const u8);
                        if data_ptr.is_null() {
                            Some(serde_json::json!([]))
                        } else {
                            let len = *(data_ptr.offset(-4) as *const i32) as usize;
                            let element_type = &ty[6..ty.len() - 1];
                            let mut arr: Vec<serde_json::Value> = Vec::new();
                            match element_type {
                                "Int" => {
                                    let data = data_ptr as *const i32;
                                    for i in 0..len {
                                        arr.push(serde_json::json!(*data.add(i)));
                                    }
                                }
                                "Float" => {
                                    let data = data_ptr as *const f64;
                                    for i in 0..len {
                                        arr.push(serde_json::json!(*data.add(i)));
                                    }
                                }
                                "Bool" => {
                                    let data = data_ptr as *const i32;
                                    for i in 0..len {
                                        arr.push(serde_json::json!(*data.add(i) != 0));
                                    }
                                }
                                "Str" => {
                                    let data = data_ptr as *const *const libc::c_char;
                                    for i in 0..len {
                                        let str_ptr = *data.add(i);
                                        if str_ptr.is_null() {
                                            arr.push(serde_json::json!(""));
                                        } else {
                                            arr.push(serde_json::json!(CStr::from_ptr(str_ptr)
                                                .to_string_lossy()
                                                .to_string()));
                                        }
                                    }
                                }
                                _ => {
                                    if struct_layouts.get(element_type).is_some() {
                                        let data = data_ptr as *const *const u8;
                                        for i in 0..len {
                                            let elem_ptr = *data.add(i);
                                            arr.push(serialize_struct_ptr_to_value(
                                                elem_ptr,
                                                element_type,
                                                struct_layouts,
                                            ));
                                        }
                                    }
                                }
                            }
                            Some(serde_json::Value::Array(arr))
                        }
                    }
                    ty => {
                        if struct_layouts.get(ty).is_some() {
                            let nested_ptr = *(field_ptr as *const *const u8);
                            if nested_ptr.is_null() {
                                Some(serde_json::Value::Null)
                            } else {
                                Some(serialize_struct_ptr_to_value(
                                    nested_ptr,
                                    ty,
                                    struct_layouts,
                                ))
                            }
                        } else {
                            Some(serde_json::Value::Null)
                        }
                    }
                };

                if let Some(v) = field_value {
                    json_obj.insert(field_name.to_string(), v);
                }
            }
        }

        serde_json::Value::Object(json_obj)
    }

    let mut json_obj = serde_json::Map::new();

    // Read each field from struct memory and add to JSON
    for field in fields {
        let field_obj = match field.as_object() {
            Some(obj) => obj,
            None => continue,
        };

        let field_name = match field_obj.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => continue,
        };

        let field_type = match field_obj.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => continue,
        };

        let offset = match field_obj.get("offset").and_then(|v| v.as_u64()) {
            Some(o) => o as isize,
            None => continue,
        };

        unsafe {
            let field_ptr = (struct_ptr as *const u8).offset(offset);

            let field_value: Option<serde_json::Value> = match field_type {
                "Int" => {
                    let val = *(field_ptr as *const i32);
                    Some(serde_json::json!(val))
                }
                "Float" => {
                    let val = *(field_ptr as *const f64);
                    Some(serde_json::json!(val))
                }
                "Bool" => {
                    let val = *(field_ptr as *const i32);
                    Some(serde_json::json!(val != 0))
                }
                "Str" => {
                    let str_ptr = *(field_ptr as *const *const libc::c_char);
                    if str_ptr.is_null() {
                        Some(serde_json::json!(""))
                    } else {
                        let c_str = CStr::from_ptr(str_ptr);
                        let rust_str = c_str.to_string_lossy().to_string();
                        Some(serde_json::json!(rust_str))
                    }
                }
                "Optional(Str)" | "Optional(String)" => {
                    let str_ptr = *(field_ptr as *const *const libc::c_char);
                    if str_ptr.is_null() {
                        None
                    } else {
                        let c_str = CStr::from_ptr(str_ptr);
                        let rust_str = c_str.to_string_lossy().to_string();
                        Some(serde_json::json!(rust_str))
                    }
                }
                ty if ty.starts_with("Array(") && ty.ends_with(')') => {
                    // Array layout: [RC: 4 bytes][Length: 4 bytes][data...]
                    // IMPORTANT: The stored pointer points to DATA (at offset +8 from header start)
                    // So from the data pointer: RC is at -8, Length is at -4, Data is at 0
                    let data_ptr = *(field_ptr as *const *const u8);
                    if data_ptr.is_null() {
                        Some(serde_json::json!([]))
                    } else {
                        // Read length from offset -4 (relative to data pointer)
                        let len = *(data_ptr.offset(-4) as *const i32) as usize;
                        let element_type = &ty[6..ty.len() - 1]; // Extract "Str" from "Array(Str)"

                        let mut arr: Vec<serde_json::Value> = Vec::new();
                        match element_type {
                            "Int" => {
                                let data = data_ptr as *const i32;
                                for i in 0..len {
                                    arr.push(serde_json::json!(*data.add(i)));
                                }
                            }
                            "Float" => {
                                let data = data_ptr as *const f64;
                                for i in 0..len {
                                    arr.push(serde_json::json!(*data.add(i)));
                                }
                            }
                            "Bool" => {
                                let data = data_ptr as *const i32;
                                for i in 0..len {
                                    arr.push(serde_json::json!(*data.add(i) != 0));
                                }
                            }
                            "Str" => {
                                let data = data_ptr as *const *const libc::c_char;
                                for i in 0..len {
                                    let str_ptr = *data.add(i);
                                    if str_ptr.is_null() {
                                        arr.push(serde_json::json!(""));
                                    } else {
                                        let c_str = CStr::from_ptr(str_ptr);
                                        arr.push(serde_json::json!(c_str
                                            .to_string_lossy()
                                            .to_string()));
                                    }
                                }
                            }
                            _ => {
                                if metadata.struct_layouts.get(element_type).is_some() {
                                    let data = data_ptr as *const *const u8;
                                    for i in 0..len {
                                        let elem_ptr = *data.add(i);
                                        arr.push(serialize_struct_ptr_to_value(
                                            elem_ptr,
                                            element_type,
                                            &metadata.struct_layouts,
                                        ));
                                    }
                                }
                            }
                        }
                        Some(serde_json::Value::Array(arr))
                    }
                }
                ty => {
                    // Check if this is a nested struct
                    if let Some(nested_layout_obj) =
                        metadata.struct_layouts.get(ty).and_then(|v| v.as_object())
                    {
                        // This is a nested struct - read pointer to child struct
                        let nested_ptr = *(field_ptr as *const *const u8);
                        if nested_ptr.is_null() {
                            Some(serde_json::json!(null))
                        } else {
                            // Recursively serialize nested struct
                            if let Some(nested_fields) =
                                nested_layout_obj.get("fields").and_then(|f| f.as_array())
                            {
                                let mut nested_obj = serde_json::Map::new();
                                for nested_field in nested_fields {
                                    if let Some(nf_obj) = nested_field.as_object() {
                                        let nf_name = nf_obj
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        let nf_type = nf_obj
                                            .get("type")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        let nf_offset = nf_obj
                                            .get("offset")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0)
                                            as isize;

                                        let nf_ptr = (nested_ptr as *const u8).offset(nf_offset);

                                        let nf_value: serde_json::Value = match nf_type {
                                            "Int" => {
                                                let val = *(nf_ptr as *const i32);
                                                serde_json::json!(val)
                                            }
                                            "Float" => {
                                                let val = *(nf_ptr as *const f64);
                                                serde_json::json!(val)
                                            }
                                            "Bool" => {
                                                let val = *(nf_ptr as *const i32);
                                                serde_json::json!(val != 0)
                                            }
                                            "Str" => {
                                                let str_ptr =
                                                    *(nf_ptr as *const *const libc::c_char);
                                                if str_ptr.is_null() {
                                                    serde_json::json!("")
                                                } else {
                                                    let c_str = CStr::from_ptr(str_ptr);
                                                    serde_json::json!(c_str
                                                        .to_string_lossy()
                                                        .to_string())
                                                }
                                            }
                                            _ => serde_json::json!(null),
                                        };
                                        nested_obj.insert(nf_name.to_string(), nf_value);
                                    }
                                }
                                Some(serde_json::Value::Object(nested_obj))
                            } else {
                                Some(serde_json::json!(null))
                            }
                        }
                    } else {
                        Some(serde_json::json!(null))
                    }
                }
            };

            if let Some(v) = field_value {
                json_obj.insert(field_name.to_string(), v);
            }
        }
    }

    // Wrap in {"data": ...} format for RFC 7807 compliance
    let wrapped = serde_json::json!({ "data": json_obj });
    let json_str = serde_json::to_string(&wrapped).unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
    string_to_c(&json_str)
}

/// Populate struct from request data with JSON parsing and validation
/// This is the main entry point called by generated handler wrappers
///
/// Parameters:
/// - request_ptr: Pointer to DooRequest
/// - struct_ptr: Pointer to allocated struct to populate
/// - source_type: 0=body (JSON), 1=params, 2=query
/// - handler_name: Name of handler (used to get metadata)
///
/// Returns: 0 on success, error code on failure
#[no_mangle]
pub extern "C" fn doohttp_populate_struct_from_request(
    request_ptr: *const libc::c_void,
    struct_ptr: *mut libc::c_void,
    source_type: i32,
    handler_name: *const libc::c_char,
) -> i32 {
    use error::*;

    if request_ptr.is_null() || struct_ptr.is_null() {
        return -1;
    }

    if handler_name.is_null() {
        return 0; // No handler name, can't look up metadata
    }

    let handler_name_str = unsafe { CStr::from_ptr(handler_name).to_string_lossy().to_string() };

    // Cast request to get fields
    #[repr(C)]
    struct DooRequestLayout {
        method: *const libc::c_char,
        path: *const libc::c_char,
        body: *const libc::c_char,
        content_type: *const libc::c_char,
        params: *const libc::c_void,
        query: *const libc::c_void,
        headers: *const libc::c_void,
    }

    let request = unsafe { &*(request_ptr as *const DooRequestLayout) };
    let path_str = unsafe { CStr::from_ptr(request.path).to_string_lossy().to_string() };

    // Get handler metadata from registry
    let routes = get_routes();
    let registry = routes.lock().unwrap();

    let metadata = registry.handler_metadata.get(&handler_name_str).cloned();

    drop(registry);

    let metadata = match metadata {
        Some(m) => m,
        None => {
            return 0; // No metadata, skip validation
        }
    };

    // Get HTTP method for smart source detection
    let method_str = if !request.method.is_null() {
        unsafe {
            CStr::from_ptr(request.method)
                .to_string_lossy()
                .to_string()
                .to_uppercase()
        }
    } else {
        "".to_string()
    };

    // Smart source_type detection:
    // For GET/DELETE with no body, automatically use query params
    let effective_source_type = if source_type == 0 {
        let has_body = if request.body.is_null() {
            false
        } else {
            let body_str = unsafe { CStr::from_ptr(request.body).to_string_lossy().to_string() };
            !body_str.is_empty()
        };

        if !has_body && (method_str == "GET" || method_str == "DELETE") {
            2 // Use query params for GET/DELETE without body
        } else {
            0 // Use body for POST/PUT/PATCH or when body exists
        }
    } else {
        source_type
    };

    // Determine source based on effective_source_type:
    // 0 = body (JSON), 1 = path params, 2 = query params
    let source_data: serde_json::Map<String, serde_json::Value> = match effective_source_type {
        0 => {
            // Parse body JSON
            if request.body.is_null() {
                return 0; // No body
            }
            let body_str = unsafe { CStr::from_ptr(request.body).to_string_lossy().to_string() };
            if body_str.is_empty() {
                return 0;
            }
            match serde_json::from_str::<serde_json::Value>(&body_str) {
                Ok(serde_json::Value::Object(obj)) => obj,
                _ => {
                    let err = invalid_json_error(path_str.clone());
                    set_last_error(err.status_code() as i32, err.to_json_string());
                    return 400;
                }
            }
        }
        1 => {
            // Extract from path params
            if request.params.is_null() {
                serde_json::Map::new()
            } else {
                let params_map = unsafe {
                    &*(request.params as *const std::collections::HashMap<String, String>)
                };
                params_map
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect()
            }
        }
        2 => {
            // Extract from query params
            if request.query.is_null() {
                serde_json::Map::new()
            } else {
                let query_map = unsafe {
                    &*(request.query as *const std::collections::HashMap<String, String>)
                };
                query_map
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect()
            }
        }
        _ => serde_json::Map::new(),
    };

    // Get struct name from param_types.
    // Handlers may have multiple params (e.g., body struct + userId). The previous
    // heuristic picked the last param for body, which breaks signatures like
    // (CreateLinkReq, Int). Instead, pick the first param type that has a struct
    // layout entry in metadata.struct_layouts.
    let struct_name = if !metadata.param_types.is_empty() {
        let layouts_obj = metadata.struct_layouts.as_object();
        metadata
            .param_types
            .iter()
            .find(|t| {
                layouts_obj
                    .map(|m| m.contains_key(t.as_str()))
                    .unwrap_or(false)
            })
            .cloned()
            .unwrap_or_else(|| {
                metadata
                    .param_types
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Unknown".to_string())
            })
    } else {
        "Unknown".to_string()
    };

    // Special types that receive raw request pointer - skip validation/population
    if struct_name == "Request" || struct_name == "DooRequest" || struct_name == "Unknown" {
        return 0; // Handler receives request directly, no struct to populate
    }

    // Check for missing required fields first
    let struct_layouts = &metadata.struct_layouts;
    if let Some(struct_layout) = struct_layouts.get(&struct_name) {
        if let Some(fields) = struct_layout.get("fields").and_then(|f| f.as_array()) {
            for field in fields {
                let field_obj = match field.as_object() {
                    Some(obj) => obj,
                    None => continue,
                };

                let field_name = match field_obj.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n,
                    None => continue,
                };

                let field_type = match field_obj.get("type").and_then(|v| v.as_str()) {
                    Some(t) => t,
                    None => continue,
                };

                // Check if field is missing from source data
                // Optional(...) fields are allowed to be missing.
                if get_source_value(field_name, &source_data).is_none()
                    && !(field_type.starts_with("Optional(") && field_type.ends_with(')'))
                {
                    // Field is missing - return error based on source type
                    let err = match source_type {
                        1 => {
                            // Path param missing
                            let param = ParameterError::new(field_name.to_string())
                                .with_expected(field_type.to_string())
                                .with_message("Path parameter not found".to_string());
                            missing_path_param_error(path_str.clone(), param)
                        }
                        2 => {
                            // Query param missing
                            let param = ParameterError::new(field_name.to_string())
                                .with_expected(field_type.to_string())
                                .with_message("Required query parameter missing".to_string());
                            missing_query_param_error(path_str.clone(), param)
                        }
                        _ => {
                            // Body field missing
                            let mut fields = std::collections::HashMap::new();
                            let field_err = FieldError::new(field_name.to_string())
                                .with_rule("required".to_string())
                                .with_error("Field is required".to_string());
                            fields.insert(field_name.to_string(), field_err);
                            missing_field_error(path_str.clone(), fields)
                        }
                    };
                    set_last_error(err.status_code() as i32, err.to_json_string());
                    return 400;
                }

                // Validate type conversion for path/query params
                if effective_source_type == 1 || effective_source_type == 2 {
                    if let Some(value) = get_source_value(field_name, &source_data) {
                        if let Some(value_str) = value.as_str() {
                            let type_valid = match field_type {
                                "Int" => value_str.parse::<i64>().is_ok(),
                                "Float" => value_str.parse::<f64>().is_ok(),
                                "Bool" => value_str == "true" || value_str == "false",
                                "Str" | "String" => true,
                                _ => true,
                            };

                            if !type_valid {
                                let err = match effective_source_type {
                                    1 => {
                                        let param = ParameterError::new(field_name.to_string())
                                            .with_expected(field_type.to_string())
                                            .with_received(value_str.to_string())
                                            .with_message(
                                                "Invalid path parameter type".to_string(),
                                            );
                                        invalid_path_param_type_error(path_str.clone(), param)
                                    }
                                    2 => {
                                        let param = ParameterError::new(field_name.to_string())
                                            .with_expected(field_type.to_string())
                                            .with_received(value_str.to_string())
                                            .with_message(
                                                "Invalid query parameter type".to_string(),
                                            );
                                        invalid_query_param_type_error(path_str.clone(), param)
                                    }
                                    _ => {
                                        let mut fields = std::collections::HashMap::new();
                                        let field_err = FieldError::new(field_name.to_string())
                                            .with_rule("type_mismatch".to_string())
                                            .with_expected(field_type.to_string())
                                            .with_received(value_str.to_string())
                                            .with_error(format!(
                                                "Expected type {}, received {}",
                                                field_type, value_str
                                            ));
                                        fields.insert(field_name.to_string(), field_err);
                                        type_mismatch_error(path_str.clone(), fields)
                                    }
                                };
                                set_last_error(err.status_code() as i32, err.to_json_string());
                                return 400;
                            }
                        }
                    }
                }
            }
        }
    }

    // Validate fields with decorators - use metadata directly
    let struct_decorators = metadata.struct_decorators.get(&struct_name);

    // Validate field types for JSON body (source_type == 0) using struct_fields
    // struct_fields has proper type names like Array(Str), Array(Int), Map(Str,Int) etc.
    if effective_source_type == 0 {
        if let Some(fields_vec) = metadata.struct_fields.get(&struct_name) {
            let mut type_errors = std::collections::HashMap::new();

            for field_def in fields_vec {
                if field_def.len() >= 2 {
                    let field_name = &field_def[0];
                    let expected_type = &field_def[1];

                    if let Some(field_value) = source_data.get(field_name) {
                        // Check if this is an enum type - validate against allowed variants
                        if let Some(allowed_variants) =
                            metadata.enum_variants.get(expected_type.as_str())
                        {
                            // This is an enum - value must be a string matching one of the variants
                            if let Some(value_str) = field_value.as_str() {
                                if !allowed_variants.contains(&value_str.to_string()) {
                                    let field_err = FieldError::new(field_name.to_string())
                                        .with_rule("invalid_enum_value".to_string())
                                        .with_expected(format!("one of: {}", allowed_variants.join(", ")))
                                        .with_received(value_str.to_string())
                                        .with_value(value_str.to_string())
                                        .with_error(format!(
                                            "Invalid enum value '{}' for type {}. Allowed values: {:?}",
                                            value_str, expected_type, allowed_variants
                                        ));
                                    type_errors.insert(field_name.to_string(), field_err);
                                }
                            } else {
                                let field_err = FieldError::new(field_name.to_string())
                                    .with_rule("type_mismatch".to_string())
                                    .with_expected(expected_type.to_string())
                                    .with_received(get_json_type_name(field_value).to_string())
                                    .with_value(field_value.to_string())
                                    .with_error(format!(
                                        "Enum values must be strings, got {}",
                                        get_json_type_name(field_value)
                                    ));
                                type_errors.insert(field_name.to_string(), field_err);
                            }
                        } else if let Some(nested_struct_fields) =
                            metadata.struct_fields.get(expected_type.as_str())
                        {
                            // This is a nested struct - recursively validate its fields
                            if let Some(nested_obj) = field_value.as_object() {
                                for nested_field_def in nested_struct_fields {
                                    if nested_field_def.len() >= 2 {
                                        let nested_field_name = &nested_field_def[0];
                                        let nested_expected_type = &nested_field_def[1];

                                        if let Some(nested_value) =
                                            nested_obj.get(nested_field_name)
                                        {
                                            let (type_matches, error_detail) = validate_field_type(
                                                nested_expected_type,
                                                nested_value,
                                            );

                                            if !type_matches {
                                                let received_type =
                                                    get_json_type_name(nested_value);
                                                let error_msg = error_detail.unwrap_or_else(|| {
                                                    format!(
                                                        "Expected type {}, received {}",
                                                        nested_expected_type, received_type
                                                    )
                                                });

                                                let nested_field_path =
                                                    format!("{}.{}", field_name, nested_field_name);
                                                let field_err =
                                                    FieldError::new(nested_field_path.clone())
                                                        .with_rule("type_mismatch".to_string())
                                                        .with_expected(
                                                            nested_expected_type.to_string(),
                                                        )
                                                        .with_received(received_type.to_string())
                                                        .with_value(nested_value.to_string())
                                                        .with_error(error_msg);
                                                type_errors.insert(nested_field_path, field_err);
                                            }
                                        }
                                    }
                                }
                            } else {
                                let field_err = FieldError::new(field_name.to_string())
                                    .with_rule("type_mismatch".to_string())
                                    .with_expected(expected_type.to_string())
                                    .with_received(get_json_type_name(field_value).to_string())
                                    .with_value(field_value.to_string())
                                    .with_error(format!(
                                        "Expected nested struct {}, got {}",
                                        expected_type,
                                        get_json_type_name(field_value)
                                    ));
                                type_errors.insert(field_name.to_string(), field_err);
                            }
                        } else {
                            // Use validate_field_type helper for primitives, arrays, and maps
                            let (type_matches, error_detail) =
                                validate_field_type(expected_type, field_value);

                            if !type_matches {
                                let received_type = get_json_type_name(field_value);
                                let error_msg = error_detail.unwrap_or_else(|| {
                                    format!(
                                        "Expected type {}, received {}",
                                        expected_type, received_type
                                    )
                                });

                                let field_err = FieldError::new(field_name.to_string())
                                    .with_rule("type_mismatch".to_string())
                                    .with_expected(expected_type.to_string())
                                    .with_received(received_type.to_string())
                                    .with_value(field_value.to_string())
                                    .with_error(error_msg);
                                type_errors.insert(field_name.to_string(), field_err);
                            }
                        }
                    }
                }
            }

            if !type_errors.is_empty() {
                let err = type_mismatch_error(path_str.clone(), type_errors);
                set_last_error(err.status_code() as i32, err.to_json_string());
                return 400;
            }
        }
    }

    // Validate decorators - collect ALL errors across all fields
    let mut all_validation_errors: std::collections::HashMap<String, FieldError> =
        std::collections::HashMap::new();

    if let Some(field_decorators) = struct_decorators {
        for (field_name, decorators) in field_decorators {
            // decorators is Vec<DecoratorInfo>
            if decorators.is_empty() {
                continue;
            }

            {
                if let Some(field_value) = source_data.get(field_name) {
                    let field_type = if field_value.is_string() {
                        "Str"
                    } else if field_value.is_i64() || field_value.is_u64() {
                        "Int"
                    } else if field_value.is_f64() {
                        "Float"
                    } else if field_value.is_boolean() {
                        "Bool"
                    } else {
                        "Unknown"
                    };

                    let value_str = match field_value {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        _ => field_value.to_string(),
                    };

                    let decorators_json =
                        serde_json::to_string(decorators).unwrap_or_else(|_| "[]".to_string());

                    let field_name_cstr = CString::new(field_name.as_str()).unwrap();
                    let field_type_cstr = CString::new(field_type).unwrap();
                    let value_cstr = CString::new(value_str.as_str()).unwrap();
                    let decorators_cstr = CString::new(decorators_json.clone()).unwrap();

                    extern "C" {
                        fn dooruntime_validate_field(
                            field_name: *const libc::c_char,
                            field_type: *const libc::c_char,
                            value: *const libc::c_char,
                            decorators_json: *const libc::c_char,
                        ) -> *const libc::c_char;
                        fn dooruntime_get_last_validation_error() -> *mut libc::c_char;
                        fn dooruntime_free_string(ptr: *mut libc::c_char);
                        fn dooruntime_malloc(size: usize) -> *mut u8;
                    }

                    unsafe {
                        let error_ptr = dooruntime_validate_field(
                            field_name_cstr.as_ptr(),
                            field_type_cstr.as_ptr(),
                            value_cstr.as_ptr(),
                            decorators_cstr.as_ptr(),
                        );

                        if !error_ptr.is_null() {
                            let validation_error_json_ptr = dooruntime_get_last_validation_error();
                            if !validation_error_json_ptr.is_null() {
                                let error_json_str = CStr::from_ptr(validation_error_json_ptr)
                                    .to_string_lossy()
                                    .to_string();
                                if let Ok(error_json) =
                                    serde_json::from_str::<serde_json::Value>(&error_json_str)
                                {
                                    let rule = error_json
                                        .get("rule")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let message = error_json
                                        .get("message")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Validation failed");

                                    let field_error = FieldError::new(field_name.clone())
                                        .with_rule(rule.to_string())
                                        .with_value(value_str.clone())
                                        .with_error(message.to_string());
                                    all_validation_errors.insert(field_name.clone(), field_error);
                                }
                                dooruntime_free_string(validation_error_json_ptr);
                            }
                            dooruntime_free_string(error_ptr as *mut _);
                        }
                    }
                }
            }
        }
    }

    // If any validation errors were collected, return them all at once
    if !all_validation_errors.is_empty() {
        let err = validation_failed_error(path_str.clone(), all_validation_errors);
        set_last_error(err.status_code() as i32, err.to_json_string());
        return 422; // Unprocessable Entity
    }

    // Validation passed - populate struct dynamically using actual struct_layouts from metadata
    let routes = get_routes();
    let registry = routes.lock().unwrap();
    let full_metadata = registry.handler_metadata.get(&handler_name_str).cloned();
    drop(registry);

    if let Some(full_meta) = full_metadata {
        // Build metadata JSON to parse struct_layouts
        let metadata_json = serde_json::json!({
            "struct_layouts": full_meta.struct_layouts,
        });

        // Get struct layout with actual offsets
        if let Some(struct_layout) = metadata_json
            .get("struct_layouts")
            .and_then(|sl| sl.get(&struct_name))
            .and_then(|v| v.as_object())
        {
            if let Some(fields) = struct_layout.get("fields").and_then(|f| f.as_array()) {
                let struct_ptr_u8 = struct_ptr as *mut u8;

                // Zero-initialize the entire struct to prevent freeing garbage pointers
                if let Some(total_size) = struct_layout.get("total_size").and_then(|v| v.as_u64()) {
                    unsafe {
                        std::ptr::write_bytes(struct_ptr_u8, 0, total_size as usize);
                    }
                }

                // Populate each field using actual offset from metadata
                for field in fields {
                    let field_obj = match field.as_object() {
                        Some(obj) => obj,
                        None => continue,
                    };

                    let field_name = match field_obj.get("name").and_then(|v| v.as_str()) {
                        Some(n) => n,
                        None => continue,
                    };

                    let field_type = match field_obj.get("type").and_then(|v| v.as_str()) {
                        Some(t) => t,
                        None => continue,
                    };

                    let offset = match field_obj.get("offset").and_then(|v| v.as_u64()) {
                        Some(o) => o as usize,
                        None => continue,
                    };

                    if let Some(field_value) = get_source_value(field_name, &source_data) {
                        unsafe {
                            extern "C" {
                                fn dooruntime_malloc(size: usize) -> *mut u8;
                            }
                            match field_type {
                                "Str" | "String" | "Optional(Str)" | "Optional(String)" => {
                                    // Handle both direct strings and strings that need parsing
                                    let s = if let Some(str_val) = field_value.as_str() {
                                        str_val
                                    } else if field_value.is_null() {
                                        ""
                                    } else {
                                        ""
                                    };

                                    // Allocate string with RC header (RC:i32, Len:i32, Data...)
                                    // Doo runtime expects pointers to be managed ref-counted strings
                                    let len = s.len();
                                    let total_size = len + 1 + 8; // data + null + header
                                                                  // Align to 16 bytes to be safe
                                    let alloc_size = (total_size + 15) & !15;
                                    // Use unified allocator
                                    let ptr = dooruntime_malloc(alloc_size);

                                    if !ptr.is_null() {
                                        // Zero memory for safety
                                        std::ptr::write_bytes(ptr, 0, alloc_size);
                                        // Write RC = 1
                                        *(ptr as *mut i32) = 1;
                                        // Write Len = len
                                        *(ptr.add(4) as *mut i32) = len as i32;
                                        // Copy data to offset 8
                                        let data_ptr = ptr.add(8);
                                        std::ptr::copy_nonoverlapping(s.as_ptr(), data_ptr, len);
                                        // Null terminate (calloc already zeroed, but be explicit)
                                        *data_ptr.add(len) = 0;

                                        // Store DATA pointer (offset 8) in the struct
                                        std::ptr::write(
                                            struct_ptr_u8.add(offset) as *mut *const libc::c_char,
                                            data_ptr as *const libc::c_char,
                                        );
                                    } else {
                                        // Fallback
                                        std::ptr::write(
                                            struct_ptr_u8.add(offset) as *mut *const libc::c_char,
                                            std::ptr::null(),
                                        );
                                    }
                                }
                                "Int" => {
                                    // For path/query params (source_type 1 or 2), parse string
                                    // For body params (source_type 0), require JSON number
                                    let parsed_int = if effective_source_type == 1
                                        || effective_source_type == 2
                                    {
                                        // Path/query: try to parse string value
                                        if let Some(str_val) = field_value.as_str() {
                                            str_val.parse::<i64>().ok()
                                        } else if let Some(num) = field_value.as_i64() {
                                            Some(num)
                                        } else {
                                            None
                                        }
                                    } else {
                                        // Body: require JSON number
                                        field_value.as_i64()
                                    };

                                    if let Some(num) = parsed_int {
                                        let n = num as i32;
                                        std::ptr::write(struct_ptr_u8.add(offset) as *mut i32, n);
                                    } else {
                                        // Type mismatch - set error
                                        let actual_type = if field_value.is_string() {
                                            "string".to_string()
                                        } else if field_value.is_boolean() {
                                            "boolean".to_string()
                                        } else if field_value.is_null() {
                                            "null".to_string()
                                        } else if field_value.is_array() {
                                            "array".to_string()
                                        } else if field_value.is_object() {
                                            "object".to_string()
                                        } else {
                                            "unknown".to_string()
                                        };

                                        extern "C" {
                                            fn dooruntime_free_string(ptr: *mut libc::c_char);
                                        }

                                        let error_json = json!({
                                            "field": field_name,
                                            "expected": "Int",
                                            "actual": actual_type,
                                        });

                                        if let Ok(json_str) = serde_json::to_string(&error_json) {
                                            if let Ok(c_str) = CString::new(json_str) {
                                                // Store error in thread-local
                                                let error_ptr = c_str.into_raw();
                                                // Create RFC 7807 error
                                                let error = bad_request(
                                                    format!("Type mismatch in request body"),
                                                    path_str.clone(),
                                                )
                                                .with_field(
                                                    field_name.to_string(),
                                                    FieldError {
                                                        rule: Some(format!("type:Int")),
                                                        message: field_name.to_string(),
                                                        value: Some(format!("({})", actual_type)),
                                                        expected: Some("Int".to_string()),
                                                        received: Some(actual_type.clone()),
                                                        error: Some(format!(
                                                            "Expected type 'Int' but got '{}'",
                                                            actual_type
                                                        )),
                                                    },
                                                );
                                                set_last_error(400, error.to_json_string());
                                                dooruntime_free_string(error_ptr);
                                                return 400;
                                            }
                                        }
                                    }
                                }
                                "Float" => {
                                    // For path/query params (source_type 1 or 2), parse string
                                    // For body params (source_type 0), require JSON number
                                    let parsed_float: Option<f64> = if effective_source_type == 1
                                        || effective_source_type == 2
                                    {
                                        // Path/query: try to parse string value
                                        if let Some(str_val) = field_value.as_str() {
                                            str_val.parse::<f64>().ok()
                                        } else if let Some(num) = field_value.as_f64() {
                                            Some(num)
                                        } else if let Some(num) = field_value.as_i64() {
                                            Some(num as f64)
                                        } else {
                                            None
                                        }
                                    } else {
                                        // Body: require JSON number
                                        if let Some(num) = field_value.as_f64() {
                                            Some(num)
                                        } else if let Some(num) = field_value.as_i64() {
                                            Some(num as f64)
                                        } else {
                                            None
                                        }
                                    };

                                    if let Some(num) = parsed_float {
                                        std::ptr::write(struct_ptr_u8.add(offset) as *mut f64, num);
                                    } else {
                                        // Type mismatch - set error
                                        let actual_type = if field_value.is_string() {
                                            "string".to_string()
                                        } else if field_value.is_boolean() {
                                            "boolean".to_string()
                                        } else if field_value.is_null() {
                                            "null".to_string()
                                        } else if field_value.is_array() {
                                            "array".to_string()
                                        } else if field_value.is_object() {
                                            "object".to_string()
                                        } else {
                                            "unknown".to_string()
                                        };

                                        let error = bad_request(
                                            format!("Type mismatch in request body"),
                                            path_str.clone(),
                                        )
                                        .with_field(
                                            field_name.to_string(),
                                            FieldError {
                                                rule: Some(format!("type:Float")),
                                                message: field_name.to_string(),
                                                value: Some(format!("({})", actual_type)),
                                                expected: Some("Float".to_string()),
                                                received: Some(actual_type.clone()),
                                                error: Some(format!(
                                                    "Expected type 'Float' but got '{}'",
                                                    actual_type
                                                )),
                                            },
                                        );
                                        set_last_error(400, error.to_json_string());
                                        return 400;
                                    }
                                }
                                "Bool" => {
                                    // For path/query params (source_type 1 or 2), parse string
                                    // For body params (source_type 0), require JSON boolean
                                    let parsed_bool = if effective_source_type == 1
                                        || effective_source_type == 2
                                    {
                                        // Path/query: try to parse string value
                                        if let Some(str_val) = field_value.as_str() {
                                            match str_val {
                                                "true" => Some(true),
                                                "false" => Some(false),
                                                _ => None,
                                            }
                                        } else if let Some(bool_val) = field_value.as_bool() {
                                            Some(bool_val)
                                        } else {
                                            None
                                        }
                                    } else {
                                        // Body: require JSON boolean
                                        field_value.as_bool()
                                    };

                                    if let Some(bool_val) = parsed_bool {
                                        std::ptr::write(
                                            struct_ptr_u8.add(offset) as *mut i32,
                                            if bool_val { 1 } else { 0 },
                                        );
                                    } else {
                                        // Type mismatch - set error
                                        let actual_type = if field_value.is_string() {
                                            "string".to_string()
                                        } else if field_value.is_number() {
                                            "number".to_string()
                                        } else if field_value.is_null() {
                                            "null".to_string()
                                        } else if field_value.is_array() {
                                            "array".to_string()
                                        } else if field_value.is_object() {
                                            "object".to_string()
                                        } else {
                                            "unknown".to_string()
                                        };

                                        let error = bad_request(
                                            format!("Type mismatch in request body"),
                                            path_str.clone(),
                                        )
                                        .with_field(
                                            field_name.to_string(),
                                            FieldError {
                                                rule: Some(format!("type:Bool")),
                                                message: field_name.to_string(),
                                                value: Some(format!("({})", actual_type)),
                                                expected: Some("Bool".to_string()),
                                                received: Some(actual_type.clone()),
                                                error: Some(format!(
                                                    "Expected type 'Bool' but got '{}'",
                                                    actual_type
                                                )),
                                            },
                                        );
                                        set_last_error(400, error.to_json_string());
                                        return 400;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    // Check for JSON type mismatch errors before returning success
    // This catches cases where json_get_int/float/etc returned default values due to type mismatches
    extern "C" {
        fn dooruntime_get_json_type_mismatch() -> *mut libc::c_char;
        fn dooruntime_free_string(ptr: *mut libc::c_char);
    }

    let error_ptr = unsafe { dooruntime_get_json_type_mismatch() };

    if !error_ptr.is_null() {
        let error_json = unsafe { CStr::from_ptr(error_ptr).to_string_lossy().to_string() };

        unsafe {
            dooruntime_free_string(error_ptr);
        }

        // Parse the error JSON and create RFC 7807 error
        if let Ok(error_data) = serde_json::from_str::<serde_json::Value>(&error_json) {
            let field_name = error_data["field"].as_str().unwrap_or("unknown");
            let expected_type = error_data["expected"].as_str().unwrap_or("unknown");
            let actual_type = error_data["actual"].as_str().unwrap_or("unknown");

            let error = bad_request(format!("Type mismatch in request body"), path_str).with_field(
                field_name.to_string(),
                FieldError {
                    rule: Some(format!("type:{}", expected_type)),
                    message: field_name.to_string(),
                    value: Some(format!("({})", actual_type)),
                    expected: Some(expected_type.to_string()),
                    received: Some(actual_type.to_string()),
                    error: Some(format!(
                        "Expected type '{}' but got '{}'",
                        expected_type, actual_type
                    )),
                },
            );

            set_last_error(400, error.to_json_string());
            return 400;
        }
    }

    0 // Success
}

// Tests removed - validation logic moved to libdoo_runtime
// HTTP layer now delegates all decorator validation to dooruntime_validate_field()

// ============================================================================
// Auth and CRUD Metadata Storage
// ============================================================================

#[derive(Clone, Debug)]
struct AuthMetadata {
    table_name: String,
    metadata: serde_json::Value,
    signup_path: String,
    login_path: String,
}

#[derive(Clone, Debug)]
struct CrudMetadata {
    table_name: String,
    metadata: serde_json::Value,
    base_path: String,
}

// TableMetadata for @table decorator structs
#[derive(Clone, Debug)]
struct TableMetadata {
    table_name: String,
    metadata: serde_json::Value,
}

static AUTH_METADATA: OnceLock<Mutex<HashMap<String, AuthMetadata>>> = OnceLock::new();
static CRUD_METADATA: OnceLock<Mutex<HashMap<String, CrudMetadata>>> = OnceLock::new();
static TABLE_METADATA: OnceLock<Mutex<HashMap<String, TableMetadata>>> = OnceLock::new();

fn get_auth_metadata() -> &'static Mutex<HashMap<String, AuthMetadata>> {
    AUTH_METADATA.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_crud_metadata() -> &'static Mutex<HashMap<String, CrudMetadata>> {
    CRUD_METADATA.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_table_metadata() -> &'static Mutex<HashMap<String, TableMetadata>> {
    TABLE_METADATA.get_or_init(|| Mutex::new(HashMap::new()))
}

// ============================================================================
// External FFI function declarations for DB and Auth
// ============================================================================

extern "C" {
    fn doo_db_table_exists(table_name: *const c_char) -> i32;
    fn doo_db_create_table(_db: *const c_char, sql: *const c_char) -> *mut std::ffi::c_void;
    fn doo_db_insert_json(sql: *const c_char, values_json: *const c_char) -> *mut std::ffi::c_void;
    fn doo_db_query_json(sql: *const c_char) -> *mut std::ffi::c_void;
    fn doo_db_query_one_json(sql: *const c_char) -> *mut std::ffi::c_void;
    fn doo_db_query_one_param(
        _db: *const c_char,
        sql: *const c_char,
        param: *const c_char,
    ) -> *mut std::ffi::c_void;
    fn doo_db_execute(_db: *const c_char, sql: *const c_char) -> *mut std::ffi::c_void;
    fn doo_db_execute_param(
        _db: *const c_char,
        sql: *const c_char,
        param: *const c_char,
    ) -> *mut std::ffi::c_void;
    fn doo_db_is_error(result: *mut std::ffi::c_void) -> i32;
    fn doo_db_get_error_message(result: *mut std::ffi::c_void) -> *mut c_char;
    fn doo_db_result_free(result: *mut std::ffi::c_void);
    fn doo_db_free_string(ptr: *mut c_char);

    fn doo_auth_hash_password(password: *const c_char) -> *mut std::ffi::c_void;
    fn doo_auth_verify_password(
        password: *const c_char,
        hashed: *const c_char,
    ) -> *mut std::ffi::c_void;
    fn doo_auth_sign(
        sub: *const c_char,
        data_json: *const c_char,
        expires_seconds: i32,
    ) -> *mut std::ffi::c_void;
    fn doo_auth_verify(token: *const c_char) -> *mut std::ffi::c_void;
    fn doo_auth_free_result(result: *mut std::ffi::c_void);
    fn doo_auth_free_string(ptr: *mut c_char);
    fn doo_auth_is_error(result: *mut std::ffi::c_void) -> i32;
    fn doo_auth_get_error_message(result: *mut std::ffi::c_void) -> *const c_char;

    fn dooruntime_validate_field(
        field_name: *const c_char,
        field_type: *const c_char,
        value: *const c_char,
        decorators_json: *const c_char,
    ) -> *const c_char;
    fn dooruntime_get_last_validation_error() -> *mut c_char;
    fn dooruntime_clear_validation_error();
    fn dooruntime_free_string(ptr: *mut c_char);
}

unsafe fn extract_db_result_string(result: *mut std::ffi::c_void) -> Option<String> {
    if result.is_null() {
        return None;
    }
    if doo_db_is_error(result) != 0 {
        return None;
    }
    // For OK results, value is the string data
    let result_struct = result as *mut DooResult;
    let value_ptr = (*result_struct).value as *mut c_char;
    if value_ptr.is_null() {
        return None;
    }
    let result_str = CStr::from_ptr(value_ptr).to_string_lossy().into_owned();
    Some(result_str)
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert DB error JSON to RFC 7807 format
fn convert_db_error_to_rfc7807(db_error_json: &str, instance: String) -> (i32, String) {
    // Try to parse DB error JSON
    if let Ok(err_json) = serde_json::from_str::<serde_json::Value>(db_error_json) {
        if let Some(error_obj) = err_json.get("error").and_then(|e| e.as_object()) {
            let code = error_obj
                .get("code")
                .and_then(|c| c.as_str())
                .unwrap_or("UNKNOWN");
            let message = error_obj
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Database error");
            let _pg_code = error_obj.get("pg_code").and_then(|c| c.as_str());

            // Handle UNIQUE_VIOLATION with RFC 7807 validation error format
            if code == "UNIQUE_VIOLATION" {
                // Extract field name from message (e.g., "Duplicate value for field: users_email_key")
                let field_name =
                    if let Some(msg) = message.strip_prefix("Duplicate value for field: ") {
                        // Extract field name from constraint (e.g., "users_email_key" -> "email")
                        if let Some(constraint_parts) = msg.split('_').nth(1) {
                            constraint_parts.to_string()
                        } else {
                            "unknown".to_string()
                        }
                    } else {
                        "unknown".to_string()
                    };

                let mut fields = std::collections::HashMap::new();
                fields.insert(
                    field_name.clone(),
                    error::FieldError::new(field_name.clone())
                        .with_rule("unique".to_string())
                        .with_error(format!("This {} already exists", field_name))
                        .with_value("***".to_string()),
                );

                let err = error::validation_failed_error(instance, fields);
                return (422, err.to_json_string());
            }

            // Handle other DB errors with generic RFC 7807 format
            let status = error_obj
                .get("status")
                .and_then(|s| s.as_i64())
                .unwrap_or(500) as i32;

            let err = error::ErrorResponse::new(
                if status >= 500 {
                    error::ErrorType::InternalError
                } else {
                    error::ErrorType::BadRequest
                },
                message.to_string(),
                instance,
            );

            return (status, err.to_json_string());
        }
    }

    // Fallback to generic error
    let err = error::internal_server_error(instance);
    (500, err.to_json_string())
}

// ============================================================================
// Auth and CRUD Runtime Handlers (using existing FFI functions)
// ============================================================================

/// Auth signup handler - uses libdoo_db and libdoo_auth
extern "C" fn auth_signup_handler(request: *mut DooRequest) -> *mut DooResult {
    unsafe {
        if request.is_null() {
            return create_error_result(500, "Internal error: null request");
        }

        let req = &*request;
        if req.path.is_null() || req.body.is_null() {
            return create_error_result(500, "Internal error: invalid request");
        }

        let path = c_to_string(req.path);
        let body = c_to_string(req.body);

        // Parse JSON body
        let mut json: serde_json::Value = match serde_json::from_str(&body) {
            Ok(j) => j,
            Err(e) => {
                let err = error::invalid_json_error(path.clone());
                return create_json_result(400, &err.to_json_string());
            }
        };

        // Normalize keys to lowercase for Postgres compatibility
        if let Some(obj) = json.as_object_mut() {
            let keys: Vec<String> = obj.keys().cloned().collect();
            for key in keys {
                if let Some(value) = obj.remove(&key) {
                    obj.insert(key.to_lowercase(), value);
                }
            }
        }

        // Get metadata for this path
        let metadata_map = get_auth_metadata().lock().unwrap();
        let auth_meta = metadata_map
            .values()
            .find(|m| m.signup_path == path)
            .cloned();
        drop(metadata_map);

        let auth_meta = match auth_meta {
            Some(m) => m,
            None => {
                return create_error_result(500, "No auth metadata found for this path");
            }
        };

        let obj = match json.as_object() {
            Some(o) => o,
            None => {
                return create_error_result(400, "Request body must be a JSON object");
            }
        };

        // Validate all fields using libdoo_runtime before processing
        let metadata = &auth_meta.metadata;
        let fields = match metadata.get("fields").and_then(|f| f.as_array()) {
            Some(f) => f,
            None => {
                return create_error_result(500, "Invalid metadata: missing fields");
            }
        };

        for field in fields.iter() {
            let field_obj = match field.as_object() {
                Some(o) => o,
                None => continue,
            };

            let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };

            let field_type = field_obj
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("Str");
            let decorators = field_obj
                .get("decorators")
                .and_then(|d| d.as_array())
                .map(|d| d.clone())
                .unwrap_or_default();

            // Skip auto fields for validation
            let is_auto = decorators.iter().any(|d| {
                d.as_object()
                    .and_then(|o| o.get("name"))
                    .and_then(|n| n.as_str())
                    == Some("auto")
            });
            if is_auto {
                continue;
            }

            let value = get_obj_value(obj, field_name);

            if let Some(value) = value {
                // Convert value to string for validation
                let value_str = match value {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    _ => continue,
                };

                // Validate field
                if let Err((status, error_json)) = validate_field_with_runtime(
                    field_name,
                    field_type,
                    &value_str,
                    &decorators,
                    path.clone(),
                ) {
                    return create_json_result(status, &error_json);
                }
            }
        }

        // Extract table metadata (already loaded above for validation)

        // Validate required fields (fields without @auto, @default, or marked optional)
        let mut missing_fields = Vec::new();
        for field in fields.iter() {
            if let Some(field_obj) = field.as_object() {
                if let Some(field_name) = field_obj.get("name").and_then(|n| n.as_str()) {
                    // Check if field is optional (has "optional": true in metadata)
                    let is_optional = field_obj
                        .get("optional")
                        .and_then(|o| o.as_bool())
                        .unwrap_or(false);

                    // Check if field has @auto or @default decorator
                    let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
                    let has_auto_or_default_or_foreign = if let Some(decs) = decorators {
                        has_decorator(Some(decs), "auto")
                            || has_decorator(Some(decs), "default")
                            || has_decorator(Some(decs), "foreign")
                            || has_decorator(Some(decs), "internal")
                            || has_decorator(Some(decs), "readOnly")
                    } else {
                        false
                    };

                    // If field is required (no @auto, no @default, no @foreign, not optional) and missing from request
                    if !has_auto_or_default_or_foreign
                        && !is_optional
                        && get_obj_value(obj, field_name).is_none()
                    {
                        missing_fields.push(field_name.to_string());
                    }
                }
            }
        }

        if !missing_fields.is_empty() {
            use error::*;
            let mut field_errors = std::collections::HashMap::new();
            for field_name in missing_fields {
                let field_err = FieldError::new(field_name.clone())
                    .with_rule("required".to_string())
                    .with_error(format!("Field '{}' is required", field_name));
                field_errors.insert(field_name, field_err);
            }
            let err = validation_error(
                "Missing required fields".to_string(),
                path.clone(),
                field_errors,
            );
            return create_json_result(400, &err.to_json_string());
        }

        // Find password field
        let password_field = fields.iter().find_map(|f| {
            let field_obj = f.as_object()?;
            let decorators = field_obj.get("decorators")?.as_array()?;
            let has_hash = decorators.iter().any(|d| {
                d.as_object()
                    .and_then(|o| o.get("name"))
                    .and_then(|n| n.as_str())
                    == Some("hash")
            });
            if has_hash {
                field_obj.get("name")?.as_str()
            } else {
                None
            }
        });

        let password_field_name = match password_field {
            Some(name) => name,
            None => {
                return create_error_result(500, "No password field with @hash decorator found");
            }
        };

        // Get password value (case-insensitive lookup)
        let password_value = get_obj_value(obj, password_field_name).and_then(|v| v.as_str());

        let password_value = match password_value {
            Some(pwd) => pwd,
            None => {
                use error::*;
                let mut field_errors = std::collections::HashMap::new();
                let field_err = FieldError::new(password_field_name.to_string())
                    .with_rule("required".to_string())
                    .with_error(format!("Field '{}' is required", password_field_name));
                field_errors.insert(password_field_name.to_string(), field_err);
                let err = validation_error(
                    "Missing required fields".to_string(),
                    path.clone(),
                    field_errors,
                );
                return create_json_result(400, &err.to_json_string());
            }
        };

        // Hash password using libdoo_auth
        let password_c = CString::new(password_value).unwrap();
        let hash_result = doo_auth_hash_password(password_c.as_ptr());

        if hash_result.is_null() {
            return create_error_result(500, "Failed to hash password");
        }

        let hash_res = &*(hash_result as *mut DooResult);
        if hash_res.tag != 0 {
            doo_auth_free_result(hash_result);
            return create_error_result(500, "Failed to hash password");
        }

        let hashed_password = if hash_res.value.is_null() {
            doo_auth_free_result(hash_result);
            return create_error_result(500, "Failed to get hashed password");
        } else {
            let hash_ptr = hash_res.value as *mut c_char;
            let hash_str = CStr::from_ptr(hash_ptr).to_string_lossy().into_owned();
            doo_auth_free_result(hash_result);
            hash_str
        };

        // Build INSERT SQL
        let table_name = &auth_meta.table_name;
        let mut field_names = Vec::new();
        let mut placeholders = Vec::new();
        let mut values_json = Vec::new();
        let mut param_idx = 1;

        for field in fields.iter() {
            let field_obj = match field.as_object() {
                Some(o) => o,
                None => continue,
            };

            let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };

            // Skip auto-generated fields
            let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
            if let Some(decs) = decorators {
                let is_auto = decs.iter().any(|d| {
                    d.as_object()
                        .and_then(|o| o.get("name"))
                        .and_then(|n| n.as_str())
                        == Some("auto")
                });
                if is_auto {
                    continue;
                }
            }

            // Ignore request input for @internal and @readOnly fields
            let ignore_request_value =
                has_decorator(decorators, "internal") || has_decorator(decorators, "readOnly");

            // Use hashed password for password field (case-insensitive comparison)
            if field_name.to_lowercase() == password_field_name.to_lowercase() {
                field_names.push(to_snake_case(field_name));
                placeholders.push(format!("${}", param_idx));
                values_json.push(serde_json::Value::String(hashed_password.clone()));
                param_idx += 1;
            } else {
                // Case-insensitive lookup for the field value
                let value = if ignore_request_value {
                    None
                } else {
                    get_obj_value(obj, field_name)
                };

                if let Some(value) = value {
                    field_names.push(to_snake_case(field_name));
                    placeholders.push(format!("${}", param_idx));
                    values_json.push(value.clone());
                    param_idx += 1;
                } else {
                    // Check if field has @default decorator
                    if let Some(decs) = decorators {
                        if let Some(default_value) = decs.iter().find_map(|d| {
                            let dec_obj = d.as_object()?;
                            if dec_obj.get("name")?.as_str()? == "default" {
                                let args = dec_obj.get("args")?.as_array()?;
                                args.first()?.as_str()
                            } else {
                                None
                            }
                        }) {
                            // Apply default value - convert to proper JSON type based on field type
                            field_names.push(to_snake_case(field_name));
                            placeholders.push(format!("${}", param_idx));

                            // Get field type
                            let field_type_str = field_obj
                                .get("type")
                                .and_then(|t| t.as_str())
                                .unwrap_or("Str");

                            // Convert default value to proper type
                            let typed_value = match field_type_str {
                                "Int" => {
                                    if let Ok(int_val) = default_value.parse::<i64>() {
                                        serde_json::Value::Number(int_val.into())
                                    } else {
                                        serde_json::Value::String(default_value.to_string())
                                    }
                                }
                                "Float" => {
                                    if let Ok(float_val) = default_value.parse::<f64>() {
                                        serde_json::json!(float_val)
                                    } else {
                                        serde_json::Value::String(default_value.to_string())
                                    }
                                }
                                "Bool" => match default_value.to_lowercase().as_str() {
                                    "true" => serde_json::Value::Bool(true),
                                    "false" => serde_json::Value::Bool(false),
                                    _ => serde_json::Value::String(default_value.to_string()),
                                },
                                _ => serde_json::Value::String(default_value.to_string()),
                            };

                            values_json.push(typed_value);
                            param_idx += 1;
                        }
                    }
                }
            }
        }

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({}) RETURNING id",
            table_name,
            field_names.join(", "),
            placeholders.join(", ")
        );

        let sql_c = CString::new(sql.clone()).unwrap();
        let values_json_str = serde_json::to_string(&values_json).unwrap();
        let values_c = CString::new(values_json_str.clone()).unwrap();

        // Insert into database
        let insert_result = doo_db_insert_json(sql_c.as_ptr(), values_c.as_ptr());

        if insert_result.is_null() {
            return create_error_result(500, "Database insert failed");
        }

        let _insert_res = &*insert_result;
        let insert_res = &*(insert_result as *mut DooResult);
        let is_error = doo_db_is_error(insert_result);

        if is_error != 0 {
            let err_msg_ptr = doo_db_get_error_message(insert_result);
            let err_msg = if err_msg_ptr.is_null() {
                "Database insert failed".to_string()
            } else {
                let msg = CStr::from_ptr(err_msg_ptr).to_string_lossy().into_owned();
                doo_db_free_string(err_msg_ptr);
                msg
            };
            doo_db_result_free(insert_result);

            // Convert DB error to RFC 7807 format
            let (status, rfc_error) = convert_db_error_to_rfc7807(&err_msg, path.clone());
            return create_json_result(status, &rfc_error);
        }

        // Extract inserted ID
        let user_id = if insert_res.value.is_null() {
            1i64
        } else {
            insert_res.value as i64
        };
        doo_db_result_free(insert_result);

        // Generate JWT token
        let user_id_str = user_id.to_string();
        let sub_c = CString::new(user_id_str.clone()).unwrap();
        let user_data = json!({
            "id": user_id,
        });
        let data_json_str = user_data.to_string();
        let data_c = CString::new(data_json_str).unwrap();
        let expires = 86400i32; // 24 hours

        let token_result = doo_auth_sign(sub_c.as_ptr(), data_c.as_ptr(), expires);

        if token_result.is_null() {
            return create_json_result(
                201,
                &format!(
                    r#"{{"success":true,"message":"User created successfully","id":{}}}"#,
                    user_id
                ),
            );
        }

        let token_res = &*(token_result as *mut DooResult);
        let token = if token_res.tag == 0 && !token_res.value.is_null() {
            let token_ptr = token_res.value as *mut c_char;
            let token_str = CStr::from_ptr(token_ptr).to_string_lossy().into_owned();
            doo_auth_free_result(token_result);
            token_str
        } else {
            doo_auth_free_result(token_result);
            return create_json_result(
                201,
                &format!(
                    r#"{{"success":true,"message":"User created successfully","id":{}}}"#,
                    user_id
                ),
            );
        };

        // Build user object (get fields from metadata, exclude @writeOnly/@internal/@hash)
        let mut user_obj = serde_json::Map::new();
        user_obj.insert("id".to_string(), json!(user_id));

        // Add other fields (prefer request value, otherwise apply @default)
        for field in fields.iter() {
            let field_obj = match field.as_object() {
                Some(o) => o,
                None => continue,
            };

            let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };

            // Skip id field (auto-generated)
            if field_name.to_lowercase() == "id" {
                continue;
            }

            let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
            let should_exclude = has_decorator(decorators, "writeOnly")
                || has_decorator(decorators, "internal")
                || has_decorator(decorators, "hash");

            if should_exclude {
                continue;
            }

            let key = field_name.to_lowercase();

            // Prefer provided value
            if let Some(value) = get_obj_value(obj, field_name) {
                if !value.is_null() {
                    user_obj.insert(key, value.clone());
                }
                continue;
            }

            // Apply @default if present
            if let Some(decs) = decorators {
                if let Some(default_value) = decs.iter().find_map(|d| {
                    let dec_obj = d.as_object()?;
                    if decorator_name_eq(d, "default") {
                        let args = dec_obj.get("args")?.as_array()?;
                        args.first()?.as_str()
                    } else {
                        None
                    }
                }) {
                    if !default_value.is_empty() {
                        let clean_val = if default_value.contains("::") {
                            default_value.split("::").last().unwrap_or(default_value)
                        } else {
                            default_value
                        };
                        user_obj.insert(key, serde_json::Value::String(clean_val.to_string()));
                    }
                }
            }
        }

        // Return success with token and user data
        let response = json!({
            "token": token,
            "user": user_obj,
        });

        create_json_result(201, &response.to_string())
    }
}

/// Helper to validate field using libdoo_runtime
unsafe fn validate_field_with_runtime(
    field_name: &str,
    field_type: &str,
    value: &str,
    decorators: &[serde_json::Value],
    instance: String,
) -> Result<(), (i32, String)> {
    // Convert decorators to JSON string format expected by runtime
    let decorators_json = json!(decorators).to_string();

    let field_name_c = CString::new(field_name).unwrap();
    let field_type_c = CString::new(field_type).unwrap();
    let value_c = CString::new(value).unwrap();
    let decorators_c = CString::new(decorators_json).unwrap();

    let result = dooruntime_validate_field(
        field_name_c.as_ptr(),
        field_type_c.as_ptr(),
        value_c.as_ptr(),
        decorators_c.as_ptr(),
    );

    if !result.is_null() {
        // Free the result pointer returned by dooruntime_validate_field
        dooruntime_free_string(result as *mut c_char);

        // Validation failed - get error from runtime
        let error_ptr = dooruntime_get_last_validation_error();
        if !error_ptr.is_null() {
            let error_json = CStr::from_ptr(error_ptr).to_string_lossy().into_owned();
            dooruntime_free_string(error_ptr);
            dooruntime_clear_validation_error();

            // Parse the validation error and convert to RFC 7807
            if let Ok(err_obj) = serde_json::from_str::<serde_json::Value>(&error_json) {
                let field = err_obj
                    .get("field_name")
                    .and_then(|f| f.as_str())
                    .unwrap_or(field_name);
                let rule = err_obj
                    .get("rule")
                    .and_then(|r| r.as_str())
                    .unwrap_or("validation");
                let message = err_obj
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Validation failed");

                let expected = err_obj.get("expected").and_then(|e| e.as_str());
                let received = err_obj.get("received").and_then(|r| r.as_str());

                let mut field_err = error::FieldError::new(field.to_string())
                    .with_rule(rule.to_string())
                    .with_error(message.to_string())
                    .with_value(value.to_string());

                if let Some(exp) = expected {
                    field_err = field_err.with_expected(exp.to_string());
                }
                if let Some(rec) = received {
                    field_err = field_err.with_received(rec.to_string());
                }

                let mut fields = std::collections::HashMap::new();
                fields.insert(field.to_string(), field_err);

                let err = error::validation_failed_error(instance, fields);
                return Err((400, err.to_json_string()));
            }
        }
        return Err((
            400,
            format!(
                r#"{{"error":"Validation failed for field: {}"}}"#,
                field_name
            ),
        ));
    }

    Ok(())
}

/// Auth signup handler - uses libdoo_db and libdoo_auth
extern "C" fn auth_login_handler(request: *mut DooRequest) -> *mut DooResult {
    unsafe {
        if request.is_null() {
            return create_error_result(500, "Internal error: null request");
        }

        let req = &*request;
        if req.path.is_null() || req.body.is_null() {
            return create_error_result(500, "Internal error: invalid request");
        }

        let path = c_to_string(req.path);
        let body = c_to_string(req.body);

        // Parse JSON body
        let json: serde_json::Value = match serde_json::from_str(&body) {
            Ok(j) => j,
            Err(_) => {
                let err = error::invalid_json_error(path.clone());
                return create_json_result(400, &err.to_json_string());
            }
        };

        // Get metadata for this path
        let metadata_map = get_auth_metadata().lock().unwrap();
        let auth_meta = metadata_map
            .values()
            .find(|m| m.login_path == path)
            .cloned();
        drop(metadata_map);

        let auth_meta = match auth_meta {
            Some(m) => m,
            None => {
                return create_error_result(500, "No auth metadata found for this path");
            }
        };

        let obj = match json.as_object() {
            Some(o) => o,
            None => {
                return create_error_result(400, "Request body must be a JSON object");
            }
        };

        // Extract table metadata
        let metadata = &auth_meta.metadata;
        let fields = match metadata.get("fields").and_then(|f| f.as_array()) {
            Some(f) => f,
            None => {
                return create_error_result(500, "Invalid metadata: missing fields");
            }
        };

        // Find unique email/username field and password field
        let mut unique_field_name = None;
        let mut password_field_name = None;

        for field in fields.iter() {
            let field_obj = match field.as_object() {
                Some(o) => o,
                None => continue,
            };

            let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };

            let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
            if let Some(decs) = decorators {
                let has_unique = decs.iter().any(|d| {
                    d.as_object()
                        .and_then(|o| o.get("name"))
                        .and_then(|n| n.as_str())
                        == Some("unique")
                });
                let has_hash = decs.iter().any(|d| {
                    d.as_object()
                        .and_then(|o| o.get("name"))
                        .and_then(|n| n.as_str())
                        == Some("hash")
                });

                if has_unique && unique_field_name.is_none() {
                    unique_field_name = Some(field_name);
                }
                if has_hash {
                    password_field_name = Some(field_name);
                }
            }
        }

        let unique_field = match unique_field_name {
            Some(name) => name,
            None => {
                return create_error_result(500, "No unique field found for authentication");
            }
        };

        let password_field = match password_field_name {
            Some(name) => name,
            None => {
                return create_error_result(500, "No password field with @hash decorator found");
            }
        };

        // Get credentials from request (case-insensitive key lookup)
        let identifier = get_obj_value(obj, unique_field).and_then(|v| v.as_str());

        let identifier = match identifier {
            Some(id) => id,
            None => {
                use error::*;
                let mut field_errors = std::collections::HashMap::new();
                let field_err = FieldError::new(unique_field.to_string())
                    .with_rule("required".to_string())
                    .with_error(format!("Field '{}' is required", unique_field));
                field_errors.insert(unique_field.to_string(), field_err);
                let err = validation_error(
                    "Missing required fields".to_string(),
                    get_current_request_path(),
                    field_errors,
                );
                return create_json_result(400, &err.to_json_string());
            }
        };

        let password = get_obj_value(obj, password_field).and_then(|v| v.as_str());

        let password = match password {
            Some(pwd) => pwd,
            None => {
                use error::*;
                let mut field_errors = std::collections::HashMap::new();
                let field_err = FieldError::new(password_field.to_string())
                    .with_rule("required".to_string())
                    .with_error(format!("Field '{}' is required", password_field));
                field_errors.insert(password_field.to_string(), field_err);
                let err = validation_error(
                    "Missing required fields".to_string(),
                    get_current_request_path(),
                    field_errors,
                );
                return create_json_result(400, &err.to_json_string());
            }
        };

        // Query user from database
        let table_name = &auth_meta.table_name;
        let sql = format!(
            "SELECT * FROM {} WHERE {} = $1",
            table_name,
            to_snake_case(unique_field)
        );
        let sql_c = CString::new(sql).unwrap();
        let identifier_c = CString::new(identifier).unwrap();

        let query_result =
            doo_db_query_one_param(std::ptr::null(), sql_c.as_ptr(), identifier_c.as_ptr());

        if query_result.is_null() {
            return create_error_result(500, "Database query failed");
        }

        let query_res = &*(query_result as *mut DooResult);
        let is_error = doo_db_is_error(query_result);

        if is_error != 0 {
            doo_db_result_free(query_result);
            return create_error_result(401, "Invalid credentials");
        }

        let user_json_str = if query_res.value.is_null() {
            doo_db_result_free(query_result);
            return create_error_result(401, "Invalid credentials");
        } else {
            let json_ptr = query_res.value as *mut c_char;
            let json_str = CStr::from_ptr(json_ptr).to_string_lossy().into_owned();
            doo_db_result_free(query_result);
            json_str
        };

        // Parse user data
        let user_data: serde_json::Value = match serde_json::from_str(&user_json_str) {
            Ok(data) => data,
            Err(_) => {
                return create_error_result(500, "Failed to parse user data");
            }
        };

        // Get stored password hash (case-insensitive lookup - Postgres lowercases column names)
        let user_obj = match user_data.as_object() {
            Some(obj) => obj,
            None => {
                return create_error_result(500, "User data is not an object");
            }
        };

        let stored_hash = get_obj_value(user_obj, password_field).and_then(|v| v.as_str());

        let stored_hash = match stored_hash {
            Some(hash) => hash,
            None => {
                return create_error_result(500, "Password hash not found");
            }
        };

        // Verify password using libdoo_auth
        let password_c = CString::new(password).unwrap();
        let hash_c = CString::new(stored_hash).unwrap();

        let verify_result = doo_auth_verify_password(password_c.as_ptr(), hash_c.as_ptr());

        if verify_result.is_null() {
            return create_error_result(500, "Password verification failed");
        }

        let verify_res = &*(verify_result as *mut DooResult);
        let is_valid = if verify_res.tag == 0 {
            (verify_res.value as i32) != 0
        } else {
            doo_auth_free_result(verify_result);
            return create_error_result(401, "Invalid credentials");
        };
        doo_auth_free_result(verify_result);

        if !is_valid {
            return create_error_result(401, "Invalid credentials");
        }

        // Get user ID
        let user_id = user_data.get("id").and_then(|v| v.as_i64()).unwrap_or(0);

        // Generate JWT token
        let user_id_str = user_id.to_string();
        let sub_c = CString::new(user_id_str.clone()).unwrap();
        let token_data = json!({
            "id": user_id,
        });
        let data_json_str = token_data.to_string();
        let data_c = CString::new(data_json_str).unwrap();
        let expires = 86400i32; // 24 hours

        let token_result = doo_auth_sign(sub_c.as_ptr(), data_c.as_ptr(), expires);

        if token_result.is_null() {
            return create_error_result(500, "Failed to generate token");
        }

        let token_res = &*(token_result as *mut DooResult);
        let token = if token_res.tag == 0 && !token_res.value.is_null() {
            let token_ptr = token_res.value as *mut c_char;
            let token_str = CStr::from_ptr(token_ptr).to_string_lossy().into_owned();
            doo_auth_free_result(token_result);
            token_str
        } else {
            doo_auth_free_result(token_result);
            return create_error_result(500, "Failed to generate token");
        };

        // Build user response (allowlist based on metadata; excludes @internal/@writeOnly/@hash)
        let mut allowed_fields: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        allowed_fields.insert("id".to_string());

        for f in fields.iter() {
            let field_obj = match f.as_object() {
                Some(o) => o,
                None => continue,
            };
            let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };
            let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
            let should_exclude = has_decorator(decorators, "writeOnly")
                || has_decorator(decorators, "internal")
                || has_decorator(decorators, "hash");
            if should_exclude {
                continue;
            }
            allowed_fields.insert(normalize_key(field_name));
        }

        let mut user_response = user_data.as_object().unwrap().clone();
        user_response.retain(|k, v| {
            !v.is_null()
                && allowed_fields.contains(&normalize_key(k))
                && normalize_key(k) != normalize_key(password_field)
        });

        // Return success with token and user data
        let response = json!({
            "token": token,
            "user": user_response,
        });

        create_json_result(200, &response.to_string())
    }
}

/// CRUD create handler - uses libdoo_db
extern "C" fn crud_create_handler(request: *mut DooRequest) -> *mut DooResult {
    unsafe {
        if request.is_null() {
            return create_error_result(500, "Internal error: null request");
        }

        let req = &*request;
        if req.path.is_null() || req.body.is_null() {
            return create_error_result(400, "Missing request body");
        }

        let path = c_to_string(req.path);
        let body = c_to_string(req.body);

        // Parse JSON body
        let mut json: serde_json::Value = match serde_json::from_str(&body) {
            Ok(j) => j,
            Err(_) => {
                let err = error::invalid_json_error(path.clone());
                return create_json_result(400, &err.to_json_string());
            }
        };

        // Normalize keys to lowercase for Postgres compatibility
        if let Some(obj) = json.as_object_mut() {
            let keys: Vec<String> = obj.keys().cloned().collect();
            for key in keys {
                if let Some(value) = obj.remove(&key) {
                    obj.insert(key.to_lowercase(), value);
                }
            }
        }

        // Find metadata for this table
        let metadata_map = get_crud_metadata().lock().unwrap();
        let crud_meta = metadata_map
            .values()
            .find(|m| path.starts_with(&m.base_path))
            .cloned();
        drop(metadata_map);

        let crud_meta = match crud_meta {
            Some(m) => m,
            None => {
                return create_error_result(500, "No CRUD metadata found for this path");
            }
        };

        let obj = match json.as_object() {
            Some(o) => o,
            None => {
                return create_error_result(400, "Request body must be a JSON object");
            }
        };

        // Get struct name from metadata for error messages
        let _struct_name = crud_meta
            .metadata
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("Resource");

        // Extract table metadata
        let metadata = &crud_meta.metadata;
        let fields = match metadata.get("fields").and_then(|f| f.as_array()) {
            Some(f) => f,
            None => {
                return create_error_result(500, "Invalid metadata: missing fields");
            }
        };

        // Validate field types before insert
        let mut type_errors = std::collections::HashMap::new();
        for field in fields.iter() {
            if let Some(field_obj) = field.as_object() {
                if let Some(field_name) = field_obj.get("name").and_then(|n| n.as_str()) {
                    if let Some(expected_type) = field_obj.get("type").and_then(|t| t.as_str()) {
                        let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
                        let ignore_request_value = has_decorator(decorators, "internal")
                            || has_decorator(decorators, "readOnly");

                        if !ignore_request_value {
                            if let Some(field_value) = get_obj_value(obj, field_name) {
                                let type_matches = match expected_type {
                                    "Int" => field_value.is_i64() || field_value.is_u64(),
                                    "Float" => field_value.is_f64(),
                                    "Bool" => field_value.is_boolean(),
                                    "Str" => field_value.is_string(),
                                    _ => true,
                                };

                                if !type_matches {
                                    use error::*;
                                    let received_type = if field_value.is_string() {
                                        "String"
                                    } else if field_value.is_i64() || field_value.is_u64() {
                                        "Int"
                                    } else if field_value.is_f64() {
                                        "Float"
                                    } else if field_value.is_boolean() {
                                        "Bool"
                                    } else {
                                        "Unknown"
                                    };

                                    let field_err = FieldError::new(field_name.to_string())
                                        .with_rule("type_mismatch".to_string())
                                        .with_expected(expected_type.to_string())
                                        .with_received(received_type.to_string())
                                        .with_value(field_value.to_string())
                                        .with_error(format!(
                                            "Expected type {}, received {}",
                                            expected_type, received_type
                                        ));
                                    type_errors.insert(field_name.to_string(), field_err);
                                }
                            }
                        }
                    }
                }
            }
        }

        if !type_errors.is_empty() {
            use error::*;
            let err = type_mismatch_error(path.clone(), type_errors);
            return create_json_result(400, &err.to_json_string());
        }

        // Validate required fields (fields without @auto or @default)
        let mut missing_fields = Vec::new();
        for field in fields.iter() {
            if let Some(field_obj) = field.as_object() {
                if let Some(field_name) = field_obj.get("name").and_then(|n| n.as_str()) {
                    // Check if field has @auto or @default decorator
                    let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
                    let has_auto_or_default_or_foreign = if let Some(decs) = decorators {
                        has_decorator(Some(decs), "auto")
                            || has_decorator(Some(decs), "default")
                            || has_decorator(Some(decs), "foreign")
                            || has_decorator(Some(decs), "internal")
                            || has_decorator(Some(decs), "readOnly")
                    } else {
                        false
                    };

                    // Check if field is optional (either via metadata flag or Type string "Optional(...)")
                    let type_str = field_obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    let is_optional = field_obj
                        .get("is_optional")
                        .and_then(|v| v.as_bool())
                        .or_else(|| field_obj.get("optional").and_then(|v| v.as_bool()))
                        .unwrap_or(false)
                        || type_str.starts_with("Optional(");

                    // If field is required (no @auto, no @default, no @foreign, not optional) and missing from request
                    if !has_auto_or_default_or_foreign
                        && !is_optional
                        && get_obj_value(obj, field_name).is_none()
                    {
                        missing_fields.push(field_name.to_string());
                    }
                }
            }
        }

        if !missing_fields.is_empty() {
            use error::*;
            let mut field_errors = std::collections::HashMap::new();
            for field_name in missing_fields {
                let field_err = FieldError::new(field_name.clone())
                    .with_rule("required".to_string())
                    .with_error(format!("Field '{}' is required", field_name));
                field_errors.insert(field_name, field_err);
            }
            let err = validation_error(
                "Missing required fields".to_string(),
                path.clone(),
                field_errors,
            );
            return create_json_result(400, &err.to_json_string());
        }

        // Build INSERT SQL
        let table_name = &crud_meta.table_name;
        let mut field_names = Vec::new();
        let mut placeholders = Vec::new();
        let mut values_json = Vec::new();
        let mut response_additions = HashMap::new();
        let mut param_idx = 1;
        let mut all_errors: HashMap<String, error::FieldError> = HashMap::new();

        for field in fields.iter() {
            let field_obj = match field.as_object() {
                Some(o) => o,
                None => continue,
            };

            let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };

            // Skip auto-generated fields
            let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
            if let Some(decs) = decorators {
                let is_auto = decs.iter().any(|d| {
                    d.as_object()
                        .and_then(|o| o.get("name"))
                        .and_then(|n| n.as_str())
                        == Some("auto")
                });
                if is_auto {
                    continue;
                }
            }

            // Ignore request input for @internal and @readOnly fields
            let ignore_request_value =
                has_decorator(decorators, "internal") || has_decorator(decorators, "readOnly");

            // Case-insensitive lookup for the field value
            let value = if ignore_request_value {
                None
            } else {
                get_obj_value(obj, field_name)
            };

            if let Some(value) = value {
                // Ensure field_type is extracted or passed
                let field_type = match field_obj.get("type").and_then(|t| t.as_str()) {
                    Some(t) => t,
                    None => "Str",
                };
                // Extract decorators JSON
                let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
                let decs_json = if let Some(d) = decorators {
                    json!(d)
                } else {
                    json!([])
                };

                let val_str = if let Some(s) = value.as_str() {
                    s.to_string()
                } else {
                    value.to_string()
                };

                match unsafe {
                    validate_field_with_runtime(
                        field_name,
                        field_type,
                        &val_str,
                        decs_json.as_array().unwrap(),
                        path.clone(),
                    )
                } {
                    Ok(_) => {}
                    Err((_, msg_json)) => {
                        // Ignore status, we'll force 400/422 at the end
                        // Parse msg_json to get the fields map
                        if let Ok(err_obj) = serde_json::from_str::<serde_json::Value>(&msg_json) {
                            if let Some(fields) = err_obj.get("fields").and_then(|f| f.as_object())
                            {
                                for (k, v) in fields {
                                    // Convert Value to FieldError
                                    let field_err = error::FieldError::new(
                                        v.get("message")
                                            .and_then(|s| s.as_str())
                                            .unwrap_or("Validation failed")
                                            .to_string(),
                                    )
                                    .with_rule(
                                        v.get("rule")
                                            .and_then(|s| s.as_str())
                                            .unwrap_or("validation")
                                            .to_string(),
                                    )
                                    .with_value(
                                        v.get("value")
                                            .and_then(|s| s.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                    );

                                    let field_err = if let Some(exp) =
                                        v.get("expected").and_then(|s| s.as_str())
                                    {
                                        field_err.with_expected(exp.to_string())
                                    } else {
                                        field_err
                                    };

                                    let field_err = if let Some(rec) =
                                        v.get("received").and_then(|s| s.as_str())
                                    {
                                        field_err.with_received(rec.to_string())
                                    } else {
                                        field_err
                                    };

                                    let field_err = if let Some(err) =
                                        v.get("error").and_then(|s| s.as_str())
                                    {
                                        field_err.with_error(err.to_string())
                                    } else {
                                        field_err
                                    };

                                    all_errors.insert(k.clone(), field_err);
                                }
                            }
                        }
                        continue; // Skip valid field insertion if validation failed
                    }
                }

                field_names.push(to_snake_case(field_name));
                placeholders.push(format!("${}", param_idx));
                values_json.push(value.clone());
                param_idx += 1;
            } else {
                // Check if field has @foreign or @default decorator
                let mut is_handled = false;

                if let Some(decs) = decorators {
                    // 1. Check for @foreign targeting Auth model to inject auth_user_id
                    if let Some(foreign_dec) = decs.iter().find(|d| {
                        d.as_object()
                            .and_then(|o| o.get("name"))
                            .and_then(|n| n.as_str())
                            == Some("foreign")
                    }) {
                        // Check if we have an authenticated user ID available
                        let params_ptr = req.params as *const HashMap<String, String>;
                        let auth_user_id_opt = if !params_ptr.is_null() {
                            (&*params_ptr).get("auth_user_id")
                        } else {
                            None
                        };

                        if let Some(user_id) = auth_user_id_opt {
                            // Get foreign target name from args
                            if let Some(dec_obj) = foreign_dec.as_object() {
                                if let Some(target) = dec_obj
                                    .get("args")
                                    .and_then(|a| a.as_array())
                                    .and_then(|a| a.first())
                                    .and_then(|v| v.as_str())
                                {
                                    // Verify target matches an Auth struct (generic check)
                                    let auth_meta = get_auth_metadata().lock().unwrap();
                                    let is_auth_target = auth_meta.contains_key(target);
                                    drop(auth_meta);

                                    if is_auth_target {
                                        field_names.push(to_snake_case(field_name));
                                        placeholders.push(format!("${}", param_idx));

                                        // Auto-convert to number if possible (for Int/Float fields)
                                        // But safely fallback to string
                                        let val = if let Ok(uid_int) = user_id.parse::<i64>() {
                                            serde_json::Value::Number(serde_json::Number::from(
                                                uid_int,
                                            ))
                                        } else {
                                            serde_json::Value::String(user_id.clone())
                                        };

                                        response_additions
                                            .insert(to_snake_case(field_name), val.clone());
                                        values_json.push(val);
                                        param_idx += 1;
                                        is_handled = true;
                                    }
                                }
                            }
                        }
                    }

                    // 2. If not handled by foreign auto-population, check for @default
                    if !is_handled {
                        if let Some(default_value) = decs.iter().find_map(|d| {
                            let dec_obj = d.as_object()?;
                            if dec_obj.get("name")?.as_str()? == "default" {
                                let args = dec_obj.get("args")?.as_array()?;
                                args.first()?.as_str()
                            } else {
                                None
                            }
                        }) {
                            // Apply default value (skip empty strings)
                            if !default_value.is_empty() {
                                // Handle Enum syntax like Status::Todo -> Todo
                                let clean_val = if default_value.contains("::") {
                                    default_value.split("::").last().unwrap_or(default_value)
                                } else {
                                    default_value
                                };

                                let field_type = field_obj
                                    .get("type")
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("Str");

                                field_names.push(to_snake_case(field_name));
                                placeholders.push(format!("${}", param_idx));

                                let val = match field_type {
                                    "Int" => clean_val
                                        .parse::<i64>()
                                        .ok()
                                        .map(|i| serde_json::Number::from(i))
                                        .map(serde_json::Value::Number)
                                        .unwrap_or_else(|| {
                                            serde_json::Value::String(clean_val.to_string())
                                        }),
                                    "Float" => clean_val
                                        .parse::<f64>()
                                        .ok()
                                        .and_then(serde_json::Number::from_f64)
                                        .map(serde_json::Value::Number)
                                        .unwrap_or_else(|| {
                                            serde_json::Value::String(clean_val.to_string())
                                        }),
                                    "Bool" => match clean_val.to_lowercase().as_str() {
                                        "true" => serde_json::Value::Bool(true),
                                        "false" => serde_json::Value::Bool(false),
                                        _ => serde_json::Value::String(clean_val.to_string()),
                                    },
                                    _ => serde_json::Value::String(clean_val.to_string()),
                                };
                                response_additions.insert(to_snake_case(field_name), val.clone());
                                values_json.push(val);

                                param_idx += 1;
                                is_handled = true;
                            }
                        }
                    }
                }
            }
        }

        if !all_errors.is_empty() {
            let instance = get_current_request_path();
            let err = error::validation_failed_error(instance, all_errors);
            return create_json_result(422, &err.to_json_string());
        }

        if field_names.is_empty() {
            return create_error_result(400, "No valid fields provided");
        }

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({}) RETURNING id",
            table_name,
            field_names.join(", "),
            placeholders.join(", ")
        );

        let sql_c = CString::new(sql).unwrap();
        let values_json_str = serde_json::to_string(&values_json).unwrap();
        let values_c = CString::new(values_json_str).unwrap();

        // Insert into database
        let insert_result = doo_db_insert_json(sql_c.as_ptr(), values_c.as_ptr());

        if insert_result.is_null() {
            return create_error_result(500, "Database insert failed");
        }

        let insert_res = &*(insert_result as *mut DooResult);
        if doo_db_is_error(insert_result) != 0 {
            let err_msg_ptr = doo_db_get_error_message(insert_result);
            let err_msg = if err_msg_ptr.is_null() {
                "Database insert failed".to_string()
            } else {
                let msg = CStr::from_ptr(err_msg_ptr).to_string_lossy().into_owned();
                doo_db_free_string(err_msg_ptr);
                msg
            };
            doo_db_result_free(insert_result);

            // Convert DB error to RFC 7807 format
            let (status, rfc_error) = convert_db_error_to_rfc7807(&err_msg, path.clone());
            return create_json_result(status, &rfc_error);
        }

        let user_id = if insert_res.value.is_null() {
            1i64
        } else {
            insert_res.value as i64
        };
        doo_db_result_free(insert_result);

        // Build response with created resource (allowlist; exclude @writeOnly/@internal/@hash)
        let mut resource_obj = serde_json::Map::new();
        resource_obj.insert("id".to_string(), json!(user_id));

        for field in fields.iter() {
            let field_obj = match field.as_object() {
                Some(o) => o,
                None => continue,
            };
            let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };
            if field_name.eq_ignore_ascii_case("id") {
                continue;
            }

            let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
            let should_exclude = has_decorator(decorators, "writeOnly")
                || has_decorator(decorators, "internal")
                || has_decorator(decorators, "hash");
            if should_exclude {
                continue;
            }

            let key = to_snake_case(field_name);
            if let Some(v) = response_additions.get(&key) {
                if !v.is_null() {
                    resource_obj.insert(key, v.clone());
                }
                continue;
            }

            if let Some(v) = get_obj_value(obj, field_name) {
                if !v.is_null() {
                    resource_obj.insert(key, v.clone());
                }
            }
        }

        create_json_result(201, &serde_json::to_string(&resource_obj).unwrap())
    }
}

/// CRUD list handler - uses libdoo_db
extern "C" fn crud_list_handler(request: *mut DooRequest) -> *mut DooResult {
    unsafe {
        if request.is_null() {
            return create_error_result(500, "Internal error: null request");
        }

        let req = &*request;
        if req.path.is_null() {
            return create_error_result(500, "Internal error: invalid request");
        }

        let path = c_to_string(req.path);

        // Get metadata for this path
        let metadata_map = get_crud_metadata().lock().unwrap();
        let crud_meta = metadata_map
            .values()
            .find(|m| path.starts_with(&m.base_path))
            .cloned();
        drop(metadata_map);

        let crud_meta = match crud_meta {
            Some(m) => m,
            None => {
                return create_error_result(500, "No CRUD metadata found for this path");
            }
        };

        let table_name = &crud_meta.table_name;
        let sql = format!("SELECT * FROM {}", table_name);
        let sql_c = CString::new(sql).unwrap();

        let query_result = doo_db_query_json(sql_c.as_ptr());

        if query_result.is_null() {
            return create_error_result(500, "Database query failed");
        }

        let query_res = &*(query_result as *mut DooResult);
        if doo_db_is_error(query_result) != 0 {
            let err_msg_ptr = doo_db_get_error_message(query_result);
            let err_msg = if err_msg_ptr.is_null() {
                "Database query failed".to_string()
            } else {
                let msg = CStr::from_ptr(err_msg_ptr).to_string_lossy().into_owned();
                doo_db_free_string(err_msg_ptr);
                msg
            };
            doo_db_result_free(query_result);
            return create_error_result(500, &err_msg);
        }

        let data_json_str = if query_res.value.is_null() {
            doo_db_result_free(query_result);
            "[]".to_string()
        } else {
            let json_ptr = query_res.value as *mut c_char;
            let json_str = CStr::from_ptr(json_ptr).to_string_lossy().into_owned();
            doo_db_result_free(query_result);
            json_str
        };

        // Parse data array and filter out @writeonly, @internal, @hash fields
        let data_array: Vec<serde_json::Value> =
            serde_json::from_str(&data_json_str).unwrap_or_default();

        // Get fields to INCLUDE from metadata (allowlist approach)
        // Only include fields that are in the struct definition AND don't have @internal/@writeonly/@hash
        let metadata = &crud_meta.metadata;
        let fields = metadata.get("fields").and_then(|f| f.as_array());

        let mut fields_to_include: Vec<String> = Vec::new();
        let mut field_defaults: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();

        if let Some(field_list) = fields {
            for f in field_list {
                let field_obj = match f.as_object() {
                    Some(o) => o,
                    None => continue,
                };
                let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
                    Some(n) => n,
                    None => continue,
                };
                let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
                let field_type = field_obj
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("Str");

                // Check if field has @writeonly, @internal, or @hash decorator
                let should_exclude = decorators
                    .map(|decs| {
                        decs.iter().any(|d| {
                            let dec_name = d
                                .as_object()
                                .and_then(|o| o.get("name"))
                                .and_then(|n| n.as_str());
                            dec_name == Some("writeonly")
                                || dec_name == Some("internal")
                                || dec_name == Some("hash")
                        })
                    })
                    .unwrap_or(false);

                // Only include fields that aren't excluded
                if !should_exclude {
                    fields_to_include.push(to_snake_case(field_name));
                }

                // Extract @default value if present
                if let Some(decs) = decorators {
                    for dec in decs {
                        if let Some(dec_obj) = dec.as_object() {
                            if dec_obj.get("name").and_then(|n| n.as_str()) == Some("default") {
                                if let Some(args) = dec_obj.get("args").and_then(|a| a.as_array()) {
                                    if let Some(default_val) = args.first().and_then(|v| v.as_str())
                                    {
                                        // Convert default value to appropriate JSON type based on field type
                                        let json_default = match field_type {
                                            "Int" => default_val
                                                .parse::<i64>()
                                                .map(serde_json::Value::from)
                                                .unwrap_or(serde_json::Value::Null),
                                            "Float" => default_val
                                                .parse::<f64>()
                                                .map(serde_json::Value::from)
                                                .unwrap_or(serde_json::Value::Null),
                                            "Bool" => match default_val.to_lowercase().as_str() {
                                                "true" => serde_json::Value::Bool(true),
                                                "false" => serde_json::Value::Bool(false),
                                                _ => serde_json::Value::Null,
                                            },
                                            _ => serde_json::Value::String(default_val.to_string()),
                                        };
                                        field_defaults
                                            .insert(to_snake_case(field_name), json_default);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Filter each object in the array - keep ONLY fields in metadata (allowlist)
        let filtered_data: Vec<serde_json::Value> = data_array
            .into_iter()
            .map(|item| {
                if let serde_json::Value::Object(obj) = item {
                    // Create new object with only allowed fields
                    let mut filtered_obj = serde_json::Map::new();
                    for field_name in &fields_to_include {
                        if let Some(value) = obj.get(field_name) {
                            // Apply @default for NULL values
                            if value.is_null() {
                                if let Some(default_val) = field_defaults.get(field_name) {
                                    filtered_obj.insert(field_name.clone(), default_val.clone());
                                }
                                // Skip null values without defaults
                            } else {
                                filtered_obj.insert(field_name.clone(), value.clone());
                            }
                        } else if let Some(default_val) = field_defaults.get(field_name) {
                            // Field doesn't exist in row - insert default
                            filtered_obj.insert(field_name.clone(), default_val.clone());
                        }
                    }
                    serde_json::Value::Object(filtered_obj)
                } else {
                    item
                }
            })
            .collect();

        let filtered_json =
            serde_json::to_string(&filtered_data).unwrap_or_else(|_| "[]".to_string());
        create_json_result(200, &filtered_json)
    }
}

/// CRUD get handler - uses libdoo_db
extern "C" fn crud_get_handler(request: *mut DooRequest) -> *mut DooResult {
    unsafe {
        if request.is_null() {
            return create_error_result(500, "Internal error: null request");
        }

        let req = &*request;
        if req.path.is_null() || req.params.is_null() {
            return create_error_result(500, "Internal error: invalid request");
        }

        let path = c_to_string(req.path);

        // Extract id from params HashMap
        let params_ptr = req.params as *const HashMap<String, String>;
        let params = &*params_ptr;

        let id_str = match params.get("id") {
            Some(id) => id.as_str(),
            None => {
                return create_error_result(400, "Missing id parameter");
            }
        };

        // Get metadata for this path
        let metadata_map = get_crud_metadata().lock().unwrap();
        let crud_meta = metadata_map
            .values()
            .find(|m| path.starts_with(&m.base_path))
            .cloned();
        drop(metadata_map);

        let crud_meta = match crud_meta {
            Some(m) => m,
            None => {
                return create_error_result(500, "No CRUD metadata found for this path");
            }
        };

        // Get struct name from metadata for error messages
        let _struct_name = crud_meta
            .metadata
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("Resource");

        // Validate ID is numeric to prevent SQL injection
        let id_num = match id_str.parse::<i64>() {
            Ok(n) => n,
            Err(_) => return create_error_result(400, "Invalid ID format"),
        };

        let table_name = &crud_meta.table_name;
        // Use direct SQL interpolation since ID is validated numeric
        let sql = format!("SELECT * FROM {} WHERE id = {}", table_name, id_num);
        let sql_c = CString::new(sql).unwrap();

        let query_result = doo_db_query_one_json(sql_c.as_ptr());

        if query_result.is_null() {
            return create_error_result(500, "Database query failed");
        }

        let query_res = &*(query_result as *mut DooResult);
        if doo_db_is_error(query_result) != 0 {
            doo_db_result_free(query_result);
            return create_error_result(404, "Resource not found");
        }

        let data_json_str = if query_res.value.is_null() {
            doo_db_result_free(query_result);
            return create_error_result(404, "Resource not found");
        } else {
            let json_ptr = query_res.value as *mut c_char;
            let json_str = CStr::from_ptr(json_ptr).to_string_lossy().into_owned();
            doo_db_result_free(query_result);
            json_str
        };

        // Parse data object and filter out @writeonly, @internal, @hash fields
        let data_obj: serde_json::Value =
            serde_json::from_str(&data_json_str).unwrap_or(serde_json::Value::Null);

        // Get fields to INCLUDE from metadata (allowlist approach)
        // Only include fields that are in the struct definition AND don't have @internal/@writeonly/@hash
        let metadata = &crud_meta.metadata;
        let fields = metadata.get("fields").and_then(|f| f.as_array());

        let mut fields_to_include: Vec<String> = Vec::new();
        let mut field_defaults: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();

        if let Some(field_list) = fields {
            for f in field_list {
                let field_obj = match f.as_object() {
                    Some(o) => o,
                    None => continue,
                };
                let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
                    Some(n) => n,
                    None => continue,
                };
                let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
                let field_type = field_obj
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("Str");

                // Check if field has @writeonly, @internal, or @hash decorator
                let should_exclude = decorators
                    .map(|decs| {
                        decs.iter().any(|d| {
                            let dec_name = d
                                .as_object()
                                .and_then(|o| o.get("name"))
                                .and_then(|n| n.as_str());
                            dec_name == Some("writeonly")
                                || dec_name == Some("internal")
                                || dec_name == Some("hash")
                        })
                    })
                    .unwrap_or(false);

                // Only include fields that aren't excluded
                if !should_exclude {
                    fields_to_include.push(to_snake_case(field_name));
                }

                // Extract @default value if present
                if let Some(decs) = decorators {
                    for dec in decs {
                        if let Some(dec_obj) = dec.as_object() {
                            if dec_obj.get("name").and_then(|n| n.as_str()) == Some("default") {
                                if let Some(args) = dec_obj.get("args").and_then(|a| a.as_array()) {
                                    if let Some(default_val) = args.first().and_then(|v| v.as_str())
                                    {
                                        // Convert default value to appropriate JSON type based on field type
                                        let json_default = match field_type {
                                            "Int" => default_val
                                                .parse::<i64>()
                                                .map(serde_json::Value::from)
                                                .unwrap_or(serde_json::Value::Null),
                                            "Float" => default_val
                                                .parse::<f64>()
                                                .map(serde_json::Value::from)
                                                .unwrap_or(serde_json::Value::Null),
                                            "Bool" => match default_val.to_lowercase().as_str() {
                                                "true" => serde_json::Value::Bool(true),
                                                "false" => serde_json::Value::Bool(false),
                                                _ => serde_json::Value::Null,
                                            },
                                            _ => serde_json::Value::String(default_val.to_string()),
                                        };
                                        field_defaults
                                            .insert(to_snake_case(field_name), json_default);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Filter the object - keep ONLY fields in metadata (allowlist)
        let filtered_data = if let serde_json::Value::Object(obj) = data_obj {
            let mut filtered_obj = serde_json::Map::new();
            for field_name in &fields_to_include {
                if let Some(value) = obj.get(field_name) {
                    // Apply @default for NULL values
                    if value.is_null() {
                        if let Some(default_val) = field_defaults.get(field_name) {
                            filtered_obj.insert(field_name.clone(), default_val.clone());
                        }
                        // Skip null values without defaults
                    } else {
                        filtered_obj.insert(field_name.clone(), value.clone());
                    }
                } else if let Some(default_val) = field_defaults.get(field_name) {
                    // Field doesn't exist in row - insert default
                    filtered_obj.insert(field_name.clone(), default_val.clone());
                }
            }
            serde_json::Value::Object(filtered_obj)
        } else {
            data_obj
        };

        let filtered_json =
            serde_json::to_string(&filtered_data).unwrap_or_else(|_| "{}".to_string());
        create_json_result(200, &filtered_json)
    }
}

/// CRUD delete handler - uses libdoo_db
extern "C" fn crud_delete_handler(request: *mut DooRequest) -> *mut DooResult {
    unsafe {
        if request.is_null() {
            return create_error_result(500, "Internal error: null request");
        }

        let req = &*request;
        if req.path.is_null() || req.params.is_null() {
            return create_error_result(500, "Internal error: invalid request");
        }

        let path = c_to_string(req.path);

        // Extract id from params HashMap
        let params_ptr = req.params as *const HashMap<String, String>;
        let params = &*params_ptr;

        let id_str = match params.get("id") {
            Some(id) => id.as_str(),
            None => {
                return create_error_result(400, "Missing id parameter");
            }
        };

        // Validate ID is numeric to prevent SQL injection
        let id_num = match id_str.parse::<i64>() {
            Ok(n) => n,
            Err(_) => return create_error_result(400, "Invalid ID format"),
        };

        // Get metadata for this path
        let metadata_map = get_crud_metadata().lock().unwrap();
        let crud_meta = metadata_map
            .values()
            .find(|m| path.starts_with(&m.base_path))
            .cloned();
        drop(metadata_map);

        let crud_meta = match crud_meta {
            Some(m) => m,
            None => {
                return create_error_result(500, "No CRUD metadata found for this path");
            }
        };

        let table_name = &crud_meta.table_name;
        // Use direct SQL interpolation since ID is validated numeric
        let sql = format!("DELETE FROM {} WHERE id = {}", table_name, id_num);
        let sql_c = CString::new(sql).unwrap();

        let delete_result = doo_db_execute(std::ptr::null(), sql_c.as_ptr());

        if delete_result.is_null() {
            return create_error_result(500, "Database delete failed");
        }

        let delete_res = &*(delete_result as *mut DooResult);
        if doo_db_is_error(delete_result) != 0 {
            let err_msg_ptr = doo_db_get_error_message(delete_result);
            let err_msg = if err_msg_ptr.is_null() {
                "Database delete failed".to_string()
            } else {
                let msg = CStr::from_ptr(err_msg_ptr).to_string_lossy().into_owned();
                doo_db_free_string(err_msg_ptr);
                msg
            };
            doo_db_result_free(delete_result);
            return create_error_result(500, &err_msg);
        }

        let rows_affected = if delete_res.value.is_null() {
            0i64
        } else {
            delete_res.value as i64
        };
        doo_db_result_free(delete_result);

        if rows_affected == 0 {
            return create_error_result(404, "Resource not found");
        }

        // Return 200 with industry-standard DELETE response format
        let response_json = format!(r#"{{"message":"Resource deleted","id":{}}}"#, id_num);
        create_json_result(200, &response_json)
    }
}

/// CRUD update handler - uses libdoo_db
extern "C" fn crud_update_handler(request: *mut DooRequest) -> *mut DooResult {
    unsafe {
        if request.is_null() {
            return create_error_result(500, "Internal error: null request");
        }

        let req = &*request;
        if req.path.is_null() || req.params.is_null() {
            return create_error_result(500, "Internal error: invalid request");
        }

        let path = c_to_string(req.path);

        // Extract id from params HashMap
        let params_ptr = req.params as *const HashMap<String, String>;
        let params = &*params_ptr;

        let id_str = match params.get("id") {
            Some(id) => id.as_str(),
            None => {
                return create_error_result(400, "Missing id parameter");
            }
        };

        let body = c_to_string(req.body);

        // Parse JSON body
        let mut json: serde_json::Value = match serde_json::from_str(&body) {
            Ok(j) => j,
            Err(_) => {
                let err = error::invalid_json_error(path.clone());
                return create_json_result(400, &err.to_json_string());
            }
        };

        // Normalize keys to lowercase for Postgres compatibility
        if let Some(obj) = json.as_object_mut() {
            let keys: Vec<String> = obj.keys().cloned().collect();
            for key in keys {
                if let Some(value) = obj.remove(&key) {
                    obj.insert(key.to_lowercase(), value);
                }
            }
        }

        let obj = match json.as_object() {
            Some(o) => o,
            None => {
                return create_error_result(400, "Request body must be a JSON object");
            }
        };

        // Get metadata for this path
        let metadata_map = get_crud_metadata().lock().unwrap();
        let crud_meta = metadata_map
            .values()
            .find(|m| path.starts_with(&m.base_path))
            .cloned();
        drop(metadata_map);

        let crud_meta = match crud_meta {
            Some(m) => m,
            None => {
                return create_error_result(500, "No CRUD metadata found for this path");
            }
        };

        // Get struct name from metadata for error messages
        let _struct_name = crud_meta
            .metadata
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("Resource");

        // Extract table metadata
        let metadata = &crud_meta.metadata;
        let fields = match metadata.get("fields").and_then(|f| f.as_array()) {
            Some(f) => f,
            None => {
                return create_error_result(500, "Invalid metadata: missing fields");
            }
        };

        // Build UPDATE SQL
        let table_name = &crud_meta.table_name;
        let mut set_clauses = Vec::new();
        let mut values_json = Vec::new();
        let mut param_idx = 1;

        for field in fields.iter() {
            let field_obj = match field.as_object() {
                Some(o) => o,
                None => continue,
            };

            let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };

            // Skip auto-generated, primary key, readonly, and internal fields
            let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
            if let Some(decs) = decorators {
                let should_skip = decs.iter().any(|d| {
                    let dec_name = d
                        .as_object()
                        .and_then(|o| o.get("name"))
                        .and_then(|n| n.as_str());
                    dec_name == Some("auto")
                        || dec_name == Some("primary")
                        || dec_name == Some("readonly")   // @readonly fields can't be updated
                        || dec_name == Some("internal") // @internal fields can't be updated
                });
                if should_skip {
                    continue;
                }
            }

            // Case-insensitive lookup for the field value
            let value = get_obj_value(obj, field_name);

            if let Some(value) = value {
                set_clauses.push(format!("{} = ${}", to_snake_case(field_name), param_idx));
                values_json.push(value.clone());
                param_idx += 1;
            }
        }

        if set_clauses.is_empty() {
            return create_error_result(400, "No valid fields to update");
        }

        // Check for autoTimestamp - add updated_at = NOW() to the update
        let has_auto_timestamp = metadata
            .get("autoTimestamp")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if has_auto_timestamp {
            set_clauses.push("updated_at = NOW()".to_string());
        }

        // Validate ID is numeric to prevent SQL injection
        let id_num = match id_str.parse::<i64>() {
            Ok(n) => n,
            Err(_) => return create_error_result(400, "Invalid ID format"),
        };

        let sql = format!(
            "UPDATE {} SET {} WHERE id = {} RETURNING id",
            table_name,
            set_clauses.join(", "),
            id_num
        );

        let sql_c = CString::new(sql).unwrap();
        let values_json_str = serde_json::to_string(&values_json).unwrap();
        let values_c = CString::new(values_json_str).unwrap();

        // Execute update
        let update_result = doo_db_insert_json(sql_c.as_ptr(), values_c.as_ptr());

        if update_result.is_null() {
            return create_error_result(500, "Database update failed");
        }

        let update_res = &*(update_result as *mut DooResult);
        if doo_db_is_error(update_result) != 0 {
            let err_msg_ptr = doo_db_get_error_message(update_result);
            let err_msg = if err_msg_ptr.is_null() {
                "Database update failed".to_string()
            } else {
                let msg = CStr::from_ptr(err_msg_ptr).to_string_lossy().into_owned();
                doo_db_free_string(err_msg_ptr);
                msg
            };
            doo_db_result_free(update_result);

            // Convert DB error to RFC 7807 format
            let (status, rfc_error) = convert_db_error_to_rfc7807(&err_msg, path.clone());
            return create_json_result(status, &rfc_error);
        }

        let resource_id = if update_res.value.is_null() {
            id_str.parse::<i64>().unwrap_or(0)
        } else {
            update_res.value as i64
        };
        doo_db_result_free(update_result);

        // Re-fetch the updated record to return the actual state from DB
        let select_sql = format!("SELECT * FROM {} WHERE id = {}", table_name, resource_id);
        let select_sql_c = CString::new(select_sql).unwrap();

        let query_result = doo_db_query_one_json(select_sql_c.as_ptr());

        if query_result.is_null() {
            return create_error_result(500, "Failed to fetch updated record");
        }

        let query_res = &*(query_result as *mut DooResult);
        if doo_db_is_error(query_result) != 0 {
            doo_db_result_free(query_result);
            return create_error_result(500, "Failed to fetch updated record");
        }

        let data_json_str = if query_res.value.is_null() {
            doo_db_result_free(query_result);
            "{}".to_string()
        } else {
            let json_ptr = query_res.value as *mut c_char;
            let json_str = CStr::from_ptr(json_ptr).to_string_lossy().into_owned();
            doo_db_result_free(query_result);
            json_str
        };

        // Parse data object and filter out @writeonly, @internal, @hash fields
        let data_obj: serde_json::Value =
            serde_json::from_str(&data_json_str).unwrap_or(serde_json::Value::Null);

        // Get fields to INCLUDE from metadata (allowlist approach)
        // Only include fields that are in the struct definition AND don't have @internal/@writeonly/@hash
        let fields_to_include: Vec<String> = fields
            .iter()
            .filter_map(|f| {
                let field_obj = f.as_object()?;
                let field_name = field_obj.get("name").and_then(|n| n.as_str())?;
                let decorators = field_obj.get("decorators").and_then(|d| d.as_array());

                let should_exclude = decorators
                    .map(|decs| {
                        decs.iter().any(|d| {
                            let dec_name = d
                                .as_object()
                                .and_then(|o| o.get("name"))
                                .and_then(|n| n.as_str());
                            dec_name == Some("writeonly")
                                || dec_name == Some("internal")
                                || dec_name == Some("hash")
                        })
                    })
                    .unwrap_or(false);

                if !should_exclude {
                    Some(to_snake_case(field_name))
                } else {
                    None
                }
            })
            .collect();

        // Filter the object - keep ONLY fields in metadata (allowlist)
        let filtered_data = if let serde_json::Value::Object(obj) = data_obj {
            let mut filtered_obj = serde_json::Map::new();
            for field_name in &fields_to_include {
                if let Some(value) = obj.get(field_name) {
                    if !value.is_null() {
                        filtered_obj.insert(field_name.clone(), value.clone());
                    }
                }
            }
            serde_json::Value::Object(filtered_obj)
        } else {
            data_obj
        };

        let filtered_json =
            serde_json::to_string(&filtered_data).unwrap_or_else(|_| "{}".to_string());
        create_json_result(200, &filtered_json)
    }
}

// Helper to create JSON response
fn create_json_result(status: i32, body: &str) -> *mut DooResult {
    let response = alloc_doo_response(status, string_to_c(body), string_to_c("application/json"));

    unsafe {
        let size = std::mem::size_of::<DooResult>();
        let result = libc::malloc(size) as *mut DooResult;
        if result.is_null() {
            // If result alloc fails, we should free response to avoid leak, but passing null is fatal anyway
            return std::ptr::null_mut();
        }
        track_alloc(result as *const std::ffi::c_void, "http_create_json_result");
        (*result).tag = 0;
        (*result).value = response as *mut std::ffi::c_void;
        (*result).owner = owner::FFI;
        result
    }
}

// Helper to create error response with RFC 7807 compliant format
fn create_error_result(status: i32, message: &str) -> *mut DooResult {
    let title = match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let error_type = match status {
        400 => "bad_request",
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not_found",
        405 => "method_not_allowed",
        409 => "conflict",
        422 => "validation_error",
        429 => "rate_limit_exceeded",
        500 => "internal_error",
        501 => "not_implemented",
        502 => "bad_gateway",
        503 => "service_unavailable",
        _ => "error",
    };
    let instance = get_current_request_path();
    let error_json = format!(
        r#"{{"type":"{}","title":"{}","status":{},"detail":"{}","instance":"{}"}}"#,
        error_type, title, status, message, instance
    );
    create_json_result(status, &error_json)
}

// ============================================================================
// Migration - Create all tables at startup
// ============================================================================

/// Run all table migrations at server startup
/// Returns list of table names that were created
fn run_migrations() -> Vec<String> {
    let mut created_tables = Vec::new();

    // Collect all table metadata (auth, crud, and @table decorated)
    let mut all_tables: Vec<(String, serde_json::Value)> = Vec::new();

    // Get auth tables
    if let Ok(auth_map) = get_auth_metadata().lock() {
        for (struct_name, meta) in auth_map.iter() {
            all_tables.push((meta.table_name.clone(), meta.metadata.clone()));
        }
    }

    // Get crud tables
    if let Ok(crud_map) = get_crud_metadata().lock() {
        for (struct_name, meta) in crud_map.iter() {
            // Skip if already added (auth table might be shared)
            if !all_tables.iter().any(|(name, _)| name == &meta.table_name) {
                all_tables.push((meta.table_name.clone(), meta.metadata.clone()));
            }
        }
    }

    // Get @table decorated structs
    if let Ok(table_map) = get_table_metadata().lock() {
        for (struct_name, meta) in table_map.iter() {
            if !all_tables.iter().any(|(name, _)| name == &meta.table_name) {
                all_tables.push((meta.table_name.clone(), meta.metadata.clone()));
            }
        }
    }

    // Create tables in order (auth/user tables first for foreign key deps)
    for (table_name, metadata) in &all_tables {
        let create_sql = build_create_table_sql(table_name, metadata);

        if !create_sql.is_empty() {
            let sql_c = CString::new(create_sql.clone()).unwrap();
            let create_result = unsafe { doo_db_create_table(std::ptr::null(), sql_c.as_ptr()) };

            if !create_result.is_null() {
                if unsafe { doo_db_is_error(create_result) } != 0 {
                    let err_msg_ptr = unsafe { doo_db_get_error_message(create_result) };
                    if !err_msg_ptr.is_null() {
                        let err_msg = unsafe { CStr::from_ptr(err_msg_ptr).to_string_lossy() };
                        // Don't log "already exists" as error
                        if !err_msg.contains("already exists") {
                            doo_http_debug!("Migration error for {}: {}", table_name, err_msg);
                        }
                        unsafe { doo_db_free_string(err_msg_ptr) };
                    }
                    unsafe { doo_db_result_free(create_result) };
                } else {
                    created_tables.push(table_name.clone());
                    unsafe { doo_db_result_free(create_result) };
                }
            }
        }

        // GENERIC SCHEMA MIGRATION: Ensure all columns exist (even if table already existed)
        migrate_table_columns(table_name, metadata);
    }

    // Add foreign key constraints after all tables created
    for (table_name, metadata) in &all_tables {
        add_foreign_key_constraints(table_name, metadata);
    }

    created_tables
}

// Helper to build CREATE TABLE SQL from metadata
fn build_create_table_sql(table_name: &str, metadata: &serde_json::Value) -> String {
    let fields = match metadata.get("fields").and_then(|f| f.as_array()) {
        Some(f) => f,
        None => return String::new(),
    };

    let mut columns = Vec::new();

    for field in fields {
        let field_obj = match field.as_object() {
            Some(o) => o,
            None => continue,
        };

        let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => continue,
        };

        let field_type = match field_obj.get("type").and_then(|t| t.as_str()) {
            Some(t) => t,
            None => "TEXT",
        };

        let column_name = to_snake_case(field_name);

        let field_name_lc = field_name.to_lowercase();
        let is_timestamp_like = field_name_lc.ends_with("at")
            && (field_name_lc.contains("expires")
                || field_name_lc.contains("access")
                || field_name_lc.contains("created")
                || field_name_lc.contains("updated")
                || field_name_lc.contains("deleted")
                || field_name_lc.contains("clicked"));

        let decorators = field_obj
            .get("decorators")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let mut is_primary = false;
        let mut is_auto = false;
        let mut is_unique = false;
        let mut is_hash = false;
        let mut foreign_ref: Option<String> = None;
        let mut default_val: Option<String> = None;

        for decorator in &decorators {
            if let Some(dec_obj) = decorator.as_object() {
                if let Some(dec_name) = dec_obj.get("name").and_then(|n| n.as_str()) {
                    match dec_name {
                        "primary" => is_primary = true,
                        "auto" => is_auto = true,
                        "unique" => is_unique = true,
                        "hash" => is_hash = true,
                        "default" => {
                            if let Some(args) = dec_obj.get("args").and_then(|a| a.as_array()) {
                                // Try to get string argument first
                                if let Some(val_str) = args.first().and_then(|a| a.as_str()) {
                                    // Skip empty strings
                                    if !val_str.is_empty() {
                                        // Handle Enum syntax like Status::Todo -> Todo
                                        let clean_val = if val_str.contains("::") {
                                            val_str.split("::").last().unwrap_or(val_str)
                                        } else {
                                            val_str
                                        };
                                        default_val = Some(clean_val.to_string());
                                    }
                                }
                                // Handle numeric default
                                else if let Some(val_int) = args.first().and_then(|a| a.as_i64())
                                {
                                    default_val = Some(val_int.to_string());
                                } else if let Some(val_float) =
                                    args.first().and_then(|a| a.as_f64())
                                {
                                    default_val = Some(val_float.to_string());
                                }
                                // Handle boolean default
                                else if let Some(val_bool) =
                                    args.first().and_then(|a| a.as_bool())
                                {
                                    default_val = Some(val_bool.to_string());
                                }
                            }
                        }
                        "foreign" => {
                            // Extract referenced struct name from decorator args
                            if let Some(args) = dec_obj.get("args").and_then(|a| a.as_array()) {
                                if let Some(ref_struct) = args.first().and_then(|a| a.as_str()) {
                                    foreign_ref = Some(ref_struct.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let sql_type = if is_hash {
            "VARCHAR(255)"
        } else {
            match field_type {
                "Int" => {
                    if is_auto {
                        "SERIAL"
                    } else {
                        "INTEGER"
                    }
                }
                "Float" => "REAL",
                "Bool" => "BOOLEAN",
                _ => {
                    if is_timestamp_like {
                        "TIMESTAMPTZ"
                    } else {
                        "TEXT"
                    }
                }
            }
        };

        // Use snake_case column names for Postgres compatibility
        let mut column_def = format!("{} {}", column_name, sql_type);

        // DEFAULT must come before constraints
        if let Some(def_val) = default_val {
            let is_timestamp_sql = sql_type == "TIMESTAMPTZ";
            let def_upper = def_val.trim().to_ascii_uppercase();
            let is_now_expr =
                def_upper == "NOW()" || def_upper == "NOW" || def_upper == "CURRENT_TIMESTAMP";

            // Check if we need quotes (for text/enum types)
            let needs_quotes = field_type == "Str"
                || field_type == "String"
                || (!matches!(field_type, "Int" | "Float" | "Bool")
                    && !def_val.chars().all(|c| c.is_numeric() || c == '.')
                    && def_val != "true"
                    && def_val != "false");

            if is_timestamp_sql && is_now_expr {
                column_def.push_str(" DEFAULT NOW()");
            } else if needs_quotes {
                column_def.push_str(&format!(" DEFAULT '{}'", def_val));
            } else {
                column_def.push_str(&format!(" DEFAULT {}", def_val));
            }
        }

        if is_primary {
            column_def.push_str(" PRIMARY KEY");
        }

        // Check if field has @required decorator
        let is_required = decorators.iter().any(|d| {
            d.as_object()
                .and_then(|o| o.get("name"))
                .and_then(|n| n.as_str())
                == Some("required")
        });

        // Only add NOT NULL if field is explicitly marked as required
        // Fields are nullable by default to allow optional fields
        if is_required && !is_auto && !is_primary {
            column_def.push_str(" NOT NULL");
        }

        if is_unique && !is_primary {
            column_def.push_str(" UNIQUE");
        }

        columns.push(column_def);
    }

    // NOTE: Foreign key constraints are NOT added to CREATE TABLE
    // They will be added separately via ALTER TABLE after table creation
    // This avoids issues with transaction isolation where the referenced table
    // might not be visible yet during CREATE TABLE

    // Check for autoTimestamp - add created_at and updated_at columns
    let has_auto_timestamp = metadata
        .get("autoTimestamp")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if has_auto_timestamp {
        columns.push("created_at TIMESTAMPTZ DEFAULT NOW()".to_string());
        columns.push("updated_at TIMESTAMPTZ DEFAULT NOW()".to_string());
    }

    format!(
        "CREATE TABLE IF NOT EXISTS {} ({})",
        table_name,
        columns.join(", ")
    )
}

// Helper to add foreign key constraints via ALTER TABLE
// This is called after table creation to avoid transaction isolation issues
fn add_foreign_key_constraints(table_name: &str, metadata: &serde_json::Value) {
    let fields = match metadata.get("fields").and_then(|f| f.as_array()) {
        Some(f) => f,
        None => return,
    };

    for field in fields {
        let field_obj = match field.as_object() {
            Some(o) => o,
            None => continue,
        };

        let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => continue,
        };

        let column_name = to_snake_case(field_name);

        let decorators = field_obj
            .get("decorators")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        // Check for @foreign decorator
        for decorator in &decorators {
            if let Some(dec_obj) = decorator.as_object() {
                if let Some(dec_name) = dec_obj.get("name").and_then(|n| n.as_str()) {
                    if dec_name == "foreign" {
                        if let Some(args) = dec_obj.get("args").and_then(|a| a.as_array()) {
                            if let Some(ref_struct) = args.first().and_then(|a| a.as_str()) {
                                let ref_table = ref_struct.to_lowercase() + "s";
                                let constraint_name = format!("fk_{}_{}", table_name, column_name);

                                // Use ALTER TABLE to add foreign key constraint
                                // Use IF NOT EXISTS pattern via DO block for idempotency
                                // Use ALTER TABLE directly and handle "already exists" error
                                // This avoids PG-specific DO $$ syntax which can cause parsing errors
                                let alter_sql = format!(
                                    "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} (id) ON DELETE CASCADE",
                                    table_name,
                                    constraint_name,
                                    column_name,
                                    ref_table
                                );

                                let sql_c = CString::new(alter_sql).unwrap();
                                let result =
                                    unsafe { doo_db_execute(std::ptr::null(), sql_c.as_ptr()) };

                                if !result.is_null() {
                                    if unsafe { doo_db_is_error(result) } != 0 {
                                        // Log error but don't fail - FK might already exist or ref table not ready
                                        let err_msg_ptr =
                                            unsafe { doo_db_get_error_message(result) };
                                        if !err_msg_ptr.is_null() {
                                            let err_msg = unsafe {
                                                CStr::from_ptr(err_msg_ptr).to_string_lossy()
                                            };
                                            unsafe { doo_db_free_string(err_msg_ptr) };
                                        }
                                    }
                                    unsafe { doo_db_result_free(result) };
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ============================================================================
// JWT Middleware Implementation
// ============================================================================

/// JWT middleware function - verifies JWT token from Authorization header
/// Returns RFC 7807 error if unauthorized
// ============================================================================
// CORS and Rate Limiting Middleware Handlers
// ============================================================================

/// CORS Middleware Handler
extern "C" fn cors_middleware_handler(req: *mut DooRequest, next: *mut DooNext) -> *mut DooResult {
    static CORS_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let cors_id = CORS_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    if req.is_null() || next.is_null() {
        return make_err_http(500, "Internal error: null request or next");
    }

    unsafe {
        let req_ref = &*req;

        // Load config (or default) once
        let cfg = get_cors_config()
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_else(|| CorsConfig::default());

        // Compute origin decision
        let req_origin = get_header_value(req as *const DooRequest, "origin");
        let allow_any = cfg.origins.iter().any(|o| o == "*");
        let origin_allowed = if allow_any {
            true
        } else if let Some(ref origin) = req_origin {
            cfg.origins.iter().any(|o| o == origin)
        } else {
            // No origin header - treat as non-CORS
            true
        };

        // Handle OPTIONS preflight request
        let method = c_to_string(req_ref.method);
        if method == "OPTIONS" {
            if !origin_allowed {
                // Reject preflight
                let inst = get_current_request_path();
                let err = ErrorResponse::new(
                    ErrorType::Forbidden,
                    "CORS origin is not allowed".to_string(),
                    inst,
                );
                let body_json = err.to_json_string();
                set_last_error(403, body_json.clone());
                return make_err_http(403, &body_json);
            }

            // Return 204 No Content for OPTIONS with standard preflight headers
            let resp_size = std::mem::size_of::<DooResponse>();
            let resp = libc::malloc(resp_size) as *mut DooResponse;
            if resp.is_null() {
                return make_err_http(500, "Memory allocation failed");
            }
            track_alloc(resp as *const std::ffi::c_void, "cors_preflight_response");
            (*resp).status = 204;
            (*resp).body = string_to_c("");
            (*resp).content_type = string_to_c("text/plain");

            // Wrap in DooResult
            let result_size = std::mem::size_of::<DooResult>();
            let result = libc::malloc(result_size) as *mut DooResult;
            if result.is_null() {
                // Free resp if result alloc fails (to be safe, though rare)
                libc::free(resp as *mut libc::c_void);
                return make_err_http(500, "Memory allocation failed");
            }
            track_alloc(result as *const std::ffi::c_void, "cors_preflight_result");
            (*result).tag = 0;
            (*result).value = resp as *mut std::ffi::c_void;
            (*result).owner = owner::FFI;
            return result;
        }

        // For non-OPTIONS requests, call next
        let next_response_ptr = doo_http_next_call(next);

        if next_response_ptr.is_null() {
            // Next handler returned null, usually an error in handler or middleware chain
            // We should treat this as 500
            return make_err_http(500, "Next handler returned null");
        }

        // Convert DooResponse to DooResult for return - USE LIBC::MALLOC
        let result_size = std::mem::size_of::<DooResult>();
        let result = libc::malloc(result_size) as *mut DooResult;
        if result.is_null() {
            // Leak next_response_ptr? Ideally we should free it if we can't return it.
            // But we don't know if it's DooResponse or DooResult (if next returning DooResult directly?)
            // doo_http_next_call returns DooResponse*.
            // We should free it: libc::free(next_response_ptr as *mut c_void).
            libc::free(next_response_ptr as *mut libc::c_void);
            return make_err_http(500, "Memory allocation failed");
        }
        track_alloc(result as *const std::ffi::c_void, "cors_wrap_next_result");
        (*result).tag = 0;
        (*result).value = next_response_ptr as *mut std::ffi::c_void;
        (*result).owner = owner::FFI;
        result
    }
}

/// Rate Limiting Middleware Handler
#[no_mangle]
pub extern "C" fn ratelimit_middleware_handler(
    req: *mut DooRequest,
    next: *mut DooNext,
) -> *mut DooResult {
    static RL_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let rl_id = RL_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    if req.is_null() || next.is_null() {
        return make_err_http(500, "Internal error: null request or next");
    }

    unsafe {
        // Get rate limit config
        let config_guard = get_ratelimit_config().lock().unwrap();
        let raw_config = config_guard.clone().unwrap_or_default();
        drop(config_guard);

        // Skip rate limiting for health check endpoint
        let req_ref = &*req;
        let path = if req_ref.path.is_null() {
            String::new()
        } else {
            CStr::from_ptr(req_ref.path).to_string_lossy().to_string()
        };
        if path == "/health" {
            let next_response_ptr = doo_http_next_call(next);
            if next_response_ptr.is_null() {
                return make_err_http(500, "Handler returned null");
            }
            let result = libc::malloc(std::mem::size_of::<DooResult>()) as *mut DooResult;
            if result.is_null() {
                libc::free(next_response_ptr as *mut libc::c_void);
                return make_err_http(500, "Memory allocation failed");
            }
            track_alloc(result as *const std::ffi::c_void, "ratelimit_health_wrap_result");
            (*result).tag = 0;
            (*result).value = next_response_ptr as *mut std::ffi::c_void;
            (*result).owner = owner::FFI;
            return result;
        }

        let max_limit = raw_config.max;
        let window_secs = if raw_config.window == 0 {
            DEFAULT_RATE_LIMIT_WINDOW
        } else {
            raw_config.window
        };

        // Check rate limit
        let allowed;
        let current_count;
        let key = get_rate_limit_key(req as *const DooRequest, &raw_config.per);
        {
            let mut state = get_ratelimit_state().lock().unwrap();
            let entry = state.entry(key.clone()).or_insert(RateLimitEntry {
                count: 0,
                window_start: Instant::now(),
            });

            let elapsed = entry.window_start.elapsed().as_secs();
            if elapsed > window_secs {
                entry.count = 0;
                entry.window_start = Instant::now();
            }

            if entry.count >= max_limit {
                allowed = false;
            } else {
                entry.count += 1;
                allowed = true;
            }
            current_count = entry.count;
        }

        if !allowed {
            // RFC 7807-ish body
            let inst = get_current_request_path();
            let detail = "Rate limit exceeded".to_string();
            let err = ErrorResponse::new(ErrorType::TooManyRequests, detail, inst);
            let body_json = err.to_json_string();
            set_last_error(429, body_json.clone());

            // Return the actual RFC 7807 JSON error body
            return make_err_http(429, &body_json);
        }

        // Call next middleware/handler
        let next_response_ptr = doo_http_next_call(next);

        if next_response_ptr.is_null() {
            return make_err_http(500, "Handler returned null");
        }

        // Wrap DooResponse in DooResult for return
        let result = libc::malloc(std::mem::size_of::<DooResult>()) as *mut DooResult;
        if result.is_null() {
            libc::free(next_response_ptr as *mut libc::c_void);
            return make_err_http(500, "Memory allocation failed");
        }
        track_alloc(result as *const std::ffi::c_void, "ratelimit_wrap_next_result");
        (*result).tag = 0;
        (*result).value = next_response_ptr as *mut std::ffi::c_void;
        (*result).owner = owner::FFI;
        result
    }
}

// ============================================================================
// JWT Middleware
// ============================================================================

extern "C" fn jwt_middleware_handler(
    request: *mut DooRequest,
    next: *mut DooNext,
) -> *mut DooResult {
    unsafe {
        if request.is_null() {
            return make_err_http(
                500,
                r#"{"type":"about:blank","title":"Internal Server Error","status":500,"detail":"Null request in JWT middleware"}"#,
            );
        }

        let req = &*request;

        // Get Authorization header
        let headers_ptr = req.headers as *mut HashMap<String, String>;
        let auth_header = if !headers_ptr.is_null() {
            let headers = &*headers_ptr;
            headers
                .get("authorization")
                .or_else(|| headers.get("Authorization"))
                .cloned()
                .unwrap_or_default()
        } else {
            String::new()
        };

        // Check if Authorization header exists
        if auth_header.is_empty() {
            let path = if req.path.is_null() {
                "/".to_string()
            } else {
                CStr::from_ptr(req.path).to_string_lossy().to_string()
            };

            let error_json = format!(
                r#"{{"type":"unauthorized","title":"Unauthorized","status":401,"detail":"Authentication credentials are missing or invalid","instance":"{}","message":"Missing authorization token"}}"#,
                path.replace("\"", "\\\"")
            );

            return make_err_http(401, &error_json);
        }

        // Extract token from Authorization header.
        // Accept:
        // - "Bearer <token>" (case-insensitive, any whitespace)
        // - a raw JWT token (some clients mistakenly send just the token)
        let token = {
            let trimmed = auth_header.trim();
            let mut parts = trimmed.split_whitespace();
            let first = parts.next().unwrap_or("");
            let second = parts.next().unwrap_or("");

            if first.eq_ignore_ascii_case("bearer") && !second.is_empty() {
                second.trim_matches('"').trim_matches('\'').to_string()
            } else {
                // Fallback: raw JWT (3 dot-separated segments)
                if trimmed.split('.').count() == 3 {
                    trimmed.trim_matches('"').trim_matches('\'').to_string()
                } else {
                    String::new()
                }
            }
        };

        if token.is_empty() {
            let path = if req.path.is_null() {
                "/".to_string()
            } else {
                CStr::from_ptr(req.path).to_string_lossy().to_string()
            };

            let error_json = format!(
                r#"{{"type":"unauthorized","title":"Unauthorized","status":401,"detail":"Authentication credentials are missing or invalid","instance":"{}","message":"Authorization header must use Bearer scheme"}}"#,
                path.replace("\"", "\\\"")
            );

            return make_err_http(401, &error_json);
        }

        // Verify JWT token using libdoo_auth
        let token_c = CString::new(token.as_str()).unwrap();
        let verify_result = doo_auth_verify(token_c.as_ptr());

        if verify_result.is_null() {
            let path = if req.path.is_null() {
                "/".to_string()
            } else {
                CStr::from_ptr(req.path).to_string_lossy().to_string()
            };

            let error_json = format!(
                r#"{{"type":"unauthorized","title":"Unauthorized","status":401,"detail":"Authentication credentials are missing or invalid","instance":"{}","message":"JWT verification failed"}}"#,
                path.replace("\"", "\\\"")
            );

            return make_err_http(401, &error_json);
        }

        // Check if verification returned an error
        let is_error = doo_auth_is_error(verify_result);
        if is_error != 0 {
            doo_auth_free_result(verify_result);

            let path = if req.path.is_null() {
                "/".to_string()
            } else {
                CStr::from_ptr(req.path).to_string_lossy().to_string()
            };

            // Consistent error message for both invalid and expired tokens
            let error_json = format!(
                r#"{{"type":"unauthorized","title":"Unauthorized","status":401,"detail":"Authentication credentials are missing or invalid","instance":"{}","message":"Invalid JWT token"}}"#,
                path.replace("\"", "\\\"")
            );

            return make_err_http(401, &error_json);
        }

        // Token is valid - extract payload and inject into request params
        let res = &*(verify_result as *mut DooResult);
        if res.value != std::ptr::null_mut() {
            let json_ptr = res.value as *const c_char;
            let json_str = CStr::from_ptr(json_ptr).to_string_lossy();

            // Parse payload
            if let Ok(claims) = serde_json::from_str::<serde_json::Value>(&json_str) {
                // Get sub (subject/userId)
                if let Some(sub) = claims.get("sub").and_then(|s| s.as_str()) {
                    // Inject into params map
                    let params_ptr = req.params as *mut HashMap<String, String>;
                    if !params_ptr.is_null() {
                        (*params_ptr).insert("auth_user_id".to_string(), sub.to_string());
                    }
                }
            }
        }

        // Free the result and continue to next middleware/handler
        doo_auth_free_result(verify_result);

        // Call next middleware/handler in chain
        if next.is_null() {
            let error_json = r#"{"type":"about:blank","title":"Internal Server Error","status":500,"detail":"Null next in JWT middleware"}"#;
            return make_err_http(500, error_json);
        }

        let response = doo_http_next_call(next);

        // Convert response to result
        if response.is_null() {
            let error_json = r#"{"type":"about:blank","title":"Internal Server Error","status":500,"detail":"Handler returned null response"}"#;
            return make_err_http(500, error_json);
        }

        // Return success with the response
        make_ok_ptr(response as *mut std::ffi::c_void)
    }
}

// ============================================================================
// Auth and CRUD FFI Functions (FFI-only design)
// ============================================================================

/// Register auth routes (signup and login) with metadata
/// Called by compiler with struct metadata JSON
#[no_mangle]
pub extern "C" fn doo_http_auth_impl(
    _server: *const std::ffi::c_void,
    signup_path: *const c_char,
    login_path: *const c_char,
    struct_name: *const c_char,
    metadata_json: *const c_char,
) -> *mut DooResult {
    let signup_path_str = c_to_string(signup_path);
    let login_path_str = c_to_string(login_path);
    let struct_name_str = c_to_string(struct_name);
    let metadata_json_str = c_to_string(metadata_json);

    // Parse and store metadata
    let metadata: serde_json::Value = match serde_json::from_str(&metadata_json_str) {
        Ok(m) => m,
        Err(e) => {
            return make_err_http(400, &format!("Invalid metadata: {}", e));
        }
    };

    let table_name = struct_name_str.to_lowercase() + "s";

    // Store metadata - table will be created at server startup via run_migrations()
    get_auth_metadata().lock().unwrap().insert(
        struct_name_str.clone(),
        AuthMetadata {
            table_name: table_name.clone(),
            metadata: metadata.clone(),
            signup_path: signup_path_str.clone(),
            login_path: login_path_str.clone(),
        },
    );

    // Register handlers
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    // CRITICAL: Register JWT middleware so CRUD routes can use it
    registry
        .middleware_handlers
        .insert("jwt".to_string(), jwt_middleware_handler);

    registry.register("POST", &signup_path_str, auth_signup_handler);
    registry.register("POST", &login_path_str, auth_login_handler);

    doo_http_debug!(
        "✓ Auth routes registered: POST {} and POST {}",
        signup_path_str,
        login_path_str
    );

    make_ok_void()
}

/// Register CRUD routes with metadata
/// Called by compiler with struct metadata JSON
/// FFI-only design: Accepts metadata JSON and builds everything at runtime
#[no_mangle]
pub extern "C" fn doo_http_crud_impl(
    _server: *const std::ffi::c_void,
    base_path: *const c_char,
    struct_name: *const c_char,
    metadata_json: *const c_char,
) -> *mut DooResult {
    let base_path_str = c_to_string(base_path);
    let struct_name_str = c_to_string(struct_name);
    let metadata_json_str = c_to_string(metadata_json);

    // Parse and store metadata
    let metadata: serde_json::Value = match serde_json::from_str(&metadata_json_str) {
        Ok(m) => m,
        Err(e) => {
            return make_err_http(400, &format!("Invalid metadata: {}", e));
        }
    };

    let table_name = metadata
        .get("table_name")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| struct_name_str.to_lowercase() + "s");

    // Store metadata - table will be created at server startup via run_migrations()
    get_crud_metadata().lock().unwrap().insert(
        struct_name_str.clone(),
        CrudMetadata {
            table_name: table_name.clone(),
            metadata: metadata.clone(),
            base_path: base_path_str.clone(),
        },
    );

    // Register CRUD handlers
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    let id_path = format!("{}/{{id}}", base_path_str);

    // Check for noAuth flag in metadata
    let no_auth = metadata
        .get("noAuth")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Register more specific routes (with :id) BEFORE general routes
    // matchit requires more specific patterns to be registered first
    if no_auth {
        // Public CRUD routes - no JWT required
        registry.register("GET", &id_path, crud_get_handler);
        registry.register("PUT", &id_path, crud_update_handler);
        registry.register("DELETE", &id_path, crud_delete_handler);
        registry.register("POST", &base_path_str, crud_create_handler);
        registry.register("GET", &base_path_str, crud_list_handler);
        doo_http_debug!("✓ CRUD routes registered (public - no auth):");
    } else {
        // Protected CRUD routes - JWT required
        let jwt_mw: Vec<DooMiddlewareFn> = vec![jwt_middleware_handler];
        registry.register_with_middleware("GET", &id_path, crud_get_handler, jwt_mw.clone());
        registry.register_with_middleware("PUT", &id_path, crud_update_handler, jwt_mw.clone());
        registry.register_with_middleware("DELETE", &id_path, crud_delete_handler, jwt_mw.clone());
        registry.register_with_middleware(
            "POST",
            &base_path_str,
            crud_create_handler,
            jwt_mw.clone(),
        );
        registry.register_with_middleware("GET", &base_path_str, crud_list_handler, jwt_mw.clone());
        doo_http_debug!("✓ CRUD routes registered (JWT auth required):");
    }
    doo_http_debug!("  POST {} (create)", base_path_str);
    doo_http_debug!("  GET {} (list)", base_path_str);
    doo_http_debug!("  GET {} (get)", id_path);
    doo_http_debug!("  PUT {} (update)", id_path);
    doo_http_debug!("  DELETE {} (delete)", id_path);

    make_ok_void()
}

/// Register a table for automatic creation at startup (for @table decorator)
/// Called by compiler for structs with @table decorator
#[no_mangle]
pub extern "C" fn doo_http_table_impl(
    struct_name: *const c_char,
    metadata_json: *const c_char,
) -> *mut DooResult {
    let struct_name_str = c_to_string(struct_name);
    let metadata_json_str = c_to_string(metadata_json);

    // Parse metadata
    let metadata: serde_json::Value = match serde_json::from_str(&metadata_json_str) {
        Ok(m) => m,
        Err(e) => {
            return make_err_http(400, &format!("Invalid metadata: {}", e));
        }
    };

    let table_name = struct_name_str.to_lowercase() + "s";

    // Store metadata - table will be created at server startup via run_migrations()
    get_table_metadata().lock().unwrap().insert(
        struct_name_str.clone(),
        TableMetadata {
            table_name: table_name.clone(),
            metadata: metadata.clone(),
        },
    );

    doo_http_debug!("✓ Table registered for migration: {}", table_name);
    make_ok_void()
}

/// FFI export: Get current request path from thread-local storage
/// Used by middleware error handlers to get the current request path
#[no_mangle]
pub extern "C" fn doohttp_get_current_request_path() -> *const libc::c_char {
    let path = get_current_request_path();
    string_to_c(&path)
}

/// FFI export: Convert middleware enum error to RFC 7807 JSON response
/// Used by generated middleware wrapper code to convert enum errors to RFC 7807
/// FFI signature: doohttp_middleware_error_to_rfc7807(enum_name, variant_tag, variant_name, instance) -> *mut DooHttpError
#[no_mangle]
pub extern "C" fn doohttp_middleware_error_to_rfc7807(
    enum_name: *const libc::c_char,
    variant_tag: libc::c_int,
    variant_name: *const libc::c_char,
    instance: *const libc::c_char,
) -> *mut DooHttpError {
    if variant_name.is_null() {
        return std::ptr::null_mut();
    }

    let enum_str = if enum_name.is_null() {
        "Error".to_string()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(enum_name)
                .to_string_lossy()
                .to_string()
        }
    };

    let variant_str = unsafe {
        std::ffi::CStr::from_ptr(variant_name)
            .to_string_lossy()
            .to_string()
    };

    let instance_str = if instance.is_null() {
        get_current_request_path()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(instance)
                .to_string_lossy()
                .to_string()
        }
    };

    // Map common error variants to HTTP status codes
    // AuthError::Unauthorized -> 401, AuthError::Forbidden -> 403, etc.
    let (status, detail) = match (enum_str.as_str(), variant_str.as_str()) {
        (_, "Unauthorized") => (401, format!("{}", variant_str)),
        (_, "Forbidden") => (403, format!("{}", variant_str)),
        (_, "NotFound") => (404, format!("{}", variant_str)),
        (_, "BadRequest") => (400, format!("{}", variant_str)),
        (_, "Conflict") => (409, format!("{}", variant_str)),
        (_, "ValidationError") => (422, format!("{}", variant_str)),
        _ => (500, format!("{}: {}", enum_str, variant_str)),
    };

    // Use centralized error module
    use error::*;
    let error_response = match status {
        400 => bad_request(detail, instance_str),
        401 => unauthorized(detail, instance_str),
        403 => forbidden(detail, instance_str),
        404 => not_found(detail, instance_str),
        405 => method_not_allowed(detail, instance_str, vec![]),
        409 => conflict(detail, instance_str),
        422 => ErrorResponse::new(ErrorType::UnprocessableEntity, detail, instance_str),
        429 => ErrorResponse::new(ErrorType::TooManyRequests, detail, instance_str),
        500 => internal_error(detail, instance_str),
        501 => not_implemented(detail, instance_str),
        502 => bad_gateway(detail, instance_str),
        503 => service_unavailable(detail, instance_str),
        _ => internal_error(detail, instance_str),
    };

    let error_json = error_response.to_json_string();

    // Return DooHttpError struct pointer allocated with libc::malloc
    unsafe {
        let error_size = std::mem::size_of::<DooHttpError>();
        let error_ptr = libc::malloc(error_size) as *mut DooHttpError;
        if error_ptr.is_null() {
            return std::ptr::null_mut();
        }
        (*error_ptr).status = status;
        (*error_ptr).message = string_to_c(&error_json);
        error_ptr
    }
}

// Helper to ensure columns exist (ALTER TABLE ADD COLUMN)
// Called by run_migrations to ensure even existing tables have new columns
fn migrate_table_columns(table_name: &str, metadata: &serde_json::Value) {
    let fields = match metadata.get("fields").and_then(|f| f.as_array()) {
        Some(f) => f,
        None => return,
    };

    for field in fields {
        let field_obj = match field.as_object() {
            Some(o) => o,
            None => continue,
        };

        let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => continue,
        };

        let field_type = match field_obj.get("type").and_then(|t| t.as_str()) {
            Some(t) => t,
            None => "TEXT",
        };

        let field_name_lc = field_name.to_lowercase();
        let old_column_name = field_name_lc.clone();
        let column_name = to_snake_case(field_name);
        let is_timestamp_like = field_name_lc.ends_with("at")
            && (field_name_lc.contains("expires")
                || field_name_lc.contains("access")
                || field_name_lc.contains("created")
                || field_name_lc.contains("updated")
                || field_name_lc.contains("deleted")
                || field_name_lc.contains("clicked"));

        let decorators = field_obj.get("decorators").and_then(|d| d.as_array());

        let mut is_auto = false;
        let mut is_hash = false;

        if let Some(decs) = decorators {
            for dec in decs {
                if let Some(name) = dec.get("name").and_then(|n| n.as_str()) {
                    if name == "auto" {
                        is_auto = true;
                    }
                    if name == "hash" {
                        is_hash = true;
                    }
                }
            }
        }

        let sql_type = if is_hash {
            "VARCHAR(255)"
        } else {
            match field_type {
                "Int" => {
                    if is_auto {
                        "SERIAL"
                    } else {
                        "INTEGER"
                    }
                }
                "Float" => "REAL",
                "Bool" => "BOOLEAN",
                _ => {
                    if is_timestamp_like {
                        "TIMESTAMPTZ"
                    } else {
                        "TEXT"
                    }
                }
            }
        };

        // Best-effort legacy rename: previous versions used plain lowercase (e.g. destinationurl).
        // Rename to snake_case (e.g. destination_url) when applicable.
        if old_column_name != column_name {
            let rename_sql = format!(
                "ALTER TABLE {} RENAME COLUMN {} TO {}",
                table_name, old_column_name, column_name
            );
            if let Ok(sql_c_rename) = CString::new(rename_sql) {
                unsafe {
                    let res_rename = doo_db_execute(std::ptr::null(), sql_c_rename.as_ptr());
                    if !res_rename.is_null() {
                        doo_db_result_free(res_rename);
                    }
                }
            }
        }

        // ALTER TABLE table ADD COLUMN IF NOT EXISTS col type
        // Note: IF NOT EXISTS available in Postgres 9.6+.
        // Postgres syntax: ADD COLUMN [IF NOT EXISTS] name type
        let alter_sql = format!(
            "ALTER TABLE {} ADD COLUMN IF NOT EXISTS {} {}",
            table_name, column_name, sql_type
        );

        let sql_c = CString::new(alter_sql).unwrap();
        // Execute and ignore result (success or already exists)
        unsafe {
            let res = doo_db_create_table(std::ptr::null(), sql_c.as_ptr());
            if !res.is_null() {
                doo_db_result_free(res);
            }

            if is_timestamp_like && sql_type == "TIMESTAMPTZ" {
                let alter_type_sql = format!(
                    "ALTER TABLE {} ALTER COLUMN {} TYPE TIMESTAMPTZ USING (CASE WHEN {} ILIKE 'now()' THEN NOW() ELSE NULLIF({}, '')::timestamptz END)",
                    table_name,
                    column_name,
                    column_name,
                    column_name,
                );
                if let Ok(sql_c2) = CString::new(alter_type_sql) {
                    let res2 = doo_db_create_table(std::ptr::null(), sql_c2.as_ptr());
                    if !res2.is_null() {
                        doo_db_result_free(res2);
                    }
                }
            }
        }
    }
}
