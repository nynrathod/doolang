//! Route Registry
//! High-performance routing with matchit, middleware chains, and handler metadata.

use crate::types::*;
use doo_ffi_core::ffi_debug;
use matchit::Router as MatchitRouter;
use std::collections::HashMap;
use std::sync::Mutex;

/// Route entry with handler and middleware
pub struct RouteEntry {
    pub handler: DooHandlerFn,
    pub handler_name: Option<String>,
    pub middleware: Vec<DooMiddlewareFn>,
}

/// Configuration for auth routes
pub struct AuthConfig {
    pub signup_path: String,
    pub login_path: String,
    pub user_struct: String,
}

/// Configuration for CRUD routes
pub struct CrudConfig {
    pub base_path: String,
    pub resource_struct: String,
}

/// Route registry with method-based routing
pub struct RouteRegistry {
    routers: HashMap<String, MatchitRouter<RouteEntry>>,
    pub global_middleware: Vec<DooMiddlewareFn>,
    pub middleware_handlers: HashMap<String, DooMiddlewareFn>,
    pub named_handlers: HashMap<String, DooHandlerFn>,
    pub handler_metadata: HashMap<String, HandlerMetadata>,
    pub route_count: usize,
    pub auth_config: Option<AuthConfig>,
    pub crud_configs: Vec<CrudConfig>,
}

impl RouteRegistry {
    pub fn new() -> Self {
        Self {
            routers: HashMap::new(),
            global_middleware: Vec::new(),
            middleware_handlers: HashMap::new(),
            named_handlers: HashMap::new(),
            handler_metadata: HashMap::new(),
            route_count: 0,
            auth_config: None,
            crud_configs: Vec::new(),
        }
    }

    /// Register a route with handler function
    pub fn register(&mut self, method: &str, path: &str, handler: DooHandlerFn) {
        self.register_with_name(method, path, handler, None);
    }

    /// Register a route with handler function and name
    pub fn register_with_name(
        &mut self,
        method: &str,
        path: &str,
        handler: DooHandlerFn,
        handler_name: Option<String>,
    ) {
        let method_upper = method.to_uppercase();
        let router = self
            .routers
            .entry(method_upper.clone())
            .or_insert_with(MatchitRouter::new);

        let entry = RouteEntry {
            handler,
            handler_name: handler_name.clone(),
            middleware: self.global_middleware.clone(),
        };

        match router.insert(path, entry) {
            Ok(_) => {
                self.route_count += 1;
                ffi_debug!(
                    "ROUTER",
                    "Registered: {} {} (total: {})",
                    method_upper,
                    path,
                    self.route_count
                );
            }
            Err(e) => {
                ffi_debug!(
                    "ROUTER",
                    "Failed to register {} {}: {:?}",
                    method_upper,
                    path,
                    e
                );
            }
        }
    }

    /// Register a route by handler name (looked up later)
    pub fn register_by_name(&mut self, method: &str, path: &str, handler_name: &str) {
        if let Some(&handler) = self.named_handlers.get(handler_name) {
            self.register_with_name(method, path, handler, Some(handler_name.to_string()));
        }
    }

    /// Register a route by handler name with middleware
    pub fn register_by_name_with_middleware(
        &mut self,
        method: &str,
        path: &str,
        handler_name: &str,
        middleware: Vec<DooMiddlewareFn>,
    ) {
        if let Some(&handler) = self.named_handlers.get(handler_name) {
            self.register_with_middleware_and_name(
                method,
                path,
                handler,
                middleware,
                Some(handler_name.to_string()),
            );
        }
    }

    /// Register a route with specific middleware
    pub fn register_with_middleware(
        &mut self,
        method: &str,
        path: &str,
        handler: DooHandlerFn,
        middleware: Vec<DooMiddlewareFn>,
    ) {
        self.register_with_middleware_and_name(method, path, handler, middleware, None);
    }

