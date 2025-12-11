//! HTTP Server FFI for Doo language
//! Phase 3, 4, 5: Complete implementation with closures, JSON, groups, middleware

mod error;

use error::*;
use std::cell::RefCell;
use std::collections::HashMap;
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
use tokio::net::TcpListener;

// Thread-local RFC 7807 last error (status, json body) populated by parsing/param helpers
thread_local! {
    static LAST_RFC_ERROR: RefCell<Option<(i32, String)>> = RefCell::new(None);
}

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
struct Route {
    handler: DooHandlerFn,
    middleware: Vec<DooMiddlewareFn>,
}

/// Route registry storing method -> router with handlers
struct RouteRegistry {
    routes: HashMap<String, Router<Route>>,  // method -> router
    handlers: HashMap<String, DooHandlerFn>, // handler_name -> function pointer
    middleware: Vec<DooMiddlewareFn>,        // global middleware
    middleware_handlers: HashMap<String, DooMiddlewareFn>, // middleware_name -> function pointer
    groups: HashMap<String, Vec<DooMiddlewareFn>>, // prefix -> middleware for groups
}

impl RouteRegistry {
    fn new() -> Self {
        Self {
            routes: HashMap::new(),
            handlers: HashMap::new(),
            middleware: Vec::new(),
            middleware_handlers: HashMap::new(),
            groups: HashMap::new(),
        }
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
        };

        if let Err(e) = router.insert(path, route) {
            eprintln!("Failed to register route {} {}: {}", method, path, e);
        } else {
            println!("✓ Registered: {} {}", method, path);
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
        };

        if let Err(e) = router.insert(path, route) {
            eprintln!("Failed to register route {} {}: {}", method, path, e);
        } else {
            println!(
                "✓ Registered: {} {} (with {} middleware)",
                method, path, middleware_len
            );
        }
    }

    fn register_by_name(&mut self, method: &str, path: &str, handler_name: &str) {
        if let Some(handler_fn) = self.handlers.get(handler_name).copied() {
            self.register(method, path, handler_fn);
        } else {
            eprintln!("Warning: Handler {} not found in registry", handler_name);
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
            eprintln!("Warning: Handler {} not found in registry", handler_name);
        }
    }

    fn add_middleware(&mut self, mw: DooMiddlewareFn) {
        self.middleware.push(mw);
    }

    fn find_route(&self, method: &str, path: &str) -> Option<(&Route, HashMap<String, String>)> {
        let method = method.to_uppercase();
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
}

fn get_routes() -> &'static Arc<Mutex<RouteRegistry>> {
    ROUTES.get_or_init(|| Arc::new(Mutex::new(RouteRegistry::new())))
}

// Result type for FFI returns
// tag: 0 = Ok, 1 = Err
#[repr(C)]
pub struct DooResult {
    tag: i32,
    value: *mut std::ffi::c_void,
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

// ============================================================================
// Next.call() FFI function
// ============================================================================

/// Call the next middleware or handler in the chain
/// Returns a Response struct (not DooResult) for middleware to use
#[no_mangle]
pub extern "C" fn doo_http_next_call(next: *mut DooNext) -> *mut DooResponse {
    if next.is_null() {
        eprintln!("Error: next.call() called with null Next pointer");
        // Return error response
        return Box::into_raw(Box::new(DooResponse {
            status: 500,
            body: string_to_c("Internal error: null Next"),
            content_type: string_to_c("text/plain"),
        }));
    }

    unsafe {
        let next_ref = &mut *next;
        let request = next_ref.request;

        // Get the remaining middleware chain
        let middleware_vec_ptr = next_ref.remaining_middleware as *mut Vec<DooMiddlewareFn>;

        let result: *mut DooResult = if middleware_vec_ptr.is_null() {
            // No more middleware, call the handler
            (next_ref.handler)(request)
        } else {
            let middleware_vec = &*middleware_vec_ptr;
            let idx = next_ref.current_index;

            if idx >= middleware_vec.len() {
                // No more middleware, call the handler
                (next_ref.handler)(request)
            } else {
                // Create a new Next for the next middleware in chain
                let new_middleware_vec = middleware_vec.clone();
                let new_next = Box::new(DooNext {
                    request,
                    remaining_middleware: Box::into_raw(Box::new(new_middleware_vec))
                        as *mut std::ffi::c_void,
                    handler: next_ref.handler,
                    current_index: idx + 1,
                });

                // Call the current middleware
                let current_middleware = middleware_vec[idx];
                current_middleware(request, Box::into_raw(new_next))
            }
        };

        // Convert DooResult to DooResponse for middleware to use
        if result.is_null() {
            return Box::into_raw(Box::new(DooResponse {
                status: 500,
                body: string_to_c("Handler returned null"),
                content_type: string_to_c("text/plain"),
            }));
        }

        let result_box = Box::from_raw(result);

        if result_box.tag == 0 {
            // Success - extract DooResponse
            if result_box.value.is_null() {
                Box::into_raw(Box::new(DooResponse {
                    status: 200,
                    body: string_to_c(""),
                    content_type: string_to_c("text/plain"),
                }))
            } else {
                let response_ptr = result_box.value as *mut DooResponse;
                response_ptr // Return the response pointer directly
            }
        } else {
            // Error - convert to error response
            let error_ptr = result_box.value as *mut DooHttpError;
            if error_ptr.is_null() {
                Box::into_raw(Box::new(DooResponse {
                    status: 500,
                    body: string_to_c("Unknown error"),
                    content_type: string_to_c("application/json"),
                }))
            } else {
                let error = Box::from_raw(error_ptr);
                Box::into_raw(Box::new(DooResponse {
                    status: error.status,
                    body: error.message,
                    content_type: string_to_c("application/json"),
                }))
            }
        }
    }
}

// Helper to convert Rust String to C string
fn string_to_c(s: &str) -> *const c_char {
    CString::new(s)
        .expect("Failed to create CString")
        .into_raw()
}

fn c_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

fn make_ok_void() -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: std::ptr::null_mut(),
    }))
}

