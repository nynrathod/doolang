//! WebSocket FFI entry points — delegates to crate::ws::* module.

use crate::make_err_http;
use doo_ffi_core::DooResult;
use std::ffi::c_void;
use std::os::raw::c_char;

use doo_ffi_core::ffi_debug;
use doo_ffi_core::{ffi_safe_cstr, ffi_safe_i64, ffi_safe_void};

use crate::helpers::{c_to_string, string_to_c};
use crate::make_ok_void;

// ============================================================================
// WEBSOCKET FFI ENTRY POINTS — All WS FFI functions centralized here
// ============================================================================
// These delegate to the crate::ws:: submodule. Keeping FFI surface in lib.rs
// follows the same pattern as HTTP routes above.

/// Register a WebSocket route on the HTTP server.
/// Doo syntax: `app.ws("/chat", (conn) => { ... })`
#[no_mangle]
pub extern "C" fn doo_ws_route(
    _server: *const c_void,
    path: *const c_char,
    handler: crate::ws::WsConnectionHandler,
) -> *mut DooResult {
    ffi_safe_result!({
        let path_str = c_to_string(path);
        ffi_debug!("WS", "Registering WebSocket route: {}", path_str);
        crate::ws::get_ws_registry().register_route(&path_str, handler);
        make_ok_void()
    })
}

/// Initialize WebSocket subsystem (called automatically).
#[no_mangle]
pub extern "C" fn doo_ws_init() {
    ffi_safe_void!({
        ffi_debug!("WS", "WebSocket subsystem initialized");
    })
}

/// Get the connection ID.
/// Doo syntax: `conn.id`
#[no_mangle]
pub extern "C" fn doo_ws_conn_id(conn: *const crate::ws::WsConnection) -> *const c_char {
    ffi_safe_cstr!({
        if conn.is_null() {
            return string_to_c("");
        }
        unsafe { string_to_c(&(*conn).id) }
    })
}

/// Emit a JSON event to a specific connection.
/// Doo syntax: `conn.emit("event", data)?`
#[no_mangle]
pub extern "C" fn doo_ws_conn_emit(
    conn: *const crate::ws::WsConnection,
    event: *const c_char,
    payload: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({
        if conn.is_null() {
            return make_err_http(400, "Null connection");
        }
        let event_str = c_to_string(event);
        let payload_str = c_to_string(payload);
        let conn_id = unsafe { &(*conn).id };

        let frame = crate::ws::build_ws_frame(&event_str, &payload_str);

        ffi_debug!("WS", "conn.emit({}) to {}", event_str, conn_id);

        match crate::ws::get_conn_registry().send_text(conn_id, &frame) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err_http(500, &format!("emit failed: {}", e)),
        }
    })
}

/// Emit binary data to a specific connection.
/// Doo syntax: `conn.emitBinary(bytes)?`
#[no_mangle]
pub extern "C" fn doo_ws_conn_emit_binary(
    conn: *const crate::ws::WsConnection,
    data: *const u8,
    len: i64,
) -> *mut DooResult {
    ffi_safe_result!({
        if conn.is_null() || data.is_null() || len <= 0 {
            return make_err_http(400, "Invalid binary emit parameters");
        }
        let conn_id = unsafe { &(*conn).id };
        let bytes = unsafe { std::slice::from_raw_parts(data, len as usize) };

        match crate::ws::get_conn_registry().send_binary(conn_id, bytes) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err_http(500, &format!("emitBinary failed: {}", e)),
        }
    })
}

/// Join a room.
/// Doo syntax: `conn.join("room1")?`
#[no_mangle]
pub extern "C" fn doo_ws_conn_join(
    conn: *const crate::ws::WsConnection,
    room: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({
        if conn.is_null() {
            return make_err_http(400, "Null connection");
        }
        let conn_id = unsafe { &(*conn).id };
        let room_str = c_to_string(room);
        ffi_debug!("WS", "conn.join({}) for {}", room_str, conn_id);
        crate::ws::get_room_registry().join(&room_str, conn_id);
        make_ok_void()
    })
}

