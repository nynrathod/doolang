//! HTTP Server FFI for Doo language
//! Phase 3, 4, 5: Complete implementation with closures, JSON, groups, middleware

mod error;

use serde::Serialize;
use serde_json::json;
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

use chrono::Local;
use std::sync::RwLock;
use std::thread;
use std::time::{Duration, Instant};

use error::not_found; // Thread-local RFC 7807 last error (status, json body) populated by parsing/param helpers
thread_local! {
    static LAST_RFC_ERROR: RefCell<Option<(i32, String)>> = RefCell::new(None);
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
    println!(
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
struct Route {
    handler: DooHandlerFn,
    middleware: Vec<DooMiddlewareFn>,
}

/// Handler metadata for validation (includes decorators)
#[derive(Clone, Debug)]
struct HandlerMetadata {
    param_types: Vec<String>,
    struct_decorators: HashMap<String, HashMap<String, Vec<DecoratorInfo>>>,
    #[allow(dead_code)]
    struct_fields: HashMap<String, Vec<Vec<String>>>,
    struct_layouts: serde_json::Value,
    return_type: String,
}

#[derive(Clone, Debug, Serialize)]
struct DecoratorInfo {
    name: String,
    args: Vec<String>,
}

/// Route registry storing method -> router with handlers
struct RouteRegistry {
    routes: HashMap<String, Router<Route>>,  // method -> router
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
        Self {
            routes: HashMap::new(),
            handlers: HashMap::new(),
            handler_metadata: HashMap::new(),
            middleware: Vec::new(),
            middleware_handlers: HashMap::new(),
            groups: HashMap::new(),
            route_count: 0,
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
            self.route_count += 1;
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
            self.route_count += 1;
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

            let metadata = HandlerMetadata {
                param_types,
                struct_decorators,
                struct_fields,
                struct_layouts,
                return_type,
            };
            registry
                .handler_metadata
                .insert(handler_name.clone(), metadata);
        }
    }

    println!(
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
        // Auto-register JWT middleware if referenced
        if mw_name == "jwt" && !registry.middleware_handlers.contains_key("jwt") {
            registry
                .middleware_handlers
                .insert("jwt".to_string(), jwt_middleware_handler);
        }

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
        // Auto-register JWT middleware if referenced
        if mw_name == "jwt" && !registry.middleware_handlers.contains_key("jwt") {
            registry
                .middleware_handlers
                .insert("jwt".to_string(), jwt_middleware_handler);
        }

        if let Some(mw_fn) = registry.middleware_handlers.get(&mw_name).copied() {
            middleware_fns.push(mw_fn);
        } else {
            eprintln!("Warning: Middleware {} not found", mw_name);
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
        } else {
            eprintln!("Warning: Middleware {} not found", mw_name);
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
        } else {
            eprintln!("Warning: Middleware {} not found", mw_name);
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
        } else {
            eprintln!("Warning: Middleware {} not found", mw_name);
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
        println!("✓ Registered global middleware: {}", middleware_str);
    } else {
        eprintln!("Warning: Middleware {} not found", middleware_str);
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
                println!("← 405 Method Not Allowed");
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
        }
    };

    let handler = route.handler;
    let middleware = route.middleware.clone();
    let global_middleware = registry.middleware.clone();

    // Get handler metadata for validation (need to find handler name from registry)
    let _handler_metadata = registry
        .handlers
        .iter()
        .find(|(_, &h)| h as usize == handler as usize)
        .and_then(|(name, _)| registry.handler_metadata.get(name))
        .cloned();

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
        handler(req_ptr)
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

    let elapsed = req_start.elapsed();
    if cfg!(debug_assertions) || std::env::var("DOO_DEBUG").is_ok() {
        println!(
            "[DEBUG] {} {} took {} ms",
            method,
            path,
            elapsed.as_millis()
        );
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
            string_to_c("0.0.0.0")
        }
    } else {
        string_to_c("0.0.0.0")
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
    }

    server_ptr as *mut std::ffi::c_void
}