fn make_ok_string(s: &str) -> *mut DooResult {
    let c_str = CString::new(s).expect("CString conversion failed");
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: c_str.into_raw() as *mut std::ffi::c_void,
    }))
}

fn make_err_http(status: u16, message: &str) -> *mut DooResult {
    let error = Box::new(DooHttpError {
        status: status as i32,
        message: string_to_c(message),
    });
    Box::into_raw(Box::new(DooResult {
        tag: 1,
        value: Box::into_raw(error) as *mut std::ffi::c_void,
    }))
}

/// Helper: get human-friendly JSON type name
fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "Null",
        serde_json::Value::Bool(_) => "Bool",
        serde_json::Value::Number(n) => {
            if n.is_i64() {
                "Int"
            } else {
                "Float"
            }
        }
        serde_json::Value::String(_) => "Str",
        serde_json::Value::Array(_) => "Array",
        serde_json::Value::Object(_) => "Map",
    }
}

// ============================================================================
// FFI Functions - Called from Doo code
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_register_handler(name: *const c_char, handler: DooHandlerFn) {
    let handler_name = c_to_string(name);
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.handlers.insert(handler_name.clone(), handler);
    println!("✓ Registered handler function: {}", handler_name);
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
    println!("✓ Registered middleware function: {}", middleware_name);
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
        if let Some(mw_fn) = registry.middleware_handlers.get(&mw_name).copied() {
            middleware_fns.push(mw_fn);
        } else {
            eprintln!("Warning: Middleware {} not found", mw_name);
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
    _server: *const std::ffi::c_void,
    middleware_name: *const c_char,
) -> *mut DooResult {
    let middleware_str = c_to_string(middleware_name);
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    // Look up middleware function pointer by name
    if let Some(mw_fn) = registry.middleware_handlers.get(&middleware_str).copied() {
        registry.add_middleware(mw_fn);
        println!("✓ Registered global middleware: {}", middleware_str);
    } else {
        eprintln!("Warning: Middleware {} not found", middleware_str);
    }

    make_ok_void()
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
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();

    // Set thread-local path early for downstream errors
    set_current_request_path(&path);

    // Log incoming request
    println!("→ {} {}", method, path);

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
    let content_type_opt = req
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());
    let content_type = content_type_opt
        .clone()
        .unwrap_or_else(|| "text/plain".to_string());

    // Enforce content-type for body methods
    let requires_body = method == "POST" || method == "PUT" || method == "PATCH";
    if requires_body {
        // Check if Content-Type header exists first
        if content_type_opt.is_none() {
            // Missing Content-Type header
            use error::*;
            let err = content_type_error(
                "Content-Type header required for POST/PUT/PATCH requests".to_string(),
                path.clone(),
                Some("application/json".to_string()),
                None,
            );
            let body_json = err.to_json_string();
            set_last_error(err.status_code() as i32, body_json.clone());
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("content-type", "application/json")
                .body(Full::new(Bytes::from(body_json)))
                .unwrap());
        }

        // Check if Content-Type is application/json
        let is_json = content_type
            .to_ascii_lowercase()
            .starts_with("application/json");
        if !is_json {
            // Wrong Content-Type header
            use error::*;
            let err = content_type_error(
                "Invalid Content-Type header".to_string(),
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
            // Check if path exists for other methods to return 405
            let mut allowed_methods = Vec::new();
            for (m, router) in registry.routes.iter() {
                if m.eq_ignore_ascii_case(&method) {
                    continue;
                }
                if router.at(&path).is_ok() {
                    allowed_methods.push(m.clone());
                }
            }

            drop(registry);

            if !allowed_methods.is_empty() {
                println!("← 405 Method Not Allowed");
                let error_response = error::method_not_allowed(
                    "The requested method is not allowed for this route".to_string(),
                    path.clone(),
                    allowed_methods,
                )
                .with_method(method.clone());
                let error_json = error_response.to_json_string();
                return Ok(Response::builder()
                    .status(StatusCode::METHOD_NOT_ALLOWED)
                    .header("content-type", "application/json")
                    .body(Full::new(Bytes::from(error_json)))
                    .unwrap());
            }

            println!("← 404 Not Found");
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
    };

    let handler = route.handler;
    let middleware = route.middleware.clone();
    let global_middleware = registry.middleware.clone();

    drop(registry);

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

    // Combine global and route-specific middleware
    let mut all_middleware = global_middleware.clone();
    all_middleware.extend(middleware.iter().cloned());

    let req_ptr = Box::into_raw(doo_request);

    // If there's middleware, create Next chain and call first middleware
    let result = if !all_middleware.is_empty() {
        // Create Next object that represents the chain
        let middleware_box = Box::new(all_middleware);
        let next = Box::new(DooNext {
            request: req_ptr,
            remaining_middleware: Box::into_raw(middleware_box) as *mut std::ffi::c_void,
            handler,
            current_index: 0,
        });

        // Call the first middleware
        let first_middleware = unsafe {
            let mw_vec_ptr = next.remaining_middleware as *mut Vec<DooMiddlewareFn>;
            let mw_vec = &*mw_vec_ptr;
            mw_vec[0]
        };

        // Create Next for second middleware onward
        let next_for_first = Box::new(DooNext {
            request: req_ptr,
            remaining_middleware: next.remaining_middleware,
            handler,
            current_index: 1,
        });

        first_middleware(req_ptr, Box::into_raw(next_for_first))
    } else {
        // No middleware, call handler directly
        println!(
            "DEBUG HANDLE_REQUEST: About to call handler, thread-local path is: {}",
            get_current_request_path()
        );
        let result = handler(req_ptr);
        println!(
            "DEBUG HANDLE_REQUEST: Handler returned, thread-local path is: {}",
            get_current_request_path()
        );
        result
    };

    // Process result
    let response = unsafe {
        if result.is_null() {
            println!("← 500 Internal Server Error (null result)");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from("Handler returned null")))
                .unwrap()
        } else {
            let result_box = Box::from_raw(result);

            if result_box.tag == 0 {
                // Success - value is DooResponse*
                if result_box.value.is_null() {
                    println!("← 200 OK (empty)");
                    Response::builder()
                        .status(StatusCode::OK)
                        .body(Full::new(Bytes::from("")))
                        .unwrap()
                } else {
                    let response_ptr = result_box.value as *mut DooResponse;
                    let response = Box::from_raw(response_ptr);

                    println!("DEBUG RESPONSE: response.status = {}", response.status);
                    let status =
                        StatusCode::from_u16(response.status as u16).unwrap_or(StatusCode::OK);
                    let body_str = if response.body.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr(response.body).to_string_lossy().to_string()
                    };
                    let content_type_str = if response.content_type.is_null() {
                        "application/json".to_string()
                    } else {
                        CStr::from_ptr(response.content_type)
                            .to_string_lossy()
                            .to_string()
                    };

                    println!(
                        "← {} {}",
                        status.as_u16(),
                        status.canonical_reason().unwrap_or("")
                    );
                    Response::builder()
                        .status(status)
                        .header("content-type", content_type_str)
                        .body(Full::new(Bytes::from(body_str)))
                        .unwrap()
                }
            } else {
                // Error - value is DooHttpError*
                let error_ptr = result_box.value as *mut DooHttpError;
                let error = Box::from_raw(error_ptr);

                let status = StatusCode::from_u16(error.status as u16)
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                let message = if error.message.is_null() {
                    "Unknown error".to_string()
                } else {
                    CStr::from_ptr(error.message).to_string_lossy().to_string()
                };

                println!(
                    "← {} {}",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("")
                );

                // Check if message is already RFC 7807 JSON (starts with {"type": or {"detail":)
                let body_str =
                    if message.starts_with("{\"type\":") || message.starts_with("{\"detail\":") {
                        // Already RFC 7807 format, use as-is
                        message
                    } else {
                        // Legacy error, wrap in simple error object
                        format!("{{\"error\":\"{}\"}}", message.replace("\"", "\\\""))
                    };

                Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .body(Full::new(Bytes::from(body_str)))
                    .unwrap()
            }
        }
    };

    Ok(response)
}

