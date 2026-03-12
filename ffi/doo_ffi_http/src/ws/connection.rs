//! WebSocket Connection — Represents a single connected client.
//!
//! Each connection has:
//! - A unique UUID-based ID
//! - A bounded sender channel for outgoing messages (backpressure)
//! - Event handlers registered by user code
//! - Lifecycle handlers (onConnect, onDisconnect, onError)
//! - Room membership (managed by RoomRegistry)

use std::os::raw::c_char;

/// FFI-compatible WebSocket connection handle.
/// Passed to user Doo code as an opaque pointer.
#[repr(C)]
pub struct WsConnection {
    /// Unique connection ID (UUID v4 string)
    pub id: String,
    /// Whether this connection has been closed
    pub closed: bool,
}

// ============================================================================
// FFI Handler Types — Single Source of Truth
// ============================================================================

/// User handler called when a new WebSocket connection is established.
/// Signature: `extern "C" fn(*const WsConnection)`
/// Doo syntax: `app.ws("/chat", (conn) => { ... })`
pub type WsConnectionHandler = extern "C" fn(*const WsConnection);

/// Event handler called when a message matching an event name arrives.
/// Receives: (connection_ptr, data_payload_as_json_string)
/// Doo syntax: `conn.on("message", onMsg)` where `fn onMsg(conn: WsConnection, msg: Str)`
pub type WsEventHandler = extern "C" fn(*const WsConnection, *const c_char);

/// Lifecycle handler (onConnect, onDisconnect) — receives connection.
/// Doo syntax: `conn.onConnect(handler)` where `fn handler(conn: WsConnection)`
pub type WsLifecycleHandler = extern "C" fn(*const WsConnection);

/// Error handler — receives connection and error message.
/// Doo syntax: `conn.onError(handler)` where `fn handler(conn: WsConnection, err: Str)`
pub type WsErrorHandler = extern "C" fn(*const WsConnection, *const c_char);