#[no_mangle]
pub extern "C" fn doo_http_listen(server_ptr: *const std::ffi::c_void) -> *mut DooResult {
    // Extract port from Server struct
    // Server struct layout: { Port: i32, Host: *const c_char }

    let startup_start = std::time::Instant::now();

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

    let addr = SocketAddr::from(([0, 0, 0, 0], port as u16));

    // Print all registered routes and handler count
    let routes = get_routes();
    let registry = routes.lock().unwrap();

    let total_routes = registry.route_count;

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

        // Now that the socket is bound, compute real boot time
        let boot_time_ms = startup_start.elapsed().as_millis();

        init_timestamp_updater();

        // Print banner AFTER bind so boot_time_ms is meaningful
        // Cyan color ANSI escape code: \x1b[36m ... \x1b[0m
        println!();
        println!("\x1b[36m  ____              ");
        println!(" |  _ \\  ___   ___  ");
        println!(" | | | |/ _ \\ / _ \\ ");
        println!(" | |_| | (_) | (_) |");
        println!(" |____/ \\___/ \\___/          Doo v{}\x1b[0m", VERSION);
        println!("--------------------------------------------------");
        println!();

        println!("Info Server Online");
        println!("--------------------------------------------------");
        println!("• Boot Time:            {} ms", boot_time_ms);
        println!("• Listening on:         http://0.0.0.0:{}", port);
        println!("• Handlers Loaded:      {}", total_routes);
        println!("• Process ID:           {}", std::process::id());
        println!("--------------------------------------------------");
        println!("🚀 Server Started on http://0.0.0.0:{}\n", port);

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

    // Allocate and return JSON string representation
    let json_str = Box::new(body_str);
    Box::into_raw(json_str) as *mut libc::c_void
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
        return Box::into_raw(Box::new(DooResponse {
            status: 500,
            body: string_to_c(
                r#"{"type":"internal_error","title":"Internal Server Error","status":500,"detail":"Invalid parameters to validation handler"}"#,
            ),
            content_type: string_to_c("application/json"),
        }));
    }

    let body_str = unsafe { CStr::from_ptr(body_json).to_string_lossy().to_string() };
    let path_str = unsafe { CStr::from_ptr(request_path).to_string_lossy().to_string() };
    let metadata_str = unsafe { CStr::from_ptr(metadata_json).to_string_lossy().to_string() };

    // Parse metadata JSON
    let metadata: serde_json::Value = match serde_json::from_str(&metadata_str) {
        Ok(m) => m,
        Err(_) => {
            let err = internal_error("Metadata parse error".to_string(), path_str);
            return Box::into_raw(Box::new(DooResponse {
                status: 500,
                body: string_to_c(&err.to_json_string()),
                content_type: string_to_c("application/json"),
            }));
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
            return Box::into_raw(Box::new(DooResponse {
                status: 400,
                body: string_to_c(&err.to_json_string()),
                content_type: string_to_c("application/json"),
            }));
        }
        Err(_) => {
            let err = bad_request("Invalid JSON format".to_string(), path_str);
            return Box::into_raw(Box::new(DooResponse {
                status: 400,
                body: string_to_c(&err.to_json_string()),
                content_type: string_to_c("application/json"),
            }));
        }
    };

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
            return Box::into_raw(Box::new(DooResponse {
                status: 422,
                body: string_to_c(&body_json),
                content_type: string_to_c("application/json"),
            }));
        }
    }

    // Validation passed - return success with body as-is (handler will process via wrapper)
    // The actual handler call happens in generated code, we just validated and return OK to proceed
    Box::into_raw(Box::new(DooResponse {
        status: 0, // Special status 0 means "validation passed, proceed with handler"
        body: string_to_c(&body_str),
        content_type: string_to_c("application/json"),
    }))
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
        return Box::into_raw(Box::new(DooResponse {
            status: 400,
            body: string_to_c(
                r#"{"type":"bad_request","title":"Bad Request","status":400,"detail":"Invalid parameters"}"#,
            ),
            content_type: string_to_c("application/json"),
        }));
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
            return Box::into_raw(Box::new(DooResponse {
                status: 400,
                body: string_to_c(&err.to_json_string()),
                content_type: string_to_c("application/json"),
            }));
        }
        Err(_) => {
            let err = bad_request("Invalid JSON format".to_string(), path_str);
            return Box::into_raw(Box::new(DooResponse {
                status: 400,
                body: string_to_c(&err.to_json_string()),
                content_type: string_to_c("application/json"),
            }));
        }
    };

    // Parse metadata JSON
    let metadata: serde_json::Value = match serde_json::from_str(&metadata_str) {
        Ok(m) => m,
        Err(_) => {
            let err = internal_error("Metadata parse error".to_string(), path_str);
            return Box::into_raw(Box::new(DooResponse {
                status: 500,
                body: string_to_c(&err.to_json_string()),
                content_type: string_to_c("application/json"),
            }));
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
            return Box::into_raw(Box::new(DooResponse {
                status: 422,
                body: string_to_c(&body_json),
                content_type: string_to_c("application/json"),
            }));
        }
    }

    // Validation passed - return success with parsed JSON body
    Box::into_raw(Box::new(DooResponse {
        status: 200,
        body: string_to_c(&body_str),
        content_type: string_to_c("application/json"),
    }))
}