#[no_mangle]
pub extern "C" fn doo_http_listen(server_ptr: *const std::ffi::c_void) -> *mut DooResult {
    // Extract port from Server struct
    // Server struct layout: { Port: i32, Host: *const c_char }
    let port = if server_ptr.is_null() {
        3000 // Default port
    } else {
        unsafe {
            // Read the first i32 field (Port)
            let port_ptr = server_ptr as *const i32;
            *port_ptr
        }
    };

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => return make_err_http(500, &format!("Failed to create tokio runtime: {}", e)),
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], port as u16));

    println!();
    println!("🚀 Server starting on http://127.0.0.1:{}", port);
    println!();

    // Print all registered routes
    let routes = get_routes();
    let registry = routes.lock().unwrap();
    println!("📋 Registered routes:");
    for (method, _router) in registry.routes.iter() {
        println!("  {} routes: registered", method);
    }
    println!();
    drop(registry);

    runtime.block_on(async {
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Failed to bind to {}: {}", addr, e);
                return;
            }
        };

        println!("✓ Listening on {}", addr);
        println!();

        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to accept connection: {}", e);
                    continue;
                }
            };

            tokio::task::spawn(async move {
                let io = TokioIo::new(stream);
                if let Err(err) = http1::Builder::new()
                    .serve_connection(io, service_fn(handle_request))
                    .await
                {
                    eprintln!("Error serving connection: {:?}", err);
                }
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
            // Mark missing path parameter error for downstream handling
            use error::*;
            let param = ParameterError::new(key_str.clone())
                .with_message(format!("Path parameter '{}' is required", key_str));
            let err = parameter_error(
                "Path parameter not found".to_string(),
                get_current_request_path(),
                param,
            );
            set_last_error(err.status_code() as i32, err.to_json_string());
            std::ptr::null()
        }
    }
}