/// Leave a room.
/// Doo syntax: `conn.leave("room1")?`
#[no_mangle]
pub extern "C" fn doo_ws_conn_leave(
    conn: *const crate::ws::WsConnection,
    room: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({
        if conn.is_null() {
            return make_err_http(400, "Null connection");
        }
        let conn_id = unsafe { &(*conn).id };
        let room_str = c_to_string(room);
        ffi_debug!("WS", "conn.leave({}) for {}", room_str, conn_id);
        crate::ws::get_room_registry().leave(&room_str, conn_id);
        make_ok_void()
    })
}

/// Close a connection.
/// Doo syntax: `conn.close()?`
#[no_mangle]
pub extern "C" fn doo_ws_conn_close(conn: *const crate::ws::WsConnection) -> *mut DooResult {
    ffi_safe_result!({
        if conn.is_null() {
            return make_err_http(400, "Null connection");
        }
        let conn_id = unsafe { &(*conn).id };
        ffi_debug!("WS", "conn.close() for {}", conn_id);
        crate::ws::get_conn_registry().close(conn_id);
        make_ok_void()
    })
}

/// Check if a connection is closed.
/// Doo syntax: `conn.isClosed()`
#[no_mangle]
pub extern "C" fn doo_ws_conn_is_closed(conn: *const crate::ws::WsConnection) -> i64 {
    ffi_safe_i64!({
        if conn.is_null() {
            return 1;
        }
        let conn_id = unsafe { &(*conn).id };
        if crate::ws::get_conn_registry().is_closed(conn_id) {
            1
        } else {
            0
        }
    })
}

/// Register an event handler on the connection.
/// Doo syntax: `conn.on("message", (msg) => { ... })`
#[no_mangle]
pub extern "C" fn doo_ws_conn_on(
    conn: *const crate::ws::WsConnection,
    event: *const c_char,
    handler: crate::ws::WsEventHandler,
) -> *mut DooResult {
    ffi_safe_result!({
        if conn.is_null() {
            return make_err_http(400, "Null connection");
        }
        let conn_id = unsafe { &(*conn).id };
        let event_str = c_to_string(event);
        ffi_debug!("WS", "conn.on({}) for {}", event_str, conn_id);
        crate::ws::get_conn_registry().register_event_handler(conn_id, &event_str, handler);
        make_ok_void()
    })
}

/// Register onConnect handler.
/// Doo syntax: `conn.onConnect(() => { ... })`
#[no_mangle]
pub extern "C" fn doo_ws_conn_on_connect(
    conn: *const crate::ws::WsConnection,
    handler: crate::ws::WsLifecycleHandler,
) -> *mut DooResult {
    ffi_safe_result!({
        if conn.is_null() {
            return make_err_http(400, "Null connection");
        }
        let conn_id = unsafe { &(*conn).id };
        ffi_debug!("WS", "conn.onConnect for {}", conn_id);
        crate::ws::get_conn_registry().set_on_connect(conn_id, handler);
        make_ok_void()
    })
}

/// Register onDisconnect handler.
/// Doo syntax: `conn.onDisconnect(() => { ... })`
#[no_mangle]
pub extern "C" fn doo_ws_conn_on_disconnect(
    conn: *const crate::ws::WsConnection,
    handler: crate::ws::WsLifecycleHandler,
) -> *mut DooResult {
    ffi_safe_result!({
        if conn.is_null() {
            return make_err_http(400, "Null connection");
        }
        let conn_id = unsafe { &(*conn).id };
        ffi_debug!("WS", "conn.onDisconnect for {}", conn_id);
        crate::ws::get_conn_registry().set_on_disconnect(conn_id, handler);
        make_ok_void()
    })
}

/// Register onError handler.
/// Doo syntax: `conn.onError((err) => { ... })`
#[no_mangle]
pub extern "C" fn doo_ws_conn_on_error(
    conn: *const crate::ws::WsConnection,
    handler: crate::ws::WsErrorHandler,
) -> *mut DooResult {
    ffi_safe_result!({
        if conn.is_null() {
            return make_err_http(400, "Null connection");
        }
        let conn_id = unsafe { &(*conn).id };
        ffi_debug!("WS", "conn.onError for {}", conn_id);
        crate::ws::get_conn_registry().set_on_error(conn_id, handler);
        make_ok_void()
    })
}