#[no_mangle]
pub extern "C" fn doohttp_format_validation_error(
    validation_error_json: *const libc::c_char,
    request_path: *const libc::c_char,
) -> *mut DooResponse {
    if validation_error_json.is_null() || request_path.is_null() {
        // Return generic 422 error
        return Box::into_raw(Box::new(DooResponse {
            status: 422,
            body: string_to_c(
                r#"{"type":"validation_error","title":"Validation Failed","status":422,"detail":"Validation error occurred"}"#,
            ),
            content_type: string_to_c("application/json"),
        }));
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

        Box::into_raw(Box::new(DooResponse {
            status: 422,
            body: string_to_c(&body_json),
            content_type: string_to_c("application/json"),
        }))
    } else {
        // Failed to parse, return generic error
        Box::into_raw(Box::new(DooResponse {
            status: 422,
            body: string_to_c(
                r#"{"type":"validation_error","title":"Validation Failed","status":422,"detail":"Validation error occurred"}"#,
            ),
            content_type: string_to_c("application/json"),
        }))
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

            let field_value: serde_json::Value = match field_type {
                "Int" => {
                    let val = *(field_ptr as *const i32);
                    serde_json::json!(val)
                }
                "Float" => {
                    let val = *(field_ptr as *const f64);
                    serde_json::json!(val)
                }
                "Bool" => {
                    let val = *(field_ptr as *const i32);
                    serde_json::json!(val != 0)
                }
                "Str" => {
                    let str_ptr = *(field_ptr as *const *const libc::c_char);
                    if str_ptr.is_null() {
                        serde_json::json!("")
                    } else {
                        let c_str = CStr::from_ptr(str_ptr);
                        let rust_str = c_str.to_string_lossy().to_string();
                        serde_json::json!(rust_str)
                    }
                }
                _ => serde_json::json!(null),
            };

            json_obj.insert(field_name.to_string(), field_value);
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

    // Determine source based on source_type:
    // 0 = body (JSON), 1 = path params, 2 = query params
    let source_data: serde_json::Map<String, serde_json::Value> = match source_type {
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

    // Get struct name from param_types based on source_type:
    // source_type 0 = body (usually last param or only param)
    // source_type 1 = path (usually first param)
    // source_type 2 = query (usually first param or named with Query/Params)
    let struct_name = if !metadata.param_types.is_empty() {
        // Use heuristic: for path/query, use first param. For body, use last param if multiple, otherwise first
        let param_index = match source_type {
            1 | 2 => 0, // path/query use first param
            _ => {
                // body uses last param (or first if only one)
                if metadata.param_types.len() > 1 {
                    metadata.param_types.len() - 1
                } else {
                    0
                }
            }
        };
        metadata
            .param_types
            .get(param_index)
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string())
    } else {
        "Unknown".to_string()
    };

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
                if !source_data.contains_key(field_name) {
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
                if source_type == 1 || source_type == 2 {
                    if let Some(value) = source_data.get(field_name) {
                        if let Some(value_str) = value.as_str() {
                            let type_valid = match field_type {
                                "Int" => value_str.parse::<i64>().is_ok(),
                                "Float" => value_str.parse::<f64>().is_ok(),
                                "Bool" => value_str == "true" || value_str == "false",
                                "Str" => true,
                                _ => true,
                            };

                            if !type_valid {
                                let err = match source_type {
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

    // Validate field types for JSON body (source_type == 0)
    if source_type == 0 {
        if let Some(struct_layout_obj) =
            struct_layouts.get(&struct_name).and_then(|v| v.as_object())
        {
            if let Some(fields_array) = struct_layout_obj.get("fields").and_then(|f| f.as_array()) {
                let mut type_errors = std::collections::HashMap::new();

                for field_info in fields_array {
                    if let Some(field_obj) = field_info.as_object() {
                        let field_name =
                            field_obj.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let expected_type =
                            field_obj.get("type").and_then(|t| t.as_str()).unwrap_or("");

                        if let Some(field_value) = source_data.get(field_name) {
                            let type_matches = match expected_type {
                                "Int" => field_value.is_i64() || field_value.is_u64(),
                                "Float" => field_value.is_f64(),
                                "Bool" => field_value.is_boolean(),
                                "Str" => field_value.is_string(),
                                _ => true, // Unknown types pass
                            };

                            if !type_matches {
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

                if !type_errors.is_empty() {
                    let err = type_mismatch_error(path_str.clone(), type_errors);
                    set_last_error(err.status_code() as i32, err.to_json_string());
                    return 400;
                }
            }
        }
    }

    // Validate decorators
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

                    for decorator in decorators {
                        let decorators_json = serde_json::to_string(&vec![serde_json::json!({
                            "name": decorator.name,
                            "args": decorator.args
                        })])
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

                                        let mut validation_errors =
                                            std::collections::HashMap::new();
                                        let field_error = FieldError::new(field_name.clone())
                                            .with_rule(rule.to_string())
                                            .with_value(value_str.clone())
                                            .with_error(message.to_string());
                                        validation_errors.insert(field_name.clone(), field_error);

                                        dooruntime_free_string(validation_error_json_ptr);

                                        // Validation failed - store error and return error code
                                        let err = validation_failed_error(
                                            path_str.clone(),
                                            validation_errors,
                                        );
                                        set_last_error(
                                            err.status_code() as i32,
                                            err.to_json_string(),
                                        );
                                        return 422; // Unprocessable Entity
                                    }
                                }
                                dooruntime_free_string(error_ptr as *mut _);
                            }
                        }
                    }
                }
            }
        }
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

                    if let Some(field_value) = source_data.get(field_name) {
                        unsafe {
                            match field_type {
                                "Str" => {
                                    // Handle both direct strings and strings that need parsing
                                    let s = if let Some(str_val) = field_value.as_str() {
                                        str_val
                                    } else {
                                        ""
                                    };
                                    let c_string = CString::new(s).unwrap();
                                    let str_ptr = c_string.into_raw();
                                    std::ptr::write(
                                        struct_ptr_u8.add(offset) as *mut *const libc::c_char,
                                        str_ptr,
                                    );
                                }
                                "Int" => {
                                    // Only accept JSON numbers, reject strings
                                    if let Some(num) = field_value.as_i64() {
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
                                    // Only accept JSON numbers, reject strings
                                    if let Some(num) = field_value.as_f64() {
                                        std::ptr::write(struct_ptr_u8.add(offset) as *mut f64, num);
                                    } else if let Some(num) = field_value.as_i64() {
                                        std::ptr::write(
                                            struct_ptr_u8.add(offset) as *mut f64,
                                            num as f64,
                                        );
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
                                    // Only accept JSON booleans, reject strings
                                    if let Some(bool_val) = field_value.as_bool() {
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

static AUTH_METADATA: OnceLock<Mutex<HashMap<String, AuthMetadata>>> = OnceLock::new();
static CRUD_METADATA: OnceLock<Mutex<HashMap<String, CrudMetadata>>> = OnceLock::new();

fn get_auth_metadata() -> &'static Mutex<HashMap<String, AuthMetadata>> {
    AUTH_METADATA.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_crud_metadata() -> &'static Mutex<HashMap<String, CrudMetadata>> {
    CRUD_METADATA.get_or_init(|| Mutex::new(HashMap::new()))
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
        expires_seconds: i64,
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
        println!("[DEBUG] auth_signup_handler called");

        if request.is_null() {
            println!("[ERROR] Null request");
            return create_error_result(500, "Internal error: null request");
        }

        let req = &*request;
        if req.path.is_null() || req.body.is_null() {
            println!("[ERROR] Invalid request pointers");
            return create_error_result(500, "Internal error: invalid request");
        }

        let path = c_to_string(req.path);
        let body = c_to_string(req.body);

        println!("[DEBUG] Path: {}", path);
        println!("[DEBUG] Body: {}", body);

        // Parse JSON body
        let mut json: serde_json::Value = match serde_json::from_str(&body) {
            Ok(j) => {
                println!("[DEBUG] JSON parsed successfully");
                j
            }
            Err(e) => {
                println!("[ERROR] JSON parse failed: {}", e);
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
        println!(
            "[DEBUG] Auth metadata paths available: {:?}",
            metadata_map
                .values()
                .map(|m| &m.signup_path)
                .collect::<Vec<_>>()
        );
        let auth_meta = metadata_map
            .values()
            .find(|m| m.signup_path == path)
            .cloned();
        drop(metadata_map);

        let auth_meta = match auth_meta {
            Some(m) => {
                println!("[DEBUG] Found auth metadata for table: {}", m.table_name);
                m
            }
            None => {
                println!("[ERROR] No auth metadata found for path: {}", path);
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

            // Get field value (case-insensitive lookup)
            let value = obj
                .iter()
                .find(|(k, _)| k.to_lowercase() == field_name.to_lowercase())
                .map(|(_, v)| v)
                .or_else(|| obj.get(field_name));

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

        // Validate required fields (fields without @auto or @default)
        let mut missing_fields = Vec::new();
        for field in fields.iter() {
            if let Some(field_obj) = field.as_object() {
                if let Some(field_name) = field_obj.get("name").and_then(|n| n.as_str()) {
                    // Check if field has @auto or @default decorator
                    let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
                    let has_auto_or_default = if let Some(decs) = decorators {
                        decs.iter().any(|d| {
                            let dec_name = d
                                .as_object()
                                .and_then(|o| o.get("name"))
                                .and_then(|n| n.as_str());
                            dec_name == Some("auto") || dec_name == Some("default")
                        })
                    } else {
                        false
                    };

                    // If field is required (no @auto, no @default) and missing from request
                    if !has_auto_or_default && !obj.contains_key(&field_name.to_lowercase()) {
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
            Some(name) => {
                println!("[DEBUG] Password field: {}", name);
                name
            }
            None => {
                println!("[ERROR] No password field with @hash decorator found");
                return create_error_result(500, "No password field with @hash decorator found");
            }
        };

        // Get password value (case-insensitive lookup)
        let password_value = obj
            .iter()
            .find(|(k, _)| k.to_lowercase() == password_field_name.to_lowercase())
            .and_then(|(_, v)| v.as_str())
            .or_else(|| obj.get(password_field_name).and_then(|v| v.as_str()));

        let password_value = match password_value {
            Some(pwd) => pwd,
            None => {
                return create_error_result(
                    400,
                    &format!("Missing or invalid field: {}", password_field_name),
                );
            }
        };

        // Hash password using libdoo_auth
        let password_c = CString::new(password_value).unwrap();
        let hash_result = doo_auth_hash_password(password_c.as_ptr());

        if hash_result.is_null() {
            println!("[ERROR] Hash result is null");
            return create_error_result(500, "Failed to hash password");
        }

        let hash_res = &*(hash_result as *mut DooResult);
        if hash_res.tag != 0 {
            println!("[ERROR] Hash result tag is non-zero: {}", hash_res.tag);
            doo_auth_free_result(hash_result);
            return create_error_result(500, "Failed to hash password");
        }

        let hashed_password = if hash_res.value.is_null() {
            println!("[ERROR] Hash value is null");
            doo_auth_free_result(hash_result);
            return create_error_result(500, "Failed to get hashed password");
        } else {
            let hash_ptr = hash_res.value as *mut c_char;
            let hash_str = CStr::from_ptr(hash_ptr).to_string_lossy().into_owned();
            doo_auth_free_result(hash_result);
            println!("[DEBUG] Password hashed successfully");
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

            // Use hashed password for password field (case-insensitive comparison)
            if field_name.to_lowercase() == password_field_name.to_lowercase() {
                field_names.push(field_name.to_lowercase());
                placeholders.push(format!("${}", param_idx));
                values_json.push(serde_json::Value::String(hashed_password.clone()));
                param_idx += 1;
            } else {
                // Case-insensitive lookup for the field value
                let value = obj
                    .iter()
                    .find(|(k, _)| k.to_lowercase() == field_name.to_lowercase())
                    .map(|(_, v)| v)
                    .or_else(|| obj.get(field_name));

                if let Some(value) = value {
                    field_names.push(field_name.to_lowercase());
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
                            field_names.push(field_name.to_lowercase());
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

        println!("[DEBUG] SQL: {}", sql);
        println!("[DEBUG] Values: {:?}", values_json);

        let sql_c = CString::new(sql.clone()).unwrap();
        let values_json_str = serde_json::to_string(&values_json).unwrap();
        let values_c = CString::new(values_json_str.clone()).unwrap();

        // Insert into database
        println!("[DEBUG] Calling doo_db_insert_json...");
        let insert_result = doo_db_insert_json(sql_c.as_ptr(), values_c.as_ptr());

        if insert_result.is_null() {
            println!("[ERROR] Insert result is null");
            return create_error_result(500, "Database insert failed");
        }

        let _insert_res = &*insert_result;
        let insert_res = &*(insert_result as *mut DooResult);
        let is_error = doo_db_is_error(insert_result);

        if is_error != 0 {
            println!("[ERROR] Database insert failed, is_error: {}", is_error);
            let err_msg_ptr = doo_db_get_error_message(insert_result);
            let err_msg = if err_msg_ptr.is_null() {
                "Database insert failed".to_string()
            } else {
                let msg = CStr::from_ptr(err_msg_ptr).to_string_lossy().into_owned();
                doo_db_free_string(err_msg_ptr);
                msg
            };
            println!("[ERROR] DB Error message: {}", err_msg);
            doo_db_result_free(insert_result);

            // Convert DB error to RFC 7807 format
            let (status, rfc_error) = convert_db_error_to_rfc7807(&err_msg, path.clone());
            return create_json_result(status, &rfc_error);
        }

        println!("[DEBUG] Insert successful");

        // Extract inserted ID
        let user_id = if insert_res.value.is_null() {
            println!("[WARN] Insert value is null, using default ID 1");
            1i64
        } else {
            insert_res.value as i64
        };
        println!("[DEBUG] User ID: {}", user_id);
        doo_db_result_free(insert_result);

        // Generate JWT token
        let user_id_str = user_id.to_string();
        let sub_c = CString::new(user_id_str.clone()).unwrap();
        let user_data = json!({
            "id": user_id,
        });
        let data_json_str = user_data.to_string();
        let data_c = CString::new(data_json_str).unwrap();
        let expires = 86400i64; // 24 hours

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

        // Build user object (get fields from metadata, exclude password)
        let mut user_obj = serde_json::Map::new();
        user_obj.insert("id".to_string(), json!(user_id));

        // Add other fields from request body (except password field)
        for field in fields.iter() {
            let field_obj = match field.as_object() {
                Some(o) => o,
                None => continue,
            };

            let field_name = match field_obj.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };

            // Skip password and auto fields (case-insensitive)
            if field_name.to_lowercase() == password_field_name.to_lowercase()
                || field_name.to_lowercase() == "id"
            {
                continue;
            }

            // Add field from original request (case-insensitive lookup)
            let value = obj
                .iter()
                .find(|(k, _)| k.to_lowercase() == field_name.to_lowercase())
                .map(|(_, v)| v)
                .or_else(|| obj.get(field_name));

            if let Some(value) = value {
                user_obj.insert(field_name.to_lowercase(), value.clone());
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

                let mut fields = std::collections::HashMap::new();
                fields.insert(
                    field.to_string(),
                    error::FieldError::new(field.to_string())
                        .with_rule(rule.to_string())
                        .with_error(message.to_string())
                        .with_value(value.to_string()),
                );

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
        let identifier = obj
            .iter()
            .find(|(k, _)| k.to_lowercase() == unique_field.to_lowercase())
            .and_then(|(_, v)| v.as_str())
            .or_else(|| obj.get(unique_field).and_then(|v| v.as_str()));

        let identifier = match identifier {
            Some(id) => id,
            None => {
                return create_error_result(
                    400,
                    &format!("Missing or invalid field: {}", unique_field),
                );
            }
        };

        let password = obj
            .iter()
            .find(|(k, _)| k.to_lowercase() == password_field.to_lowercase())
            .and_then(|(_, v)| v.as_str())
            .or_else(|| obj.get(password_field).and_then(|v| v.as_str()));

        let password = match password {
            Some(pwd) => pwd,
            None => {
                return create_error_result(
                    400,
                    &format!("Missing or invalid field: {}", password_field),
                );
            }
        };

        // Query user from database (use lowercase column name for Postgres)
        let table_name = &auth_meta.table_name;
        let sql = format!(
            "SELECT * FROM {} WHERE {} = $1",
            table_name,
            unique_field.to_lowercase()
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

        let stored_hash = user_obj
            .iter()
            .find(|(k, _)| k.to_lowercase() == password_field.to_lowercase())
            .and_then(|(_, v)| v.as_str());

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
        let expires = 86400i64; // 24 hours

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

        // Build user response (exclude password - case-insensitive removal)
        let mut user_response = user_data.as_object().unwrap().clone();
        user_response.retain(|k, _| k.to_lowercase() != password_field.to_lowercase());

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
                        if let Some(field_value) = obj.get(&field_name.to_lowercase()) {
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
                    let has_auto_or_default = if let Some(decs) = decorators {
                        decs.iter().any(|d| {
                            let dec_name = d
                                .as_object()
                                .and_then(|o| o.get("name"))
                                .and_then(|n| n.as_str());
                            dec_name == Some("auto") || dec_name == Some("default")
                        })
                    } else {
                        false
                    };

                    // If field is required (no @auto, no @default) and missing from request
                    if !has_auto_or_default && !obj.contains_key(&field_name.to_lowercase()) {
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

            // Case-insensitive lookup for the field value
            let value = obj
                .iter()
                .find(|(k, _)| k.to_lowercase() == field_name.to_lowercase())
                .map(|(_, v)| v)
                .or_else(|| obj.get(field_name));

            if let Some(value) = value {
                field_names.push(field_name.to_lowercase());
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
                        // Apply default value
                        field_names.push(field_name.to_lowercase());
                        placeholders.push(format!("${}", param_idx));
                        values_json.push(serde_json::Value::String(default_value.to_string()));
                        param_idx += 1;
                    }
                }
            }
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

        // Build response with created resource
        let mut resource_obj = obj.clone();
        resource_obj.insert("id".to_string(), json!(user_id));

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
        let sql = format!("SELECT * FROM {}", table_name.to_lowercase());
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

        // Parse data array
        let _data_array: Vec<serde_json::Value> =
            serde_json::from_str(&data_json_str).unwrap_or_default();

        // Return array directly
        create_json_result(200, &data_json_str)
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
        let sql = format!(
            "SELECT * FROM {} WHERE id = {}",
            table_name.to_lowercase(),
            id_num
        );
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

        // Return data directly
        create_json_result(200, &data_json_str)
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
        let sql = format!(
            "DELETE FROM {} WHERE id = {}",
            table_name.to_lowercase(),
            id_num
        );
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

        // Return empty 204 No Content for successful delete
        let response = Box::into_raw(Box::new(DooResponse {
            status: 204,
            body: std::ptr::null_mut(),
            content_type: string_to_c("application/json"),
        }));
        Box::into_raw(Box::new(DooResult {
            tag: 0,
            value: response as *mut _,
        }))
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

            // Skip auto-generated and primary key fields
            let decorators = field_obj.get("decorators").and_then(|d| d.as_array());
            if let Some(decs) = decorators {
                let is_auto_or_primary = decs.iter().any(|d| {
                    let dec_name = d
                        .as_object()
                        .and_then(|o| o.get("name"))
                        .and_then(|n| n.as_str());
                    dec_name == Some("auto") || dec_name == Some("primary")
                });
                if is_auto_or_primary {
                    continue;
                }
            }

            // Case-insensitive lookup for the field value
            let value = obj
                .iter()
                .find(|(k, _)| k.to_lowercase() == field_name.to_lowercase())
                .map(|(_, v)| v)
                .or_else(|| obj.get(field_name));

            if let Some(value) = value {
                set_clauses.push(format!("{} = ${}", field_name.to_lowercase(), param_idx));
                values_json.push(value.clone());
                param_idx += 1;
            }
        }

        if set_clauses.is_empty() {
            return create_error_result(400, "No valid fields to update");
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

        // Build response with updated resource
        let mut resource_obj = obj.clone();
        resource_obj.insert("id".to_string(), json!(resource_id));

        create_json_result(200, &serde_json::to_string(&resource_obj).unwrap())
    }
}

// Helper to create JSON response
fn create_json_result(status: i32, body: &str) -> *mut DooResult {
    let response = Box::into_raw(Box::new(DooResponse {
        status,
        body: string_to_c(body),
        content_type: string_to_c("application/json"),
    }));
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: response as *mut _,
    }))
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

        let decorators = field_obj
            .get("decorators")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let mut is_primary = false;
        let mut is_auto = false;
        let mut is_unique = false;
        let mut is_hash = false;

        for decorator in &decorators {
            if let Some(dec_obj) = decorator.as_object() {
                if let Some(dec_name) = dec_obj.get("name").and_then(|n| n.as_str()) {
                    match dec_name {
                        "primary" => is_primary = true,
                        "auto" => is_auto = true,
                        "unique" => is_unique = true,
                        "hash" => is_hash = true,
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
                _ => "TEXT",
            }
        };

        // Use lowercase column names for Postgres compatibility
        let mut column_def = format!("{} {}", field_name.to_lowercase(), sql_type);

        if is_primary {
            column_def.push_str(" PRIMARY KEY");
        }
        // NOT NULL must come before UNIQUE in PostgreSQL
        if !is_auto && !is_primary {
            column_def.push_str(" NOT NULL");
        }
        if is_unique && !is_primary {
            column_def.push_str(" UNIQUE");
        }

        columns.push(column_def);
    }

    format!(
        "CREATE TABLE IF NOT EXISTS {} ({})",
        table_name,
        columns.join(", ")
    )
}

// ============================================================================
// JWT Middleware Implementation
// ============================================================================

/// JWT middleware function - verifies JWT token from Authorization header
/// Returns RFC 7807 error if unauthorized
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

        // Extract token from "Bearer <token>" format
        let token = if auth_header.starts_with("Bearer ") || auth_header.starts_with("bearer ") {
            auth_header[7..].trim()
        } else {
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
        };

        // Verify JWT token using libdoo_auth
        let token_c = CString::new(token).unwrap();
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

        // Token is valid, free the result and continue to next middleware/handler
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
        Box::into_raw(Box::new(DooResult {
            tag: 0,
            value: response as *mut std::ffi::c_void,
        }))
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
            println!("[ERROR] Invalid metadata JSON: {}", e);
            return make_err_http(400, &format!("Invalid metadata: {}", e));
        }
    };

    let table_name = struct_name_str.to_lowercase() + "s";

    // Create table from metadata
    let create_sql = build_create_table_sql(&table_name, &metadata);

    if !create_sql.is_empty() {
        let sql_c = CString::new(create_sql.clone()).unwrap();
        let create_result = unsafe { doo_db_create_table(std::ptr::null(), sql_c.as_ptr()) };

        if !create_result.is_null() {
            if unsafe { doo_db_is_error(create_result) } != 0 {
                let err_msg_ptr = unsafe { doo_db_get_error_message(create_result) };
                if !err_msg_ptr.is_null() {
                    let err_msg = unsafe { CStr::from_ptr(err_msg_ptr).to_string_lossy() };
                    println!("[WARN] Table creation warning: {}", err_msg);
                    unsafe { doo_db_free_string(err_msg_ptr) };
                }
                unsafe { doo_db_result_free(create_result) };
            } else {
                unsafe { doo_db_result_free(create_result) };
            }
        }
    }

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

    println!(
        "✓ Auth routes registered: POST {} and POST {}",
        signup_path_str, login_path_str
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
            println!("[ERROR] Invalid metadata JSON: {}", e);
            return make_err_http(400, &format!("Invalid metadata: {}", e));
        }
    };

    let table_name = struct_name_str.to_lowercase() + "s";

    // Create table from metadata
    let create_sql = build_create_table_sql(&table_name, &metadata);
    let sql_c = CString::new(create_sql.clone()).unwrap();
    let create_result = unsafe { doo_db_create_table(std::ptr::null(), sql_c.as_ptr()) };

    if !create_result.is_null() {
        if unsafe { doo_db_is_error(create_result) } != 0 {
            let err_msg_ptr = unsafe { doo_db_get_error_message(create_result) };
            if !err_msg_ptr.is_null() {
                let err_msg = unsafe { CStr::from_ptr(err_msg_ptr).to_string_lossy() };
                println!("[WARN] Table creation warning: {}", err_msg);
                unsafe { doo_db_free_string(err_msg_ptr) };
            }
            unsafe { doo_db_result_free(create_result) };
        } else {
            unsafe { doo_db_result_free(create_result) };
        }
    }

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
        println!("✓ CRUD routes registered (public - no auth):");
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
        println!("✓ CRUD routes registered (JWT auth required):");
    }
    println!("  POST {} (create)", base_path_str);
    println!("  GET {} (list)", base_path_str);
    println!("  GET {} (get)", id_path);
    println!("  PUT {} (update)", id_path);
    println!("  DELETE {} (delete)", id_path);

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

    println!(
        "DEBUG MIDDLEWARE ERROR: enum={}, tag={}, variant={}, instance={}",
        enum_str, variant_tag, variant_str, instance_str
    );

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

    // Return DooHttpError struct pointer, not just JSON string
    Box::into_raw(Box::new(DooHttpError {
        status,
        message: string_to_c(&error_json),
    }))
}
