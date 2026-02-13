//! Connection Registry — Global concurrent registry of all active WebSocket connections.
//!
//! Uses `DashMap` for lock-free concurrent access.
//! Single source of truth for:
//! - Active connections and their send channels
//! - Event handlers per connection
//! - Lifecycle handlers (onConnect, onDisconnect, onError)
//! - Connection state (open/closed)

use super::connection::*;
use super::room::get_room_registry;

use dashmap::DashMap;
use doo_ffi_core::ffi_debug;
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::mpsc;

// ============================================================================
// Send Channel Message Types
// ============================================================================

/// Messages sent through the per-connection channel.
#[derive(Debug)]
pub enum WsSendMessage {
    /// Text frame (JSON event or raw text)
    Text(String),
    /// Binary frame
    Binary(Vec<u8>),
    /// Ping frame for heartbeat
    Ping,
    /// Close the connection gracefully
    Close,
}

// ============================================================================
// Per-Connection State
// ============================================================================

/// Full state for a single WebSocket connection.
/// Stored in the global `ConnRegistry`.
pub struct ConnState {
    /// Bounded sender for outgoing messages (backpressure)
    pub sender: mpsc::Sender<WsSendMessage>,
    /// Event handlers: event_name → handler function
    pub event_handlers: HashMap<String, WsEventHandler>,
    /// Lifecycle: onConnect
    pub on_connect: Option<WsLifecycleHandler>,
    /// Lifecycle: onDisconnect
    pub on_disconnect: Option<WsLifecycleHandler>,
    /// Error handler
    pub on_error: Option<WsErrorHandler>,
    /// Whether this connection has been closed
    pub closed: bool,
    /// Cancellation token — drop to cancel all spawned tasks for this connection
    pub cancel: tokio::sync::watch::Sender<bool>,
}

// ============================================================================
// Route Registry — maps path patterns to connection handlers
// ============================================================================

/// Route entry for a WebSocket path.
pub struct WsRouteEntry {
    pub handler: WsConnectionHandler,
}

/// WebSocket route registry — maps URL paths to handlers.
pub struct WsRouteRegistry {
    routes: DashMap<String, WsRouteEntry>,
}

impl WsRouteRegistry {
    pub fn new() -> Self {
        Self {
            routes: DashMap::new(),
        }
    }

    /// Register a WS route.
    pub fn register_route(&self, path: &str, handler: WsConnectionHandler) {
        self.routes
            .insert(path.to_string(), WsRouteEntry { handler });
        ffi_debug!(
            "WS",
            "Route registered: {} (total: {})",
            path,
            self.routes.len()
        );
    }

    /// Look up a WS route handler by path.
    pub fn match_route(&self, path: &str) -> Option<WsConnectionHandler> {
        self.routes.get(path).map(|entry| entry.handler)
    }

    /// Check if any WS routes are registered.
    pub fn has_routes(&self) -> bool {
        !self.routes.is_empty()
    }

    /// Get the number of registered WS routes.
    pub fn count(&self) -> usize {
        self.routes.len()
    }
}

/// Global WS route registry.
static WS_ROUTE_REGISTRY: OnceLock<WsRouteRegistry> = OnceLock::new();

pub fn get_ws_registry() -> &'static WsRouteRegistry {
    WS_ROUTE_REGISTRY.get_or_init(WsRouteRegistry::new)
}

// ============================================================================
// Connection Registry — maps conn_id to ConnState
// ============================================================================

/// Global connection registry — thread-safe, lock-free.
pub struct ConnRegistry {
    connections: DashMap<String, ConnState>,
}

