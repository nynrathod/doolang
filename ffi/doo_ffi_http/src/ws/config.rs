//! WebSocket Configuration — Single Source of Truth for all WS settings.
//!
//! All configurable parameters are centralized here.
//! Accessed via `get_ws_config()` which returns a thread-safe read-write lock.

use std::sync::{OnceLock, RwLock};

/// WebSocket server configuration — all tunables in one place.
#[derive(Debug, Clone)]
pub struct WsConfig {
    /// Maximum message size in bytes (default: 64KB).
    /// Messages exceeding this are rejected and the connection is closed.
    pub max_message_size: usize,

    /// Ping interval in seconds (default: 30).
    /// Server sends a ping frame every N seconds to detect dead connections.
    pub heartbeat_interval_secs: u64,

    /// Pong timeout in seconds (default: 10).
    /// If no pong is received within this time after a ping, the connection is dropped.
    pub heartbeat_timeout_secs: u64,

    /// Maximum number of pending outgoing messages per connection (default: 256).
    /// When the queue is full, new sends are dropped and the connection may be closed.
    pub send_queue_size: usize,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            max_message_size: 64 * 1024,       // 64 KB
            heartbeat_interval_secs: 30,
            heartbeat_timeout_secs: 10,
            send_queue_size: 256,
        }
    }
}

/// Global WS config — single source of truth.
static WS_CONFIG: OnceLock<RwLock<WsConfig>> = OnceLock::new();

/// Get the global WebSocket configuration (read-heavy, write-rare).
pub fn get_ws_config() -> &'static RwLock<WsConfig> {
    WS_CONFIG.get_or_init(|| RwLock::new(WsConfig::default()))
}