/// Extract query parameter with type validation and set RFC7807 errors on failure
#[no_mangle]
pub extern "C" fn doohttp_extract_query_typed(
    request: *const DooRequest,
    param_name: *const libc::c_char,
    param_type: *const libc::c_char,
) -> *const libc::c_char {
    clear_last_error();
    if request.is_null() || param_name.is_null() || param_type.is_null() {
        return std::ptr::null();
    }

    let name_str = unsafe {
        std::ffi::CStr::from_ptr(param_name)
            .to_string_lossy()
            .to_string()
    };
    let ty_str = unsafe {
        std::ffi::CStr::from_ptr(param_type)
            .to_string_lossy()
            .to_string()
    };

    let req = unsafe { &*request };
    if req.query.is_null() {
        return std::ptr::null();
    }
    let query_map = unsafe { &*(req.query as *const HashMap<String, String>) };

    match query_map.get(&name_str) {
        Some(value) => {
            let ok = match ty_str.as_str() {
                "Int" | "I32" | "I64" => value.parse::<i64>().is_ok(),
                "Float" | "F32" | "F64" => value.parse::<f64>().is_ok(),
                "Bool" => value == "true" || value == "false",
                _ => true,
            };
            if ok {
                string_to_c(value)
            } else {
                let param = ParameterError::new(name_str.clone())
                    .with_expected(ty_str.clone())
                    .with_received(value.clone());
                let err = invalid_query_param_type_error(get_current_request_path(), param);
                set_last_error(err.status_code() as i32, err.to_json_string());
                std::ptr::null()
            }
        }
        None => {
            let param = ParameterError::new(name_str.clone())
                .with_message(format!("Query parameter '{}' is required", name_str))
                .with_expected(ty_str.clone());
            let err = missing_query_param_error(get_current_request_path(), param);
            set_last_error(err.status_code() as i32, err.to_json_string());
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

// ============================================================================
// Memory Management
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_free_result(result: *mut DooResult) {
    if !result.is_null() {
        unsafe { drop(Box::from_raw(result)) };
    }
}

#[no_mangle]
pub extern "C" fn doo_http_free_string(s: *const c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s as *mut c_char)) };
    }
}