    /// Register a route with specific middleware and handler name
    pub fn register_with_middleware_and_name(
        &mut self,
        method: &str,
        path: &str,
        handler: DooHandlerFn,
        middleware: Vec<DooMiddlewareFn>,
        handler_name: Option<String>,
    ) {
        let method_upper = method.to_uppercase();
        let router = self
            .routers
            .entry(method_upper.clone())
            .or_insert_with(MatchitRouter::new);

        let mut mw = self.global_middleware.clone();
        mw.extend(middleware);

        let entry = RouteEntry {
            handler,
            handler_name: handler_name.clone(),
            middleware: mw,
        };

        match router.insert(path, entry) {
            Ok(_) => {
                self.route_count += 1;
                ffi_debug!(
                    "ROUTER",
                    "Registered (w/ middleware): {} {} (total: {})",
                    method_upper,
                    path,
                    self.route_count
                );
            }
            Err(e) => {
                ffi_debug!(
                    "ROUTER",
                    "Failed to register {} {}: {:?}",
                    method_upper,
                    path,
                    e
                );
            }
        }
    }

    /// Add global middleware
    pub fn add_middleware(&mut self, mw: DooMiddlewareFn) {
        if !self
            .global_middleware
            .iter()
            .any(|m| *m as usize == mw as usize)
        {
            self.global_middleware.push(mw);
        }
    }

    /// Register a named handler for later binding
    pub fn register_handler(&mut self, name: &str, handler: DooHandlerFn) {
        self.named_handlers.insert(name.to_string(), handler);
    }

    /// Register handler with metadata
    pub fn register_handler_with_metadata(
        &mut self,
        name: &str,
        handler: DooHandlerFn,
        metadata: HandlerMetadata,
    ) {
        self.named_handlers.insert(name.to_string(), handler);
        self.handler_metadata.insert(name.to_string(), metadata);
    }

    /// Match a route and extract parameters
    /// NOTE: `method` is expected to be uppercase (hyper always provides uppercase)
    pub fn match_route(
        &self,
        method: &str,
        path: &str,
    ) -> Option<(&RouteEntry, HashMap<String, String>)> {
        // hyper methods are already uppercase — skip to_uppercase() allocation
        ffi_debug!(
            "ROUTER",
            "Looking for {} in routers. Available methods: {:?}",
            method,
            self.routers.keys().collect::<Vec<_>>()
        );

        let router = match self.routers.get(method) {
            Some(r) => r,
            None => {
                ffi_debug!("ROUTER", "No router for method: {}", method);
                return None;
            }
        };

        ffi_debug!(
            "ROUTER",
            "Found router for {}, attempting match on '{}'",
            method,
            path
        );

        match router.at(path) {
            Ok(matched) => {
                let params: HashMap<String, String> = matched
                    .params
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                ffi_debug!(
                    "ROUTER",
                    "Matched: {} {} -> params: {:?}",
                    method,
                    path,
                    params
                );
                Some((matched.value, params))
            }
            Err(e) => {
                ffi_debug!("ROUTER", "No match for {} {}: {:?}", method, path, e);
                None
            }
        }
    }

    /// Get total number of registered routes
    pub fn count(&self) -> usize {
        self.route_count
    }

    /// Find all HTTP methods that have a matching route for the given path.
    /// Used to generate 405 Method Not Allowed responses with `allowed_methods`.
    pub fn find_allowed_methods(&self, path: &str) -> Vec<String> {
        let mut allowed = Vec::new();
        for (method, router) in &self.routers {
            if router.at(path).is_ok() {
                allowed.push(method.clone());
            }
        }
        allowed.sort();
        allowed
    }
}

// Global route registry — mutable during registration (before server starts)
static ROUTES: std::sync::OnceLock<Mutex<RouteRegistry>> = std::sync::OnceLock::new();

/// Frozen, lock-free registry used for ALL request-time lookups (after server starts)
static FROZEN_ROUTES: std::sync::OnceLock<RouteRegistry> = std::sync::OnceLock::new();

/// Get mutable route registry for registration (before server starts)
pub fn get_routes() -> &'static Mutex<RouteRegistry> {
    ROUTES.get_or_init(|| Mutex::new(RouteRegistry::new()))
}

/// Freeze the registry — move from Mutex to lock-free OnceLock.
/// Called exactly once before the accept loop starts.
/// After this, `get_frozen_routes()` provides zero-cost access.
pub fn freeze_routes() {
    let routes = get_routes();
    let mut guard = routes.lock().unwrap();
    let registry = std::mem::replace(&mut *guard, RouteRegistry::new());
    let _ = FROZEN_ROUTES.set(registry);
}

/// Request-time lookup: NO lock, NO contention.
/// Panics if `freeze_routes()` was not called.
#[inline]
pub fn get_frozen_routes() -> &'static RouteRegistry {
    FROZEN_ROUTES
        .get()
        .expect("Routes not frozen — call freeze_routes() before serving")
}
