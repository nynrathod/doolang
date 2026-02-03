//! Route Registry
//! High-performance routing with matchit, middleware chains, and handler metadata.

use crate::types::*;
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
                eprintln!(
                    "[ROUTER] Registered: {} {} (total: {})",
                    method_upper, path, self.route_count
                );
            }
            Err(e) => {
                eprintln!(
                    "[ROUTER] Failed to register {} {}: {:?}",
                    method_upper, path, e
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
                eprintln!(
                    "[ROUTER] Registered (w/ middleware): {} {} (total: {})",
                    method_upper, path, self.route_count
                );
            }
            Err(e) => {
                eprintln!(
                    "[ROUTER] Failed to register {} {}: {:?}",
                    method_upper, path, e
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
    pub fn match_route(
        &self,
        method: &str,
        path: &str,
    ) -> Option<(&RouteEntry, HashMap<String, String>)> {
        let method_upper = method.to_uppercase();

        // Debug: list all routers and their state
        eprintln!(
            "[ROUTER DEBUG] Looking for {} in routers. Available methods: {:?}",
            method_upper,
            self.routers.keys().collect::<Vec<_>>()
        );

        let router = match self.routers.get(&method_upper) {
            Some(r) => r,
            None => {
                eprintln!("[ROUTER] No router for method: {}", method_upper);
                return None;
            }
        };

        eprintln!(
            "[ROUTER DEBUG] Found router for {}, attempting match on '{}'",
            method_upper, path
        );

        match router.at(path) {
            Ok(matched) => {
                let params: HashMap<String, String> = matched
                    .params
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                eprintln!(
                    "[ROUTER] Matched: {} {} -> params: {:?}",
                    method_upper, path, params
                );
                Some((matched.value, params))
            }
            Err(e) => {
                eprintln!("[ROUTER] No match for {} {}: {:?}", method_upper, path, e);
                None
            }
        }
    }

    /// Get total number of registered routes
    pub fn count(&self) -> usize {
        self.route_count
    }
}

// Global route registry
static ROUTES: std::sync::OnceLock<Mutex<RouteRegistry>> = std::sync::OnceLock::new();

pub fn get_routes() -> &'static Mutex<RouteRegistry> {
    ROUTES.get_or_init(|| Mutex::new(RouteRegistry::new()))
}