/// Broadcast an event to ALL connected clients.
/// Doo syntax: `ws.broadcast("event", data)?`
#[no_mangle]
pub extern "C" fn doo_ws_broadcast(
    _server: *const c_void,
    event: *const c_char,
    payload: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({
        let event_str = c_to_string(event);
        let payload_str = c_to_string(payload);

        let frame = crate::ws::build_ws_frame(&event_str, &payload_str);

        ffi_debug!("WS", "broadcast({})", event_str);
        let failures = crate::ws::get_conn_registry().broadcast_text(&frame);
        if failures > 0 {
            ffi_debug!("WS", "broadcast had {} failed sends", failures);
        }
        make_ok_void()
    })
}

/// Emit an event to all connections in a specific room.
/// Doo syntax: `ws.to("room1").emit("event", data)?`
#[no_mangle]
pub extern "C" fn doo_ws_room_emit(
    _server: *const c_void,
    room: *const c_char,
    event: *const c_char,
    payload: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({
        let room_str = c_to_string(room);
        let event_str = c_to_string(event);
        let payload_str = c_to_string(payload);

        let frame = crate::ws::build_ws_frame(&event_str, &payload_str);

        ffi_debug!("WS", "room_emit({}, {})", room_str, event_str);
        let conn_ids = crate::ws::get_room_registry().get_members(&room_str);
        let mut failures = 0usize;
        for conn_id in &conn_ids {
            if crate::ws::get_conn_registry().send_text(conn_id, &frame).is_err() {
                failures += 1;
            }
        }
        if failures > 0 {
            ffi_debug!("WS", "room_emit had {} failed sends", failures);
        }
        make_ok_void()
    })
}

/// Set WebSocket configuration.
/// Doo syntax: `ws.config({ "max_message_size": "65536", ... })`
#[no_mangle]
pub extern "C" fn doo_ws_config(
    _server: *const c_void,
    config_json: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({
        let json_str = c_to_string(config_json);
        ffi_debug!("WS", "Setting WS config: {}", json_str);

        match serde_json::from_str::<serde_json::Value>(&json_str) {
            Ok(val) => {
                let mut cfg = crate::ws::get_ws_config().write().unwrap();
                if let Some(v) = val.get("max_message_size").and_then(|v| v.as_u64()) {
                    cfg.max_message_size = v as usize;
                }
                if let Some(v) = val.get("heartbeat_interval").and_then(|v| v.as_u64()) {
                    cfg.heartbeat_interval_secs = v;
                }
                if let Some(v) = val.get("heartbeat_timeout").and_then(|v| v.as_u64()) {
                    cfg.heartbeat_timeout_secs = v;
                }
                if let Some(v) = val.get("send_queue_size").and_then(|v| v.as_u64()) {
                    cfg.send_queue_size = v as usize;
                }
                drop(cfg);
                make_ok_void()
            }
            Err(e) => make_err_http(400, &format!("Invalid config JSON: {}", e)),
        }
    })
}

/// Graceful shutdown — close all WebSocket connections.
#[no_mangle]
pub extern "C" fn doo_ws_shutdown(_server: *const c_void) {
    ffi_safe_void!({
        ffi_debug!("WS", "Shutting down WebSocket subsystem");
        crate::ws::get_conn_registry().shutdown_all();
    })
}

/// Get count of active WebSocket connections.
#[no_mangle]
pub extern "C" fn doo_ws_active_connections(_server: *const c_void) -> i64 {
    ffi_safe_i64!({ crate::ws::get_conn_registry().count() as i64 })
}

/// Check if a path is a registered WebSocket route (used by server.rs).
#[no_mangle]
pub extern "C" fn doo_ws_is_ws_route(_server: *const c_void, path: *const c_char) -> i64 {
    ffi_safe_i64!({
        let path_str = c_to_string(path);
        if crate::ws::is_ws_route(&path_str) {
            1
        } else {
            0
        }
    })
}