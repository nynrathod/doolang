//! WebSocket package codegen hooks.
//!
//! All WebSocket-specific codegen behavior is isolated here:
//! - Route handler wrappers (fn(WsConnection) → void)
//! - Event/error handler wrappers (fn(WsConnection, data) → void)
//! - Lifecycle handler wrappers (fn(WsConnection) → void for connect/disconnect)
//!
//! The WS handler wrapper signatures differ from HTTP:
//! - HTTP: fn(DooRequest*) → DooResult* (complex marshalling)
//! - WS:   fn(WsConnection*) → void (simple pointer passthrough)

use crate::context::CodegenContext;
use crate::instructions::calls::call_wrappers;
use inkwell::values::FunctionValue;

// ============================================================================
// WebSocket FFI Symbol Constants (Package-Owned)
// ============================================================================

pub(crate) const DOO_WS_ROUTE: &str = "doo_ws_route";
pub(crate) const DOO_WS_CONN_ON: &str = "doo_ws_conn_on";
pub(crate) const DOO_WS_CONN_ON_ERROR: &str = "doo_ws_conn_on_error";
pub(crate) const DOO_WS_CONN_ON_CONNECT: &str = "doo_ws_conn_on_connect";
pub(crate) const DOO_WS_CONN_ON_DISCONNECT: &str = "doo_ws_conn_on_disconnect";

/// Handle FuncRef wrapping for WebSocket package symbols.
///
/// Dispatches to the appropriate WS wrapper generator based on symbol:
/// - `doo_ws_route` → route handler wrapper
/// - `doo_ws_conn_on` / `doo_ws_conn_on_error` → event handler wrapper
/// - `doo_ws_conn_on_connect` / `doo_ws_conn_on_disconnect` → lifecycle wrapper
/// - Other WS symbols → route handler wrapper (default)
pub(crate) fn wrap_func_ref<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    symbol: &str,
    func_name: &str,
) -> FunctionValue<'ctx> {
    if symbol == DOO_WS_ROUTE {
        // Route handler: fn(*const WsConnection) → void
        call_wrappers::get_or_generate_ws_handler_wrapper(ctx, func_name)
    } else if symbol == DOO_WS_CONN_ON || symbol == DOO_WS_CONN_ON_ERROR {
        // Event/error handler: fn(*const WsConnection, *const c_char) → void
        call_wrappers::get_or_generate_ws_event_handler_wrapper(ctx, func_name)
    } else if symbol == DOO_WS_CONN_ON_CONNECT || symbol == DOO_WS_CONN_ON_DISCONNECT {
        // Lifecycle handler: fn(*const WsConnection) → void
        call_wrappers::get_or_generate_ws_lifecycle_handler_wrapper(ctx, func_name)
    } else {
        // Default WS wrapper: route handler style
        call_wrappers::get_or_generate_ws_handler_wrapper(ctx, func_name)
    }
}