impl ConnRegistry {
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
        }
    }

    /// Register a new connection with its send channel.
    pub fn insert(
        &self,
        conn_id: &str,
        sender: mpsc::Sender<WsSendMessage>,
        cancel: tokio::sync::watch::Sender<bool>,
    ) {
        let state = ConnState {
            sender,
            event_handlers: HashMap::new(),
            on_connect: None,
            on_disconnect: None,
            on_error: None,
            closed: false,
            cancel,
        };
        self.connections.insert(conn_id.to_string(), state);
        ffi_debug!(
            "WS",
            "Connection registered: {} (active: {})",
            conn_id,
            self.connections.len()
        );
    }

    /// Remove a connection and clean up.
    pub fn remove(&self, conn_id: &str) -> Option<ConnState> {
        let removed = self.connections.remove(conn_id).map(|(_, state)| state);
        if removed.is_some() {
            // Auto-remove from all rooms
            get_room_registry().remove_from_all(conn_id);
            ffi_debug!(
                "WS",
                "Connection removed: {} (active: {})",
                conn_id,
                self.connections.len()
            );
        }
        removed
    }

    /// Send a text message to a specific connection.
    pub fn send_text(&self, conn_id: &str, text: &str) -> Result<(), String> {
        if let Some(state) = self.connections.get(conn_id) {
            if state.closed {
                return Err("Connection is closed".to_string());
            }
            state
                .sender
                .try_send(WsSendMessage::Text(text.to_string()))
                .map_err(|e| format!("Send failed (backpressure): {}", e))
        } else {
            Err(format!("Connection not found: {}", conn_id))
        }
    }

    /// Send binary data to a specific connection.
    pub fn send_binary(&self, conn_id: &str, data: &[u8]) -> Result<(), String> {
        if let Some(state) = self.connections.get(conn_id) {
            if state.closed {
                return Err("Connection is closed".to_string());
            }
            state
                .sender
                .try_send(WsSendMessage::Binary(data.to_vec()))
                .map_err(|e| format!("Send failed (backpressure): {}", e))
        } else {
            Err(format!("Connection not found: {}", conn_id))
        }
    }

    /// Close a connection by sending a Close message and cancelling tasks.
    pub fn close(&self, conn_id: &str) {
        if let Some(mut state) = self.connections.get_mut(conn_id) {
            state.closed = true;
            let _ = state.sender.try_send(WsSendMessage::Close);
            // Signal cancellation to all tasks for this connection
            let _ = state.cancel.send(true);
        }
    }

    /// Check if a connection is closed.
    pub fn is_closed(&self, conn_id: &str) -> bool {
        self.connections
            .get(conn_id)
            .map(|s| s.closed)
            .unwrap_or(true) // Not found = closed
    }

    /// Register an event handler for a connection.
    pub fn register_event_handler(&self, conn_id: &str, event: &str, handler: WsEventHandler) {
        if let Some(mut state) = self.connections.get_mut(conn_id) {
            state.event_handlers.insert(event.to_string(), handler);
        }
    }

    /// Set onConnect handler.
    pub fn set_on_connect(&self, conn_id: &str, handler: WsLifecycleHandler) {
        if let Some(mut state) = self.connections.get_mut(conn_id) {
            state.on_connect = Some(handler);
        }
    }

    /// Set onDisconnect handler.
    pub fn set_on_disconnect(&self, conn_id: &str, handler: WsLifecycleHandler) {
        if let Some(mut state) = self.connections.get_mut(conn_id) {
            state.on_disconnect = Some(handler);
        }
    }

    /// Set onError handler.
    pub fn set_on_error(&self, conn_id: &str, handler: WsErrorHandler) {
        if let Some(mut state) = self.connections.get_mut(conn_id) {
            state.on_error = Some(handler);
        }
    }

    /// Get event handler for a connection + event name.
    pub fn get_event_handler(&self, conn_id: &str, event: &str) -> Option<WsEventHandler> {
        self.connections
            .get(conn_id)
            .and_then(|s| s.event_handlers.get(event).copied())
    }

    /// Get onConnect handler.
    pub fn get_on_connect(&self, conn_id: &str) -> Option<WsLifecycleHandler> {
        self.connections.get(conn_id).and_then(|s| s.on_connect)
    }

    /// Get onDisconnect handler.
    pub fn get_on_disconnect(&self, conn_id: &str) -> Option<WsLifecycleHandler> {
        self.connections.get(conn_id).and_then(|s| s.on_disconnect)
    }

    /// Get onError handler.
    pub fn get_on_error(&self, conn_id: &str) -> Option<WsErrorHandler> {
        self.connections.get(conn_id).and_then(|s| s.on_error)
    }

    /// Broadcast text to ALL connected clients. Returns count of failed sends.
    pub fn broadcast_text(&self, text: &str) -> usize {
        let mut failures = 0;
        for entry in self.connections.iter() {
            if entry.closed {
                continue;
            }
            if entry
                .sender
                .try_send(WsSendMessage::Text(text.to_string()))
                .is_err()
            {
                failures += 1;
            }
        }
        failures
    }

    /// Get the cancel watch receiver for a connection.
    pub fn subscribe_cancel(&self, conn_id: &str) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.connections.get(conn_id).map(|s| s.cancel.subscribe())
    }

    /// Get count of active connections.
    pub fn count(&self) -> usize {
        self.connections.len()
    }

    /// Graceful shutdown — close all connections.
    pub fn shutdown_all(&self) {
        let conn_ids: Vec<String> = self.connections.iter().map(|e| e.key().clone()).collect();
        for conn_id in &conn_ids {
            self.close(conn_id);
        }
        ffi_debug!(
            "WS",
            "All {} connections closed for shutdown",
            conn_ids.len()
        );
    }
}

/// Global connection registry.
static CONN_REGISTRY: OnceLock<ConnRegistry> = OnceLock::new();

pub fn get_conn_registry() -> &'static ConnRegistry {
    CONN_REGISTRY.get_or_init(ConnRegistry::new)
}
