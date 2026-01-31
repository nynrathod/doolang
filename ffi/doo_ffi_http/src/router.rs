//! Route Registry
//! High-performance routing with matchit, middleware chains, and handler metadata.

use crate::types::*;
use matchit::Router as MatchitRouter;
use std::collections::HashMap;
use std::sync::Mutex;

/// Route entry with handler and middleware
pub struct RouteEntry {
    pub handler: DooHandlerFn,
    pub middleware: Vec<DooMiddlewareFn>,
}

/// Route registry with method-based routing
pub struct RouteRegistry {
    routers: HashMap<String, MatchitRouter<RouteEntry>>,
    pub global_middleware: Vec<DooMiddlewareFn>,
    pub middleware_handlers: HashMap<String, DooMiddlewareFn>,
    pub named_handlers: HashMap<String, DooHandlerFn>,
    pub handler_metadata: HashMap<String, HandlerMetadata>,
    pub route_count: usize,
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
        }
    }

    /// Register a route with handler function
    pub fn register(&mut self, method: &str, path: &str, handler: DooHandlerFn) {
        let router = self.routers.entry(method.to_uppercase())
            .or_insert_with(MatchitRouter::new);
        
        let entry = RouteEntry {
            handler,
            middleware: self.global_middleware.clone(),
        };
        
        if router.insert(path, entry).is_ok() {
            self.route_count += 1;
        }
    }

    /// Register a route by handler name (looked up later)
    pub fn register_by_name(&mut self, method: &str, path: &str, handler_name: &str) {
        if let Some(&handler) = self.named_handlers.get(handler_name) {
            self.register(method, path, handler);
        }
    }

    /// Register a route with specific middleware
    pub fn register_with_middleware(
        &mut self, 
        method: &str, 
        path: &str, 
        handler: DooHandlerFn,
        middleware: Vec<DooMiddlewareFn>
    ) {
        let router = self.routers.entry(method.to_uppercase())
            .or_insert_with(MatchitRouter::new);
        
        let mut mw = self.global_middleware.clone();
        mw.extend(middleware);
        
        let entry = RouteEntry {
            handler,
            middleware: mw,
        };
        
        if router.insert(path, entry).is_ok() {
            self.route_count += 1;
        }
    }

    /// Add global middleware
    pub fn add_middleware(&mut self, mw: DooMiddlewareFn) {
        if !self.global_middleware.iter().any(|m| *m as usize == mw as usize) {
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
        metadata: HandlerMetadata
    ) {
        self.named_handlers.insert(name.to_string(), handler);
        self.handler_metadata.insert(name.to_string(), metadata);
    }

    /// Match a route and extract parameters
    pub fn match_route(&self, method: &str, path: &str) -> Option<(&RouteEntry, HashMap<String, String>)> {
        let router = self.routers.get(&method.to_uppercase())?;
        let matched = router.at(path).ok()?;
        
        let params: HashMap<String, String> = matched.params.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        
        Some((matched.value, params))
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
