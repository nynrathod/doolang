//! WebSocket Message Handler — Processes incoming WebSocket frames.
//!
//! Dispatches incoming messages to user-registered event handlers.
//! All handlers receive a WsConnection pointer as first argument so they
//! can emit/join/leave without closure capture (C FFI limitation).

use super::connection::WsConnection;
use super::registry::*;
use crate::helpers::string_to_c;

use doo_ffi_core::ffi_debug;

/// Build a JSON event frame: `{"event":"<event>","data":<payload>}`
///
/// Single source of truth for frame building. Used by:
/// - `doo_ws_conn_emit`
/// - `doo_ws_room_emit`
/// - `doo_ws_broadcast`
///
/// The payload is a raw string from Doo code (`Str` type).
/// - If it parses as valid JSON, it's inserted as-is (object/array/number).
/// - Otherwise, it's JSON-encoded as a string value.
pub fn build_ws_frame(event: &str, payload: &str) -> String {
    if payload.is_empty() {
        return format!(r#"{{"event":"{}","data":null}}"#, event);
    }
    // If the payload is already valid JSON (object, array, number, bool, null,
    // or a quoted string), use it directly in the frame.
    if serde_json::from_str::<serde_json::Value>(payload).is_ok() {
        format!(r#"{{"event":"{}","data":{}}}"#, event, payload)
    } else {
        // Plain string that's not valid JSON — encode as JSON string
        let encoded = serde_json::to_string(payload).unwrap_or_else(|_| format!("\"{}\"", payload));
        format!(r#"{{"event":"{}","data":{}}}"#, event, encoded)
    }
}

/// Create a stack-local WsConnection for the given conn_id.
/// SAFETY: The returned pointer is valid only for the duration of the handler call.
/// The handler must NOT store it — all persistent state uses conn_id via registries.
fn make_conn_handle(conn_id: &str) -> WsConnection {
    WsConnection {
        id: conn_id.to_string(),
        closed: get_conn_registry().is_closed(conn_id),
    }
}

/// Process an incoming text message.
/// Parses JSON event framing and dispatches to the appropriate handler.
///
/// Expected format: `{ "event": "name", "data": <any> }`
/// If no event field, dispatches to "message" handler with the raw text.
pub fn handle_text_message(conn_id: &str, text: &str) {
    let registry = get_conn_registry();

    // Try to parse as JSON event frame
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(event) = val.get("event").and_then(|e| e.as_str()) {
            // Extract data payload — raw string for String values,
            // JSON-encoded for objects/arrays/numbers/booleans.
            // This ensures conn.join(data) / conn.leave(data) get
            // the actual room name, not a JSON-quoted version.
            let data = match val.get("data") {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(d) => d.to_string(),
                None => "null".to_string(),
            };

            ffi_debug!(
                "WS",
                "Event '{}' from {} with data len={}",
                event,
                conn_id,
                data.len()
            );

            // Dispatch to event handler — pass conn + data
            if let Some(handler) = registry.get_event_handler(conn_id, event) {
                let conn = make_conn_handle(conn_id);
                let data_c = string_to_c(&data);
                handler(&conn as *const WsConnection, data_c);
                return;
            }
            // Fall through to "message" handler if specific event not registered
        }
    }

    // No event framing or no specific handler — dispatch to generic "message" handler
    if let Some(handler) = registry.get_event_handler(conn_id, "message") {
        let conn = make_conn_handle(conn_id);
        let text_c = string_to_c(text);
        handler(&conn as *const WsConnection, text_c);
    } else {
        ffi_debug!("WS", "No handler for message from {}", conn_id);
    }
}

/// Process an incoming binary frame.
/// Dispatches to "binary" event handler if registered.
pub fn handle_binary_message(conn_id: &str, data: &[u8]) {
    let registry = get_conn_registry();

    // Pass binary info as JSON metadata
    let data_json = serde_json::json!({
        "length": data.len(),
        "type": "binary"
    })
    .to_string();

    if let Some(handler) = registry.get_event_handler(conn_id, "binary") {
        let conn = make_conn_handle(conn_id);
        let data_c = string_to_c(&data_json);
        handler(&conn as *const WsConnection, data_c);
    } else {
        ffi_debug!(
            "WS",
            "No binary handler for {} (data len={})",
            conn_id,
            data.len()
        );
    }
}

/// Called when a connection is fully established.
/// Fires the onConnect lifecycle handler with conn pointer.
pub fn fire_on_connect(conn_id: &str) {
    if let Some(handler) = get_conn_registry().get_on_connect(conn_id) {
        ffi_debug!("WS", "Firing onConnect for {}", conn_id);
        let conn = make_conn_handle(conn_id);
        handler(&conn as *const WsConnection);
    }
}

/// Called when a connection is disconnected.
/// Fires the onDisconnect lifecycle handler with conn pointer.
pub fn fire_on_disconnect(conn_id: &str) {
    if let Some(handler) = get_conn_registry().get_on_disconnect(conn_id) {
        ffi_debug!("WS", "Firing onDisconnect for {}", conn_id);
        let conn = make_conn_handle(conn_id);
        handler(&conn as *const WsConnection);
    }
}

/// Called when an error occurs on a connection.
pub fn fire_on_error(conn_id: &str, error_msg: &str) {
    if let Some(handler) = get_conn_registry().get_on_error(conn_id) {
        let conn = make_conn_handle(conn_id);
        let err_c = string_to_c(error_msg);
        handler(&conn as *const WsConnection, err_c);
    }
}
