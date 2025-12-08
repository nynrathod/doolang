//! HTTP Server FFI for Doo language
//! Uses hyper-rs for HTTP server and matchit for routing

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Global route registry for storing registered handlers
static ROUTES: OnceLock<Arc<Mutex<RouteRegistry>>> = OnceLock::new();

/// Route registry storing method -> path -> handler mappings
struct RouteRegistry {
    routes: HashMap<String, HashMap<String, String>>, // method -> path -> handler_name
}

impl RouteRegistry {
    fn new() -> Self {
        Self {
            routes: HashMap::new(),
        }
    }

    fn register(&mut self, method: &str, path: &str, handler: &str) {
        let method_routes = self.routes.entry(method.to_uppercase()).or_insert_with(HashMap::new);
        method_routes.insert(path.to_string(), handler.to_string());
    }

    fn get_handler(&self, method: &str, path: &str) -> Option<String> {
        self.routes
            .get(&method.to_uppercase())
            .and_then(|m| m.get(path).cloned())
    }
}

fn get_routes() -> Arc<Mutex<RouteRegistry>> {
    ROUTES.get_or_init(|| Arc::new(Mutex::new(RouteRegistry::new()))).clone()
}

/// Doo Result struct layout: { i32 tag, void* value }
/// tag = 0 for Ok, tag = 1 for Err
#[repr(C)]
pub struct DooResult {
    tag: i32,
    value: *mut std::ffi::c_void,
}

/// HTTP Error struct layout - matches Doo's HttpError struct
#[repr(C)]
pub struct DooHttpError {
    status: i32,
    message: *mut c_char,
}

// === Helper functions ===

fn string_to_c(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(cs) => cs.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

fn c_to_string(s: *const c_char) -> Result<String, String> {
    if s.is_null() {
        return Err("Null pointer".to_string());
    }
    unsafe {
        CStr::from_ptr(s)
            .to_str()
            .map(|s| s.to_string())
            .map_err(|e| format!("Invalid UTF-8: {}", e))
    }
}

fn make_ok_void() -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: std::ptr::null_mut(),
    }))
}

fn make_ok_string(s: String) -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: string_to_c(s) as *mut std::ffi::c_void,
    }))
}

fn make_err_http(status: i32, message: String) -> *mut DooResult {
    let error_struct = Box::new(DooHttpError {
        status,
        message: string_to_c(message),
    });
    Box::into_raw(Box::new(DooResult {
        tag: 1,
        value: Box::into_raw(error_struct) as *mut std::ffi::c_void,
    }))
}

// === Route Registration Functions ===

/// Register a GET route handler
#[no_mangle]
pub extern "C" fn doo_http_get(path: *const c_char, handler: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_http(500, e),
    };
    let handler_str = match c_to_string(handler) {
        Ok(s) => s,
        Err(e) => return make_err_http(500, e),
    };

    let routes = get_routes();
    routes.lock().unwrap().register("GET", &path_str, &handler_str);
    make_ok_void()
}

/// Register a POST route handler
#[no_mangle]
pub extern "C" fn doo_http_post(path: *const c_char, handler: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_http(500, e),
    };
    let handler_str = match c_to_string(handler) {
        Ok(s) => s,
        Err(e) => return make_err_http(500, e),
    };

    let routes = get_routes();
    routes.lock().unwrap().register("POST", &path_str, &handler_str);
    make_ok_void()
}

/// Register a PUT route handler
#[no_mangle]
pub extern "C" fn doo_http_put(path: *const c_char, handler: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_http(500, e),
    };
    let handler_str = match c_to_string(handler) {
        Ok(s) => s,
        Err(e) => return make_err_http(500, e),
    };

    let routes = get_routes();
    routes.lock().unwrap().register("PUT", &path_str, &handler_str);
    make_ok_void()
}

/// Register a DELETE route handler
#[no_mangle]
pub extern "C" fn doo_http_delete(path: *const c_char, handler: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_http(500, e),
    };
    let handler_str = match c_to_string(handler) {
        Ok(s) => s,
        Err(e) => return make_err_http(500, e),
    };

    let routes = get_routes();
    routes.lock().unwrap().register("DELETE", &path_str, &handler_str);
    make_ok_void()
}

/// Register a PATCH route handler
#[no_mangle]
pub extern "C" fn doo_http_patch(path: *const c_char, handler: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_http(500, e),
    };
    let handler_str = match c_to_string(handler) {
        Ok(s) => s,
        Err(e) => return make_err_http(500, e),
    };

    let routes = get_routes();
    routes.lock().unwrap().register("PATCH", &path_str, &handler_str);
    make_ok_void()
}

/// Start the HTTP server (placeholder - actual server implementation TBD)
/// The full hyper server integration requires callback mechanism from Doo runtime
#[no_mangle]
pub extern "C" fn doo_http_listen(port: i32) -> *mut DooResult {
    // For now, just print the routes that would be registered
    let routes = get_routes();
    let registry = routes.lock().unwrap();
    
    println!("HTTP Server would start on port {}", port);
    println!("Registered routes:");
    for (method, paths) in &registry.routes {
        for (path, handler) in paths {
            println!("  {} {} -> {}", method, path, handler);
        }
    }
    
    // TODO: Implement actual hyper server with callback mechanism
    // This requires Doo runtime support for calling back into Doo functions
    
    make_ok_void()
}

// === Request Helper Functions ===

/// Get a query parameter (placeholder)
#[no_mangle]
pub extern "C" fn doo_http_req_query(_name: *const c_char) -> *mut c_char {
    // Placeholder - actual implementation needs request context
    string_to_c(String::new())
}

/// Get a route parameter (placeholder)
#[no_mangle]
pub extern "C" fn doo_http_req_param(_name: *const c_char) -> *mut c_char {
    // Placeholder - actual implementation needs request context
    string_to_c(String::new())
}

/// Get a header value (placeholder)
#[no_mangle]
pub extern "C" fn doo_http_req_header(_name: *const c_char) -> *mut c_char {
    // Placeholder - actual implementation needs request context
    string_to_c(String::new())
}

/// Parse request body as JSON (placeholder)
#[no_mangle]
pub extern "C" fn doo_http_req_json(_type_name: *const c_char) -> *mut DooResult {
    // Placeholder - actual implementation needs request context and type info
    make_err_http(501, "JSON parsing not yet implemented".to_string())
}

// === Memory Management ===

#[no_mangle]
pub extern "C" fn doo_http_free_result(result: *mut DooResult) {
    if !result.is_null() {
        unsafe {
            let _ = Box::from_raw(result);
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_http_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            let _ = CString::from_raw(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_registration() {
        let path = CString::new("/users").unwrap();
        let handler = CString::new("getUsers").unwrap();
        
        let result = doo_http_get(path.as_ptr(), handler.as_ptr());
        assert!(!result.is_null());
        unsafe {
            assert_eq!((*result).tag, 0); // Ok
            doo_http_free_result(result);
        }
    }
}