#[no_mangle]
pub extern "C" fn doo_http_free_request(req: *mut DooRequest) {
    if req.is_null() {
        return;
    }
    unsafe {
        let request = Box::from_raw(req);

        // Free C strings
        if !request.method.is_null() {
            drop(CString::from_raw(request.method as *mut c_char));
        }
        if !request.path.is_null() {
            drop(CString::from_raw(request.path as *mut c_char));
        }
        if !request.body.is_null() {
            drop(CString::from_raw(request.body as *mut c_char));
        }
        if !request.content_type.is_null() {
            drop(CString::from_raw(request.content_type as *mut c_char));
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
    let struct_name_str = unsafe {
        std::ffi::CStr::from_ptr(struct_name)
            .to_string_lossy()
            .to_string()
    };
    let validator_str = if validator_spec.is_null() {
        String::new()
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(validator_spec)
                .to_string_lossy()
                .to_string()
        }
    };

    // Parse JSON
    let json_value: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(v) => v,
        Err(_) => {
            // 400 Bad Request - malformed JSON
            let inst = get_current_request_path();
            let err = invalid_json_error(inst);
            set_last_error(err.status_code() as i32, err.to_json_string());
            return std::ptr::null_mut();
        }
    };

    // Must be an object
    let obj = match json_value.as_object() {
        Some(o) => o,
        None => {
            let inst = get_current_request_path();
            let err = invalid_json_error(inst);
            set_last_error(err.status_code() as i32, err.to_json_string());
            return std::ptr::null_mut();
        }
    };

    // Parse validator spec entries: name:type:validators
    #[derive(Clone)]
    struct FieldSpec {
        name: String,
        ty: String,
        validators: String,
    }
    let mut specs = Vec::new();
    for entry in validator_str.split(';') {
        if entry.is_empty() {
            continue;
        }
        let parts: Vec<&str> = entry.split(':').collect();
        if parts.len() < 2 {
            continue;
        }
        let name = parts[0].to_string();
        let ty = parts.get(1).unwrap_or(&"Str").to_string();
        let validators = if parts.len() > 2 {
            parts[2..].join(":")
        } else {
            String::new()
        };
        specs.push(FieldSpec {
            name,
            ty,
            validators,
        });
    }

    // Type + required validation
    let mut field_errors: HashMap<String, FieldError> = HashMap::new();
    for spec in &specs {
        match obj.get(&spec.name) {
            Some(val) => {
                // Type check - strict type checking
                let ok = match spec.ty.as_str() {
                    "Str" | "String" => val.is_string(),
                    "Int" | "I32" | "I64" => {
                        // Must be integer JSON type only - reject strings and any floats
                        val.is_i64() || val.is_u64()
                    }
                    "Float" | "F32" | "F64" => {
                        // Accept any number for float
                        val.is_number()
                    }
                    "Bool" => val.is_boolean(),
                    "Array" => val.is_array(),
                    "Map" | "Struct" => val.is_object(),
                    _ => true,
                };
                if !ok {
                    let mut fe = FieldError::new("Type mismatch in request body".to_string())
                        .with_expected(spec.ty.clone());
                    fe = fe.with_received(json_type_name(val).to_string());

                    // Include the actual value in the error
                    let value_str = match val {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        _ => val.to_string(),
                    };
                    fe = fe.with_value(value_str);
                    field_errors.insert(spec.name.clone(), fe);
                } else if matches!(spec.ty.as_str(), "Str" | "String") {
                    // Check for empty strings on Str fields - all fields are required
                    if let Some(s) = val.as_str() {
                        if s.is_empty() {
                            let fe = FieldError::new(
                                "This field is required and cannot be empty".to_string(),
                            )
                            .with_error("required".to_string())
                            .with_value(String::new());
                            field_errors.insert(spec.name.clone(), fe);
                        }
                    }
                }
            }
            None => {
                // Missing required
                let fe = FieldError::new("This field is required".to_string())
                    .with_error("required".to_string());
                field_errors.insert(spec.name.clone(), fe);
            }
        }
    }

    if !field_errors.is_empty() {
        let inst = get_current_request_path();
        let err = if field_errors
            .values()
            .any(|fe| fe.error.as_deref() == Some("required"))
        {
            missing_field_error(inst, field_errors)
        } else {
            type_mismatch_error(inst, field_errors)
        };
        set_last_error(err.status_code() as i32, err.to_json_string());
        return std::ptr::null_mut();
    }

    // Decorator validation (422)
    if let Err(fields) = validate_json_value(&json_value, &struct_name_str, &validator_str) {
        let inst = get_current_request_path();
        let err = validation_failed_error(inst, fields);
        set_last_error(err.status_code() as i32, err.to_json_string());
        return std::ptr::null_mut();
    }

    // Allocate and return JSON string representation
    let json_str = Box::new(body_str);
    Box::into_raw(json_str) as *mut libc::c_void
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

// ===== PHASE 7: VALIDATION DECORATORS =====

/// Validate a JSON value against decorator specifications
/// Format: "field1:email;field2:min8|max100;field3:enum:a|b|c"
fn validate_json_value(
    json: &serde_json::Value,
    _struct_name: &str,
    validator_spec: &str,
) -> Result<(), HashMap<String, error::FieldError>> {
    let mut errors: HashMap<String, error::FieldError> = HashMap::new();

    if validator_spec.is_empty() {
        return Ok(());
    }

    // Parse validator spec
    for field_spec in validator_spec.split(';') {
        if field_spec.is_empty() {
            continue;
        }

        let parts: Vec<&str> = field_spec.split(':').collect();
        if parts.len() < 2 {
            continue;
        }

        let field_name = parts[0];
        let field_type = parts[1];
        // format is field:type:validator1|validator2...
        let validators = if parts.len() >= 3 {
            parts[2..].join(":")
        } else {
            String::new()
        };

        // Get field value from JSON
        let json_val = json.get(field_name);
        let field_value = match json_val {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            Some(serde_json::Value::Bool(b)) => b.to_string(),
            Some(serde_json::Value::Null) => String::new(),
            None => String::new(),
            Some(other) => {
                let fe = error::FieldError::new("Type mismatch in request body".to_string())
                    .with_expected("scalar".to_string())
                    .with_received(json_type_name(other).to_string());
                errors.insert(field_name.to_string(), fe);
                continue;
            }
        };

        // For numeric types, validate as numbers; for strings, validate as strings
        let is_numeric_type = matches!(field_type, "Int" | "I32" | "I64" | "Float" | "F32" | "F64");
        if let Some(fe) = validate_field_value(&field_value, &validators, is_numeric_type) {
            errors.insert(field_name.to_string(), fe);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validate a single field value against decorators
/// Returns Some(FieldError) when validation fails
fn validate_field_value(
    value: &str,
    validator_spec: &str,
    is_numeric: bool,
) -> Option<error::FieldError> {
    for validator in validator_spec.split('|') {
        if validator.is_empty() {
            continue;
        }

        if let Some(fe) = apply_validator(value, validator, is_numeric) {
            return Some(fe);
        }
    }

    None
}

/// Apply a single validator to a value, returning FieldError on failure
fn apply_validator(value: &str, validator: &str, is_numeric: bool) -> Option<FieldError> {
    if validator == "email" {
        let valid = value.contains('@') && value.contains('.');
        if !valid {
            return Some(
                FieldError::new("Invalid email format".to_string())
                    .with_rule("email".to_string())
                    .with_value(value.to_string()),
            );
        }
    } else if validator.starts_with("min") {
        if let Ok(min) = validator[3..].parse::<i64>() {
            if is_numeric {
                // Numeric validation for Int/Float fields
                if let Ok(num_val) = value.parse::<i64>() {
                    if num_val < min {
                        return Some(
                            FieldError::new(format!("Must be at least {}", min))
                                .with_rule(format!("min:{}", min))
                                .with_value(value.to_string()),
                        );
                    }
                }
            } else {
                // String length validation
                if value.len() < min as usize {
                    return Some(
                        FieldError::new(format!("Must be at least {} characters", min))
                            .with_rule(format!("min:{}", min))
                            .with_value(value.to_string()),
                    );
                }
            }
        }
    } else if validator.starts_with("max") {
        if let Ok(max) = validator[3..].parse::<i64>() {
            if is_numeric {
                // Numeric validation for Int/Float fields
                if let Ok(num_val) = value.parse::<i64>() {
                    if num_val > max {
                        return Some(
                            FieldError::new(format!("Maximum {} allowed", max))
                                .with_rule(format!("max:{}", max))
                                .with_value(value.to_string()),
                        );
                    }
                }
            } else {
                // String length validation
                if value.len() > max as usize {
                    return Some(
                        FieldError::new(format!("Maximum {} characters allowed", max))
                            .with_rule(format!("max:{}", max))
                            .with_value(value.to_string()),
                    );
                }
            }
        }
    } else if validator.starts_with("pattern:") {
        // Pattern validation would require regex crate; skip for now
    } else if validator.starts_with("enum:") {
        let allowed = validator[5..].split('|').collect::<Vec<_>>();
        if !allowed.contains(&value) {
            let expected = format!("enum:{}", allowed.join("|"));
            return Some(
                FieldError::new(format!("Must be one of: {}", allowed.join(", ")))
                    .with_rule(format!("enum:{}", allowed.join("|")))
                    .with_value(value.to_string())
                    .with_expected(expected),
            );
        }
    } else if validator == "required" {
        if value.is_empty() {
            return Some(
                FieldError::new("This field is required".to_string())
                    .with_rule("required".to_string()),
            );
        }
    } else if validator == "optional" {
        // Always ok
    }
    None
}

/// Validate struct field value
/// FFI signature: doohttp_validate_field_value(value, field_name, validators) -> bool
#[no_mangle]
pub extern "C" fn doohttp_validate_field_value(
    value: *const libc::c_char,
    field_name: *const libc::c_char,
    validators: *const libc::c_char,
) -> libc::c_int {
    if value.is_null() || field_name.is_null() || validators.is_null() {
        return 0; // Invalid
    }

    let value_str = unsafe {
        std::ffi::CStr::from_ptr(value)
            .to_string_lossy()
            .to_string()
    };
    let validator_str = unsafe {
        std::ffi::CStr::from_ptr(validators)
            .to_string_lossy()
            .to_string()
    };

    if validate_field_value(&value_str, &validator_str, false).is_none() {
        1 // Valid
    } else {
        0 // Invalid
    }
}

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
                    let param = ParameterError::new(param_name_str.clone())
                        .with_expected("Int".to_string())
                        .with_received(value.clone());
                    let err = invalid_path_param_type_error(get_current_request_path(), param);
                    set_last_error(err.status_code() as i32, err.to_json_string());
                    std::ptr::null()
                }
            }
            "Float" => {
                if value.parse::<f64>().is_ok() {
                    string_to_c(value)
                } else {
                    let param = ParameterError::new(param_name_str.clone())
                        .with_expected("Float".to_string())
                        .with_received(value.clone());
                    let err = invalid_path_param_type_error(get_current_request_path(), param);
                    set_last_error(err.status_code() as i32, err.to_json_string());
                    std::ptr::null()
                }
            }
            "Bool" => {
                if value == "true" || value == "false" {
                    string_to_c(value)
                } else {
                    let param = ParameterError::new(param_name_str.clone())
                        .with_expected("Bool".to_string())
                        .with_received(value.clone());
                    let err = invalid_path_param_type_error(get_current_request_path(), param);
                    set_last_error(err.status_code() as i32, err.to_json_string());
                    std::ptr::null()
                }
            }
            _ => string_to_c(value), // String or other types
        }
    } else {
        let param = ParameterError::new(param_name_str.clone())
            .with_message("Path parameter 'id' is required".to_string());
        let err = missing_path_param_error(get_current_request_path(), param);
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

    // Allocate and return struct representation as JSON
    let json_str = Box::new(format!("{:?}", query_map));
    Box::into_raw(json_str) as *mut libc::c_void
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
        return error::ErrorType::InternalError.status_code() as i32;
    }

    let variant_str = unsafe {
        std::ffi::CStr::from_ptr(variant)
            .to_string_lossy()
            .to_string()
    };

    // Map error variants to status codes using centralized ErrorType
    use error::ErrorType;
    let error_type = match variant_str.as_str() {
        "NotFound" => ErrorType::NotFound,
        "InvalidInput" | "ValidationError" => ErrorType::UnprocessableEntity,
        "Unauthorized" => ErrorType::Unauthorized,
        "Forbidden" => ErrorType::Forbidden,
        "Conflict" | "AlreadyExists" => ErrorType::Conflict,
        "BadRequest" => ErrorType::BadRequest,
        "MethodNotAllowed" => ErrorType::MethodNotAllowed,
        "TooManyRequests" => ErrorType::TooManyRequests,
        "NotImplemented" => ErrorType::NotImplemented,
        "BadGateway" => ErrorType::BadGateway,
        "ServiceUnavailable" => ErrorType::ServiceUnavailable,
        _ => ErrorType::InternalError,
    };

    error_type.status_code() as i32
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

    // Generate detailed messages dynamically based on error type
    use error::ErrorType;
    let (error_type, message) = match variant_str.as_str() {
        "NotFound" => (ErrorType::NotFound, "The requested resource does not exist"),
        "InvalidInput" => (ErrorType::UnprocessableEntity, "Invalid input provided"),
        "ValidationError" => (ErrorType::UnprocessableEntity, "Validation failed"),
        "Unauthorized" => (
            ErrorType::Unauthorized,
            "Authentication required or token invalid",
        ),
        "Forbidden" => (
            ErrorType::Forbidden,
            "You do not have permission to access this resource",
        ),
        "Conflict" => (ErrorType::Conflict, "Resource conflict"),
        "AlreadyExists" => (ErrorType::Conflict, "Resource already exists"),
        "BadRequest" => (ErrorType::BadRequest, "Request is malformed or invalid"),
        "MethodNotAllowed" => (
            ErrorType::MethodNotAllowed,
            "Method not allowed for this resource",
        ),
        "TooManyRequests" => (
            ErrorType::TooManyRequests,
            "Too many requests, please try again later",
        ),
        "NotImplemented" => (
            ErrorType::NotImplemented,
            "This feature is not yet implemented",
        ),
        "BadGateway" => (ErrorType::BadGateway, "Bad gateway"),
        "ServiceUnavailable" => (
            ErrorType::ServiceUnavailable,
            "Service temporarily unavailable",
        ),
        _ => (ErrorType::InternalError, "An unexpected error occurred"),
    };

    // Could also use error_type.title() for more standardized messages
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
    println!("DEBUG RFC7807: instance pointer address: {:?}", instance);
    let instance_str = if instance.is_null() {
        let path = get_current_request_path();
        println!(
            "DEBUG RFC7807: instance is NULL, using thread-local path: {}",
            path
        );
        path
    } else {
        let path = unsafe {
            std::ffi::CStr::from_ptr(instance)
                .to_string_lossy()
                .to_string()
        };
        println!(
            "DEBUG RFC7807: path string: '{}', length: {}, bytes: {:?}",
            path,
            path.len(),
            path.as_bytes()
        );
        // Check for sentinel string or empty string
        if path.is_empty() || path == "__USE_THREAD_LOCAL_REQUEST_PATH_FROM_STORAGE_PLEASE__" {
            let thread_path = get_current_request_path();
            println!(
                "DEBUG RFC7807: instance is EMPTY or SENTINEL, using thread-local path: {}",
                thread_path
            );
            thread_path
        } else {
            println!("DEBUG RFC7807: instance is NOT NULL, provided: {}", path);
            path
        }
    };

    println!(
        "DEBUG RFC7807: Creating error - status={}, detail={}, instance={}",
        status, detail_str, instance_str
    );

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

/// Create RFC 7807 bad request with fields (for deserialization errors)
/// FFI signature: doohttp_error_rfc7807_bad_request_with_fields(detail, instance, fields_json) -> json_string
#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807_bad_request_with_fields(
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

    let error_response = bad_request(detail_str, instance_str).with_fields(fields);
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

    println!(
        "DEBUG RFC7807 AUTO: Creating error - status={}, detail={}, instance={}",
        status, detail_str, instance_str
    );

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_registration() {
        extern "C" fn dummy_handler(_req: *mut DooRequest) -> *mut DooResult {
            std::ptr::null_mut()
        }

        let routes = get_routes();
        let mut registry = routes.lock().unwrap();
        registry.register("GET", "/test", dummy_handler);
        assert!(registry.routes.contains_key("GET"));
    }

    #[test]
    fn test_query_parsing() {
        let query = "name=John&age=30";
        let params = parse_query(query);
        assert_eq!(params.get("name"), Some(&"John".to_string()));
        assert_eq!(params.get("age"), Some(&"30".to_string()));
    }

    // #[test]
    // fn test_email_validation() {
    //     assert!(apply_validator("test@example.com", "email").is_none());
    //     assert!(apply_validator("not-an-email", "email").is_some());
    // }

    // #[test]
    // fn test_min_max_validation() {
    //     assert!(apply_validator("12345678", "min8").is_none());
    //     assert!(apply_validator("short", "min8").is_some());
    //     assert!(apply_validator("short", "max10").is_none());
    //     assert!(apply_validator("this is very long text", "max10").is_some());
    // }

    // #[test]
    // fn test_enum_validation() {
    //     assert!(apply_validator("admin", "enum:user|admin|mod").is_none());
    //     assert!(apply_validator("superadmin", "enum:user|admin|mod").is_some());
    // }

    #[test]
    fn test_error_mapping() {
        use std::ffi::CString;

        let error_type = CString::new("UserError").unwrap();
        let not_found = CString::new("NotFound").unwrap();
        let invalid = CString::new("InvalidInput").unwrap();

        assert_eq!(
            doohttp_error_to_status(error_type.as_ptr(), not_found.as_ptr()),
            404
        );
        assert_eq!(
            doohttp_error_to_status(error_type.as_ptr(), invalid.as_ptr()),
            422
        );
    }
}
