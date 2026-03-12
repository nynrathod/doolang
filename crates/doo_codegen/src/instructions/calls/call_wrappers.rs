//! Handler wrapper generation — HTTP and WebSocket wrapper codegen.

use super::RouteContext;
use crate::builtins::JsonBuiltins;
use crate::context::CodegenContext;
use crate::packages::http as http_pkg;
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_core::types::TypeKind;
use doo_mir::{MirConst, MirOperand};
use inkwell::values::{FunctionValue, PointerValue};
/// Extract route context from FFI call arguments.
/// For HTTP route registrations like doo_http_get_fn(server, path, handler),
/// this extracts the route path pattern and middleware information.
pub(crate) fn extract_route_context(symbol: &str, args: &[MirOperand]) -> RouteContext {
    let mut ctx = RouteContext::default();

    // Only process HTTP route registrations
    if !symbol.starts_with("doo_http_") {
        return ctx;
    }

    // Extract HTTP method from symbol name
    ctx.http_method = if symbol.contains("_get") {
        Some("GET".to_string())
    } else if symbol.contains("_post") {
        Some("POST".to_string())
    } else if symbol.contains("_put") {
        Some("PUT".to_string())
    } else if symbol.contains("_delete") {
        Some("DELETE".to_string())
    } else if symbol.contains("_patch") {
        Some("PATCH".to_string())
    } else {
        None
    };

    // For route registrations: args[1] is the path pattern
    // doo_http_get_fn(server, path, handler)
    // doo_http_get_with_middleware(server, path, middleware, handler)
    if args.len() >= 2 {
        if let MirOperand::Const(MirConst::Str(path)) = &args[1] {
            ctx.route_path = Some(path.clone());
        }
    }

    // For middleware variants: args[2] is the middleware names
    if symbol.ends_with("_with_middleware") && args.len() >= 3 {
        if let MirOperand::Const(MirConst::Str(middleware_str)) = &args[2] {
            ctx.middleware_names = middleware_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    ctx
}

// ============================================================================
// WEBSOCKET HANDLER WRAPPERS
// ============================================================================
// WS handlers have simpler signatures compared to HTTP:
// - Route handler:     fn(*const WsConnection) → void
// - Event handler:     fn(*const c_char) → void      (message/error data)
// - Lifecycle handler: fn() → void                    (connect/disconnect)

/// Generate a wrapper for WebSocket route handlers.
/// FFI expects: extern "C" fn(*const WsConnection)
/// User has:    fn(conn: WsConnection) { ... } or fn(conn: WsConnection, app: Server) { ... }
pub(crate) fn get_or_generate_ws_handler_wrapper<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    user_func_name: &str,
) -> FunctionValue<'ctx> {
    let wrapper_name = format!("__ws_wrapper_{}", user_func_name);
    if let Some(existing) = ctx.get_function(&wrapper_name) {
        return existing;
    }

    let ptr_type = ctx.ptr_type();
    let wrapper_fn_type = ctx.context.void_type().fn_type(&[ptr_type.into()], false);
    let wrapper_fn = ctx
        .module
        .add_function(&wrapper_name, wrapper_fn_type, None);

    let current_block = ctx.builder.get_insert_block();
    let entry = ctx.context.append_basic_block(wrapper_fn, "entry");
    ctx.builder.position_at_end(entry);

    let ws_conn_ptr = wrapper_fn
        .get_nth_param(0)
        .expect("ICE: wrapper function missing expected parameter")
        .into_pointer_value();

    match ctx.get_function(user_func_name) {
        Some(user_func) => {
            // Type-aware dispatch
            let param_type_ids = ctx.get_function_param_types(user_func_name);
            let all_types: Vec<doo_core::types::TypeId> =
                param_type_ids.map(|t| t.to_vec()).unwrap_or_default();

            let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();

            for param_tid in &all_types {
                let type_name = ctx
                    .get_type_kind(*param_tid)
                    .map(|tk| match tk {
                        TypeKind::Struct { ref name, .. } => name.clone(),
                        _ => "Unknown".to_string(),
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                match type_name.as_str() {
                    "WsConnection" => call_args.push(ws_conn_ptr.into()),
                    "Server" | "DooServer" => {
                        let get_server_fn = ctx
                            .module
                            .get_function(http_pkg::DOO_HTTP_GET_SERVER_INSTANCE)
                            .unwrap_or_else(|| {
                                let fn_type = ptr_type.fn_type(&[], false);
                                ctx.module.add_function(
                                    http_pkg::DOO_HTTP_GET_SERVER_INSTANCE,
                                    fn_type,
                                    Some(inkwell::module::Linkage::External),
                                )
                            });
                        let server_ptr = ctx
                            .builder
                            .build_call(get_server_fn, &[], "server_instance")
                            .ok()
                            .and_then(|cs| cs.try_as_basic_value().basic())
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ptr_type.const_null());
                        call_args.push(server_ptr.into());
                    }
                    _ => call_args.push(ws_conn_ptr.into()),
                }
            }

            if !call_args.is_empty() {
                let _ = ctx.builder.build_call(user_func, &call_args, "");
            } else {
                let _ = ctx.builder.build_call(user_func, &[ws_conn_ptr.into()], "");
            }
        }
        None => {
            doo_debug!(
                "CODEGEN",
                "Warning: WS handler {} not found",
                user_func_name
            );
        }
    }

    let _ = ctx.builder.build_return(None);

    if let Some(block) = current_block {
        ctx.builder.position_at_end(block);
    }
    wrapper_fn
}

/// Generate a wrapper for WebSocket event/error handlers.
/// FFI expects: extern "C" fn(*const WsConnection, *const c_char)
/// User has:    fn(conn: WsConnection, data: Str) { ... }
pub(crate) fn get_or_generate_ws_event_handler_wrapper<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    user_func_name: &str,
) -> FunctionValue<'ctx> {
    let wrapper_name = format!("__ws_event_wrapper_{}", user_func_name);
    if let Some(existing) = ctx.get_function(&wrapper_name) {
        return existing;
    }

    let ptr_type = ctx.ptr_type();
    // FFI sends: (conn_ptr, data_ptr)
    let wrapper_fn_type = ctx
        .context
        .void_type()
        .fn_type(&[ptr_type.into(), ptr_type.into()], false);
    let wrapper_fn = ctx
        .module
        .add_function(&wrapper_name, wrapper_fn_type, None);

    let current_block = ctx.builder.get_insert_block();
    let entry = ctx.context.append_basic_block(wrapper_fn, "entry");
    ctx.builder.position_at_end(entry);

    let conn_ptr = wrapper_fn
        .get_nth_param(0)
        .expect("ICE: wrapper function missing expected parameter")
        .into_pointer_value();
    let data_ptr = wrapper_fn
        .get_nth_param(1)
        .expect("ICE: wrapper function missing expected parameter")
        .into_pointer_value();

    match ctx.get_function(user_func_name) {
        Some(user_func) => {
            // Type-aware dispatch: "order doesnt matter, only type matters"
            let param_type_ids = ctx.get_function_param_types(user_func_name);
            let all_types: Vec<doo_core::types::TypeId> =
                param_type_ids.map(|t| t.to_vec()).unwrap_or_default();

            let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();

            for param_tid in &all_types {
                let type_name = ctx
                    .get_type_kind(*param_tid)
                    .map(|tk| match tk {
                        TypeKind::Struct { ref name, .. } => name.clone(),
                        TypeKind::Str => "Str".to_string(),
                        _ => "Unknown".to_string(),
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                match type_name.as_str() {
                    "WsConnection" => call_args.push(conn_ptr.into()),
                    "Str" => call_args.push(data_ptr.into()),
                    "Server" | "DooServer" => {
                        // Inject global server instance
                        let get_server_fn = ctx
                            .module
                            .get_function(http_pkg::DOO_HTTP_GET_SERVER_INSTANCE)
                            .unwrap_or_else(|| {
                                let fn_type = ptr_type.fn_type(&[], false);
                                ctx.module.add_function(
                                    http_pkg::DOO_HTTP_GET_SERVER_INSTANCE,
                                    fn_type,
                                    Some(inkwell::module::Linkage::External),
                                )
                            });
                        let server_ptr = ctx
                            .builder
                            .build_call(get_server_fn, &[], "server_instance")
                            .ok()
                            .and_then(|cs| cs.try_as_basic_value().basic())
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ptr_type.const_null());
                        call_args.push(server_ptr.into());
                    }
                    _ => {
                        // Unknown type — pass data_ptr as fallback
                        call_args.push(data_ptr.into());
                    }
                }
            }

            if !call_args.is_empty() {
                let _ = ctx.builder.build_call(user_func, &call_args, "");
            } else {
                let _ = ctx.builder.build_call(user_func, &[], "");
            }
        }
        None => {
            doo_debug!(
                "CODEGEN",
                "Warning: WS event handler {} not found",
                user_func_name
            );
        }
    }

    let _ = ctx.builder.build_return(None);

    if let Some(block) = current_block {
        ctx.builder.position_at_end(block);
    }
    wrapper_fn
}

/// Generate a wrapper for WebSocket lifecycle handlers (onConnect/onDisconnect).
/// FFI expects: extern "C" fn(*const WsConnection)
/// User has:    fn(conn: WsConnection) { ... } or fn(conn: WsConnection, app: Server) { ... }
pub(crate) fn get_or_generate_ws_lifecycle_handler_wrapper<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    user_func_name: &str,
) -> FunctionValue<'ctx> {
    let wrapper_name = format!("__ws_lifecycle_wrapper_{}", user_func_name);
    if let Some(existing) = ctx.get_function(&wrapper_name) {
        return existing;
    }

    let ptr_type = ctx.ptr_type();
    // FFI sends: (conn_ptr)
    let wrapper_fn_type = ctx.context.void_type().fn_type(&[ptr_type.into()], false);
    let wrapper_fn = ctx
        .module
        .add_function(&wrapper_name, wrapper_fn_type, None);

    let current_block = ctx.builder.get_insert_block();
    let entry = ctx.context.append_basic_block(wrapper_fn, "entry");
    ctx.builder.position_at_end(entry);

    let conn_ptr = wrapper_fn
        .get_nth_param(0)
        .expect("ICE: wrapper function missing expected parameter")
        .into_pointer_value();

    match ctx.get_function(user_func_name) {
        Some(user_func) => {
            // Type-aware dispatch
            let param_type_ids = ctx.get_function_param_types(user_func_name);
            let all_types: Vec<doo_core::types::TypeId> =
                param_type_ids.map(|t| t.to_vec()).unwrap_or_default();

            let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();

            for param_tid in &all_types {
                let type_name = ctx
                    .get_type_kind(*param_tid)
                    .map(|tk| match tk {
                        TypeKind::Struct { ref name, .. } => name.clone(),
                        _ => "Unknown".to_string(),
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                match type_name.as_str() {
                    "WsConnection" => call_args.push(conn_ptr.into()),
                    "Server" | "DooServer" => {
                        let get_server_fn = ctx
                            .module
                            .get_function(http_pkg::DOO_HTTP_GET_SERVER_INSTANCE)
                            .unwrap_or_else(|| {
                                let fn_type = ptr_type.fn_type(&[], false);
                                ctx.module.add_function(
                                    http_pkg::DOO_HTTP_GET_SERVER_INSTANCE,
                                    fn_type,
                                    Some(inkwell::module::Linkage::External),
                                )
                            });
                        let server_ptr = ctx
                            .builder
                            .build_call(get_server_fn, &[], "server_instance")
                            .ok()
                            .and_then(|cs| cs.try_as_basic_value().basic())
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ptr_type.const_null());
                        call_args.push(server_ptr.into());
                    }
                    _ => call_args.push(conn_ptr.into()),
                }
            }

            if !call_args.is_empty() {
                let _ = ctx.builder.build_call(user_func, &call_args, "");
            } else {
                let _ = ctx.builder.build_call(user_func, &[], "");
            }
        }
        None => {
            doo_debug!(
                "CODEGEN",
                "Warning: WS lifecycle handler {} not found",
                user_func_name
            );
        }
    }

    let _ = ctx.builder.build_return(None);

    if let Some(block) = current_block {
        ctx.builder.position_at_end(block);
    }
    wrapper_fn
}

/// Generate or retrieve a wrapper function that adapts a user handler to FFI signature.
///
/// This is the COMPILER MAGIC that allows any handler signature to work with FFI.
///
/// FFI expects: extern "C" fn(*const DooRequest) -> *mut DooResult
/// User might have: fn() -> Str, fn(Request) -> Response, etc.
///
/// The wrapper:
/// 1. Has the FFI-expected signature
/// 2. Calls the user's function with appropriate arguments
/// 3. Wraps the return value in DooResult format
pub(crate) fn get_or_generate_handler_wrapper<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    user_func_name: &str,
    ffi_symbol: &str,
) -> FunctionValue<'ctx> {
    // Delegate to context-aware version with empty context
    get_or_generate_handler_wrapper_with_context(
        ctx,
        user_func_name,
        ffi_symbol,
        &RouteContext::default(),
    )
}

/// Generate or retrieve a wrapper function with route context.
/// This version knows about the route pattern and middleware, allowing correct
/// parameter extraction from path params, JWT claims, etc.
pub(crate) fn get_or_generate_handler_wrapper_with_context<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    user_func_name: &str,
    ffi_symbol: &str,
    route_context: &RouteContext,
) -> FunctionValue<'ctx> {
    let wrapper_name = format!("__ffi_wrapper_{}", user_func_name);

    // Check if wrapper already exists
    if let Some(existing) = ctx.get_function(&wrapper_name) {
        return existing;
    }

    let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();
    if debug {
        doo_debug!(
            "CODEGEN",
            "Generating FFI wrapper for {} (used by {})",
            user_func_name,
            ffi_symbol
        );
    }

    // Check if this is an FFI function that needs to be called via its external symbol
    let ffi_symbol_info = ctx
        .get_ffi_symbol(user_func_name)
        .map(|(_, sym)| sym.to_string());

    // Get the user's function (or the FFI external function)
    let user_func = if let Some(ref ext_symbol) = ffi_symbol_info {
        // FFI function: get or declare the external symbol
        ctx.module.get_function(ext_symbol).unwrap_or_else(|| {
            // Declare the external function with the expected signature
            let ptr_type = ctx.ptr_type();
            // For simple FFI functions like jwt() -> Str, we use a simple signature
            let fn_type = ptr_type.fn_type(&[], false);
            ctx.module.add_function(
                ext_symbol,
                fn_type,
                Some(inkwell::module::Linkage::External),
            )
        })
    } else {
        // User-defined function: get it directly
        match ctx.get_function(user_func_name) {
            Some(f) => f,
            None => {
                // Function not found - create a dummy wrapper that returns null
                doo_debug!(
                    "CODEGEN",
                    "Warning: Function {} not found for wrapper generation",
                    user_func_name
                );
                return create_dummy_wrapper(ctx, &wrapper_name);
            }
        }
    };

    // Check if return type is a struct (not a primitive)
    let return_type_id = ctx.get_function_return_type(user_func_name);
    let return_type_name = return_type_id.and_then(|tid| {
        ctx.get_type_kind(tid).map(|tk| match tk {
            TypeKind::Str => "Str".to_string(),
            TypeKind::Int => "Int".to_string(),
            TypeKind::Float => "Float".to_string(),
            TypeKind::Bool => "Bool".to_string(),
            TypeKind::Void => "Void".to_string(),
            TypeKind::Struct { name, .. } => name.clone(),
            TypeKind::Enum { name, .. } => name.clone(),
            TypeKind::Array { .. } => "Array".to_string(),
            _ => "Unknown".to_string(),
        })
    });

    let returns_struct = return_type_name.as_ref().map_or(false, |name: &String| {
        !matches!(
            name.as_str(),
            "Str" | "Int" | "Float" | "Bool" | "Void" | "Array" | "Unknown"
        )
    });

    // Analyze user function signature
    let user_fn_type = user_func.get_type();
    let user_param_count = user_fn_type.count_param_types();
    let user_return_type = user_fn_type.get_return_type();

    // Check if this is a middleware function (2 params with second being "Next" type)
    // For FFI functions, we need to check the original function's param types, not the FFI wrapper
    let param_type_ids = ctx.get_function_param_types(user_func_name);
    let all_param_types: Vec<doo_core::types::TypeId> = param_type_ids
        .map(|types| types.to_vec())
        .unwrap_or_default();

    let is_middleware = user_param_count == 2 && all_param_types.len() == 2 && {
        // Check if second param is "Next" type
        all_param_types
            .get(1)
            .map_or(false, |tid| match ctx.get_type_kind(*tid) {
                Some(doo_core::types::TypeKind::Struct { name, .. }) => {
                    name == "Next" || name == "DooNext"
                }
                _ => false,
            })
    };

    // For FFI functions, check if it's a middleware based on the ffi_symbol being passed to
    // (e.g., doo_http_get_with_middleware means this is used as middleware)
    // BUT only if the FFI function actually has the middleware signature (2 params)
    let is_ffi_middleware = ffi_symbol_info.is_some()
        && ffi_symbol.ends_with("_with_middleware")
        && user_param_count == 2;

    if debug {
        doo_debug!("CODEGEN", "User function {} has {} params, returns {:?}, return_type_name={:?}, returns_struct={}, is_middleware={}, is_ffi={}",
            user_func_name, user_param_count, user_return_type, return_type_name, returns_struct, is_middleware, ffi_symbol_info.is_some());
    }

    // Create wrapper function with FFI signature:
    // - Handlers: fn(ptr) -> ptr
    // - Middleware (true middleware with 2 params): fn(ptr, fn_ptr) -> ptr (request + next function)
    let ptr_type = ctx.ptr_type();
    let wrapper_fn_type = if is_middleware || is_ffi_middleware {
        ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false)
    } else {
        ptr_type.fn_type(&[ptr_type.into()], false)
    };
    let wrapper_fn = ctx
        .module
        .add_function(&wrapper_name, wrapper_fn_type, None);

    // Disable tail calls to prevent sret + tail call stack corruption on Windows x64
    let no_tail = ctx
        .context
        .create_string_attribute("disable-tail-calls", "true");
    wrapper_fn.add_attribute(inkwell::attributes::AttributeLoc::Function, no_tail);

    // Save current position
    let current_block = ctx.builder.get_insert_block();

    // Create wrapper body
    let entry = ctx.context.append_basic_block(wrapper_fn, "entry");
    ctx.builder.position_at_end(entry);

    // Get the request parameter
    let request_ptr = wrapper_fn
        .get_nth_param(0)
        .expect("ICE: wrapper function missing expected parameter")
        .into_pointer_value();

    // Types we'll need
    let i32_type = ctx.i32_type();
    let i64_type = ctx.i64_type();
    let _i8_type = ctx.context.i8_type();

    // Allocate result on heap (we'll need this for both success and error paths)
    // DooResult layout: { i64 tag, ptr data, i32 owner }
    let result_struct_type = ctx
        .context
        .struct_type(&[i64_type.into(), ptr_type.into(), i32_type.into()], false);

    let malloc_fn = ctx
        .module
        .get_function(ffi_names::MALLOC)
        .unwrap_or_else(|| {
            let fn_type = ptr_type.fn_type(&[i64_type.into()], false);
            ctx.module.add_function(ffi_names::MALLOC, fn_type, None)
        });

    // Call the user's function (or FFI function via external symbol)
    // For FFI functions, use their actual parameter count, not the middleware signature
    let user_result = if ffi_symbol_info.is_some() {
        // FFI function: call with the function's actual signature
        // For functions like jwt() that take no params, call with no args
        if user_param_count == 0 {
            ctx.builder
                .build_call(user_func, &[], "user_call")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
        } else if user_param_count == 1 {
            // FFI function with 1 param (e.g., request)
            ctx.builder
                .build_call(user_func, &[request_ptr.into()], "user_call")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
        } else if user_param_count == 2 && (is_middleware || is_ffi_middleware) {
            // FFI middleware with 2 params (request, next)
            let next_fn_ptr = wrapper_fn
                .get_nth_param(1)
                .expect("ICE: wrapper function missing expected parameter")
                .into_pointer_value();
            ctx.builder
                .build_call(
                    user_func,
                    &[request_ptr.into(), next_fn_ptr.into()],
                    "user_call",
                )
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
        } else {
            // Fallback: try with request only
            ctx.builder
                .build_call(user_func, &[request_ptr.into()], "user_call")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
        }
    } else if is_middleware {
        // Middleware function: fn(Request, Next) -> Response
        // Get the next function pointer from wrapper's second param
        let next_fn_ptr = wrapper_fn
            .get_nth_param(1)
            .expect("ICE: wrapper function missing expected parameter")
            .into_pointer_value();

        // Call user's middleware with both request and next
        ctx.builder
            .build_call(
                user_func,
                &[request_ptr.into(), next_fn_ptr.into()],
                "user_call",
            )
            .ok()
            .and_then(|cs| cs.try_as_basic_value().basic())
    } else if user_param_count == 0 {
        // Simple handler: fn() -> Str - no validation needed
        ctx.builder
            .build_call(user_func, &[], "user_call")
            .ok()
            .and_then(|cs| cs.try_as_basic_value().basic())
    } else {
        // Handler with struct parameter: validate request body first
        // Get or declare doohttp_populate_struct_from_request
        let populate_fn = ctx
            .module
            .get_function(http_pkg::DOOHTTP_POPULATE_STRUCT_FROM_REQUEST)
            .unwrap_or_else(|| {
                let fn_type = i32_type.fn_type(
                    &[
                        ptr_type.into(),
                        ptr_type.into(),
                        i32_type.into(),
                        ptr_type.into(),
                    ],
                    false,
                );
                ctx.module.add_function(
                    http_pkg::DOOHTTP_POPULATE_STRUCT_FROM_REQUEST,
                    fn_type,
                    None,
                )
            });

        // Get handler name as C string for the validation call
        let handler_name_str = ctx.const_string(user_func_name);

        // Call populate_struct_from_request to validate the body
        // Arguments: request_ptr, struct_ptr (null - we just want validation), source_type (0=body), handler_name
        let validate_result = ctx
            .builder
            .build_call(
                populate_fn,
                &[
                    request_ptr.into(),
                    ptr_type.const_null().into(), // struct_ptr - null since we just want validation
                    i32_type.const_int(0, false).into(), // source_type = 0 (body)
                    handler_name_str.into(),
                ],
                "validate_result",
            )
            .ok()
            .and_then(|cs| cs.try_as_basic_value().basic())
            .map(|v| v.into_int_value())
            .unwrap_or_else(|| i32_type.const_int(0, false));

        // Check if validation failed (non-zero = error)
        let validation_failed = ctx
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                validate_result,
                i32_type.const_zero(),
                "validation_failed",
            )
            .ok();

        if let Some(validation_failed) = validation_failed {
            // Create error and success blocks
            let Some(parent) = ctx.current_function() else {
                return wrapper_fn;
            };
            let error_block = ctx.context.append_basic_block(parent, "validation_error");
            let success_block = ctx.context.append_basic_block(parent, "validation_success");

            ctx.builder
                .build_conditional_branch(validation_failed, error_block, success_block)
                .ok();

            // Error block: return RFC 7807 error from last_error
            ctx.builder.position_at_end(error_block);

            // Get error status and JSON
            let get_status_fn = ctx
                .module
                .get_function(http_pkg::DOOHTTP_LAST_ERROR_STATUS)
                .unwrap_or_else(|| {
                    let fn_type = i32_type.fn_type(&[], false);
                    ctx.module
                        .add_function(http_pkg::DOOHTTP_LAST_ERROR_STATUS, fn_type, None)
                });

            let get_json_fn = ctx
                .module
                .get_function(http_pkg::DOOHTTP_LAST_ERROR_JSON)
                .unwrap_or_else(|| {
                    let fn_type = ptr_type.fn_type(&[], false);
                    ctx.module
                        .add_function(http_pkg::DOOHTTP_LAST_ERROR_JSON, fn_type, None)
                });

            let error_status = ctx
                .builder
                .build_call(get_status_fn, &[], "error_status")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
                .map(|v| v.into_int_value())
                .unwrap_or_else(|| i32_type.const_int(400, false));

            let error_json = ctx
                .builder
                .build_call(get_json_fn, &[], "error_json")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
                .map(|v| v.into_pointer_value())
                .unwrap_or_else(|| ptr_type.const_null());

            // Build error response struct { status, body, content_type }
            let error_response_type = ctx
                .context
                .struct_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);
            let error_response_size = i64_type.const_int(24, false);
            let error_response_ptr = ctx
                .builder
                .build_call(malloc_fn, &[error_response_size.into()], "error_response")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
                .map(|v| v.into_pointer_value())
                .unwrap_or_else(|| ptr_type.const_null());

            // Set status
            if let Ok(status_ptr) = ctx.builder.build_struct_gep(
                error_response_type,
                error_response_ptr,
                0,
                "status_ptr",
            ) {
                let _ = ctx.builder.build_store(status_ptr, error_status);
            }
            // Set body
            if let Ok(body_ptr) =
                ctx.builder
                    .build_struct_gep(error_response_type, error_response_ptr, 1, "body_ptr")
            {
                let _ = ctx.builder.build_store(body_ptr, error_json);
            }
            // Set content_type (application/json)
            let json_content_type = ctx.const_string("application/json");
            if let Ok(ct_ptr) =
                ctx.builder
                    .build_struct_gep(error_response_type, error_response_ptr, 2, "ct_ptr")
            {
                let _ = ctx.builder.build_store(ct_ptr, json_content_type);
            }

            // Build DooResult for error: { tag=1, value=error_response, owner=1 }
            let result_size = i64_type.const_int(24, false);
            let error_result_ptr = ctx
                .builder
                .build_call(malloc_fn, &[result_size.into()], "error_result")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
                .map(|v| v.into_pointer_value())
                .unwrap_or_else(|| ptr_type.const_null());

            if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                result_struct_type,
                error_result_ptr,
                0,
                "error_tag_ptr",
            ) {
                let _ = ctx
                    .builder
                    .build_store(tag_ptr, i64_type.const_int(1, false)); // tag = 1 (error)
            }
            if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                result_struct_type,
                error_result_ptr,
                1,
                "error_value_ptr",
            ) {
                let _ = ctx.builder.build_store(value_ptr, error_response_ptr);
            }
            if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                result_struct_type,
                error_result_ptr,
                2,
                "error_owner_ptr",
            ) {
                let _ = ctx
                    .builder
                    .build_store(owner_ptr, i32_type.const_int(1, false)); // owner = 1 (FFI)
            }

            let _ = ctx.builder.build_return(Some(&error_result_ptr));

            // Success block: call the user's function
            ctx.builder.position_at_end(success_block);
        }

        // Get ALL parameter types of the user function
        let param_type_ids = ctx.get_function_param_types(user_func_name);
        let all_param_types: Vec<doo_core::types::TypeId> = param_type_ids
            .map(|types| types.to_vec())
            .unwrap_or_default();
        let first_param_type = all_param_types.first().copied();

        // Check if the first parameter is a special "Request" type that receives raw pointer
        let is_raw_request = first_param_type.map_or(false, |tid| match ctx.get_type_kind(tid) {
            Some(doo_core::types::TypeKind::Struct { name, .. }) => {
                name == "Request" || name == "DooRequest"
            }
            Some(doo_core::types::TypeKind::TypeRef { name }) => {
                name == "Request" || name == "DooRequest"
            }
            _ => false,
        });

        if is_raw_request || first_param_type.is_none() {
            // User function expects raw request pointer
            ctx.builder
                .build_call(user_func, &[request_ptr.into()], "user_call")
                .ok()
                .and_then(|cs| cs.try_as_basic_value().basic())
        } else {
            // User function expects parsed struct(s) - handle single and multi-param cases
            // DooRequest layout: { *method, *path, *body, *headers, *params, *query, *user_id }
            let doo_request_type = ctx.context.struct_type(
                &[
                    ptr_type.into(), // 0: method
                    ptr_type.into(), // 1: path
                    ptr_type.into(), // 2: body
                    ptr_type.into(), // 3: headers
                    ptr_type.into(), // 4: params (path params as JSON)
                    ptr_type.into(), // 5: query
                    ptr_type.into(), // 6: user_id (JWT claims)
                ],
                false,
            );

            // Helper to load a field from request by index
            let load_request_field =
                |ctx: &mut CodegenContext<'ctx>, index: u32, name: &str| -> PointerValue<'ctx> {
                    ctx.builder
                        .build_struct_gep(
                            doo_request_type,
                            request_ptr,
                            index,
                            &format!("{}_field_ptr", name),
                        )
                        .ok()
                        .and_then(|gep| ctx.builder.build_load(ptr_type, gep, name).ok())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null())
                };

            // Build arguments for the user function call
            let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
            let param_count = all_param_types.len();

            // Get path param names from route context
            let path_param_names = route_context.path_param_names();
            let has_jwt_middleware = route_context.has_jwt_middleware();

            if debug {
                doo_debug!(
                    "CODEGEN",
                    "Handler {} with {} params, path_params={:?}, jwt={}",
                    user_func_name,
                    param_count,
                    path_param_names,
                    has_jwt_middleware
                );
            }

            // Get or declare doo_json_get_field for extracting specific fields from params JSON
            let json_get_field_fn = ctx
                .module
                .get_function(ffi_names::DOO_JSON_GET_FIELD)
                .unwrap_or_else(|| {
                    let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                    ctx.module
                        .add_function(ffi_names::DOO_JSON_GET_FIELD, fn_type, None)
                });

            for (idx, param_type) in all_param_types.iter().enumerate() {
                // Determine the correct source field for this parameter:
                // 0. Server type -> inject global server instance (no parsing)
                // 1. JWT middleware + single Int param -> user_id (index 6)
                // 2. Path param match -> params (index 4)
                //    - For STRUCT types: pass entire params JSON (struct fields extracted by emit_parse_struct)
                //    - For primitive types: extract specific field value first
                // 3. Otherwise -> body (index 2)

                // Check if this param is a Server type — inject global server instance
                let is_server_param = ctx
                    .get_type_kind(*param_type)
                    .map(|k| matches!(k, TypeKind::Struct { ref name, .. } if name == "Server" || name == "DooServer"))
                    .unwrap_or(false);

                if is_server_param {
                    // Server param: call doo_http_get_server_instance() → ptr
                    if debug {
                        doo_debug!(
                            "CODEGEN",
                            "Param {} is Server — injecting global server instance",
                            idx
                        );
                    }
                    let get_server_fn = ctx
                        .module
                        .get_function(http_pkg::DOO_HTTP_GET_SERVER_INSTANCE)
                        .unwrap_or_else(|| {
                            let fn_type = ptr_type.fn_type(&[], false);
                            ctx.module.add_function(
                                http_pkg::DOO_HTTP_GET_SERVER_INSTANCE,
                                fn_type,
                                Some(inkwell::module::Linkage::External),
                            )
                        });
                    let server_ptr = ctx
                        .builder
                        .build_call(get_server_fn, &[], "server_instance")
                        .ok()
                        .and_then(|cs| cs.try_as_basic_value().basic())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());
                    call_args.push(server_ptr.into());
                    continue;
                }

                let is_int_param = ctx
                    .get_type_kind(*param_type)
                    .map(|k| matches!(k, TypeKind::Int))
                    .unwrap_or(false);

                // Check if param type is a struct (needs whole params JSON, not extracted field)
                let is_struct_param = ctx
                    .get_type_kind(*param_type)
                    .map(|k| matches!(k, TypeKind::Struct { .. }))
                    .unwrap_or(false);

                let source_ptr = if has_jwt_middleware && param_count == 1 && is_int_param {
                    // JWT handler with single Int param - get from user_id field
                    if debug {
                        doo_debug!("CODEGEN", "Param {} from user_id (JWT)", idx);
                    }
                    load_request_field(ctx, 6, "user_id")
                } else if idx < path_param_names.len() {
                    // This param corresponds to a path parameter
                    let path_param_name = path_param_names.get(idx).cloned().unwrap_or_default();
                    if debug {
                        doo_debug!(
                            "CODEGEN",
                            "Param {} from params (path param: {}, is_struct={})",
                            idx,
                            path_param_name,
                            is_struct_param
                        );
                    }
                    let params_json = load_request_field(ctx, 4, "params");

                    if is_struct_param {
                        // For STRUCT types: pass entire params JSON object
                        // emit_parse_struct will extract individual fields from it
                        params_json
                    } else {
                        // For primitive types: extract the specific field value first
                        // params is like {"id": "123"}, extract "id" → "123"
                        let field_name_str = ctx.const_string(&path_param_name);
                        ctx.builder
                            .build_call(
                                json_get_field_fn,
                                &[params_json.into(), field_name_str.into()],
                                "field_json",
                            )
                            .ok()
                            .and_then(|cs| cs.try_as_basic_value().basic())
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ptr_type.const_null())
                    }
                } else {
                    // Default: use body
                    if debug {
                        doo_debug!("CODEGEN", "Param {} from body", idx);
                    }
                    load_request_field(ctx, 2, "body_json")
                };

                let parsed = JsonBuiltins::emit_parse(ctx, source_ptr.into(), Some(*param_type));

                if let Some(val) = parsed {
                    call_args.push(val.into());
                } else {
                    // Fallback: pass null pointer for this param
                    if debug {
                        doo_debug!(
                            "CODEGEN",
                            "Warning: Failed to parse param {} for {}",
                            idx,
                            user_func_name
                        );
                    }
                    call_args.push(ptr_type.const_null().into());
                }
            }

            // Call user function with all parsed arguments
            if call_args.len() == param_count {
                ctx.builder
                    .build_call(user_func, &call_args, "user_call")
                    .ok()
                    .and_then(|cs| cs.try_as_basic_value().basic())
            } else {
                // Fallback to passing request_ptr if parsing fails
                if debug {
                    doo_debug!(
                        "CODEGEN",
                        "Warning: Param count mismatch for {}, expected {} got {}",
                        user_func_name,
                        param_count,
                        call_args.len()
                    );
                }
                None
            }
        }
    };

    // For middleware, we need to wrap the result in DooResult format
    // Middleware can return either:
    // - Response directly (ptr) -> wrap in DooResult { tag=0, value=response }
    // - Result<Response, Error> ({ i32, ptr }) -> extract and rewrap in DooResult
    if is_middleware {
        // Check if the LLVM return type is a struct { i32, ptr } which indicates Result type
        // This is more reliable than checking return_type_name since that might not capture Result wrapper
        let user_returns_result_struct = user_return_type
            .map(|rt| {
                if let inkwell::types::BasicTypeEnum::StructType(st) = rt {
                    // Result<T, E> is represented as { i32 tag, ptr value }
                    st.count_fields() == 2
                } else {
                    false
                }
            })
            .unwrap_or(false);

        // Get the error type info for this middleware function (if it returns Result)
        let error_type_id = ctx.get_function_error_type(user_func_name);
        let error_type_name = error_type_id.and_then(|tid| {
            ctx.get_type_kind(tid).map(|tk| match tk {
                TypeKind::Enum { name, .. } => name.clone(),
                _ => String::new(),
            })
        });

        if debug {
            doo_debug!(
                "CODEGEN",
                "Middleware {} user_returns_result_struct={} error_type={:?}",
                user_func_name,
                user_returns_result_struct,
                error_type_name
            );
        }

        // Allocate DooResult on heap
        let result_size = i64_type.const_int(24, false);
        let doo_result_ptr = ctx
            .builder
            .build_call(malloc_fn, &[result_size.into()], "doo_result")
            .ok()
            .and_then(|cs| cs.try_as_basic_value().basic())
            .map(|v| v.into_pointer_value())
            .unwrap_or_else(|| ptr_type.const_null());

        if let Some(val) = user_result {
            if debug {
                doo_debug!(
                    "CODEGEN",
                    "Middleware {} val.is_struct_value()={}, user_returns_result_struct={}",
                    user_func_name,
                    val.is_struct_value(),
                    user_returns_result_struct
                );
            }

            // If the function returns a struct type { i64, ptr } (SimpleResult), we need to extract values
            // Try to convert to struct value if the return type indicates it's a result struct
            if user_returns_result_struct && error_type_name.is_some() {
                // The call returns { i64, ptr } directly as a struct value
                // We need to extract the tag and value from it
                if let Ok(user_result_struct) = val.try_into() {
                    let user_result_struct: inkwell::values::StructValue = user_result_struct;

                    // Extract i64 tag
                    let tag = ctx
                        .builder
                        .build_extract_value(user_result_struct, 0, "result_tag")
                        .map(|v| v.into_int_value())
                        .unwrap_or_else(|_| i64_type.const_int(0, false));

                    // Extract ptr value directly (no int_to_ptr needed)
                    let value = ctx
                        .builder
                        .build_extract_value(user_result_struct, 1, "result_value_ptr")
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|_| ptr_type.const_null());

                    // Create blocks for Ok and Err paths
                    let Some(parent) = ctx.current_function() else {
                        return wrapper_fn;
                    };
                    let ok_block = ctx.context.append_basic_block(parent, "middleware_ok");
                    let err_block = ctx.context.append_basic_block(parent, "middleware_err");
                    let merge_block = ctx.context.append_basic_block(parent, "middleware_merge");

                    // Branch based on tag (0 = Ok, non-zero = Err) - use i64 constant
                    let is_err = ctx
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::NE,
                            tag,
                            i64_type.const_zero(),
                            "is_err",
                        )
                        .unwrap();
                    ctx.builder
                        .build_conditional_branch(is_err, err_block, ok_block)
                        .ok();

                    // OK PATH: Extract Body field from Response struct
                    // Response struct layout: { i64 Status, ptr Body, ptr ContentType }
                    ctx.builder.position_at_end(ok_block);
                    let response_struct_type = ctx
                        .context
                        .struct_type(&[i64_type.into(), ptr_type.into(), ptr_type.into()], false);

                    // Load the Status field (index 0) from the Response struct
                    let status_field = ctx
                        .builder
                        .build_struct_gep(response_struct_type, value, 0, "status_field_ptr")
                        .ok()
                        .and_then(|gep| {
                            ctx.builder
                                .build_load(i64_type, gep, "response_status")
                                .ok()
                        })
                        .map(|v| v.into_int_value())
                        .unwrap_or_else(|| i64_type.const_int(200, false));

                    // Load the Body field (index 1) from the Response struct pointer
                    let body_field_ptr = ctx
                        .builder
                        .build_struct_gep(response_struct_type, value, 1, "body_field_ptr")
                        .ok()
                        .and_then(|gep| ctx.builder.build_load(ptr_type, gep, "response_body").ok())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());

                    // Load the ContentType field (index 2) from the Response struct
                    let ct_field_ptr = ctx
                        .builder
                        .build_struct_gep(response_struct_type, value, 2, "ct_field_ptr")
                        .ok()
                        .and_then(|gep| ctx.builder.build_load(ptr_type, gep, "response_ct").ok())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());

                    // Check if Response.Status >= 400 (error from inner middleware)
                    // If so, propagate as error DooResult instead of wrapping as Ok
                    let is_error_status = ctx
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::SGE,
                            status_field,
                            i64_type.const_int(400, false),
                            "is_error_status",
                        )
                        .unwrap();

                    let Some(current_fn) = ctx.current_function() else {
                        return wrapper_fn;
                    };
                    let ok_normal = ctx.context.append_basic_block(current_fn, "ok_normal");
                    let ok_error_passthrough = ctx
                        .context
                        .append_basic_block(current_fn, "ok_error_passthrough");
                    ctx.builder
                        .build_conditional_branch(is_error_status, ok_error_passthrough, ok_normal)
                        .ok();

                    // NORMAL OK PATH: status < 400, wrap as DooResult tag=0
                    ctx.builder.position_at_end(ok_normal);
                    if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        0,
                        "ok_tag_ptr",
                    ) {
                        let _ = ctx.builder.build_store(tag_ptr, i64_type.const_zero());
                    }
                    if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        1,
                        "ok_value_ptr",
                    ) {
                        let _ = ctx.builder.build_store(value_ptr, body_field_ptr);
                    }
                    if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        2,
                        "ok_owner_ptr",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(owner_ptr, i32_type.const_int(1, false));
                    }
                    ctx.builder.build_unconditional_branch(merge_block).ok();

                    // ERROR PASSTHROUGH PATH: status >= 400, build error struct and set tag=1
                    // This propagates errors from inner middleware that were passed through
                    ctx.builder.position_at_end(ok_error_passthrough);

                    // Build error struct: { i32 status, ptr body, ptr content_type }
                    let error_struct_type_inner = ctx
                        .context
                        .struct_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);
                    let error_struct_size = error_struct_type_inner
                        .size_of()
                        .unwrap_or(i64_type.const_int(24, false));

                    // Declare malloc
                    let malloc_fn =
                        ctx.module
                            .get_function(ffi_names::MALLOC)
                            .unwrap_or_else(|| {
                                let fn_type = ptr_type.fn_type(&[i64_type.into()], false);
                                ctx.module.add_function(ffi_names::MALLOC, fn_type, None)
                            });

                    let error_struct_mem = ctx
                        .builder
                        .build_call(malloc_fn, &[error_struct_size.into()], "error_struct_mem")
                        .ok()
                        .and_then(|cs| cs.try_as_basic_value().basic())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());

                    // Truncate i64 status to i32 for the error struct
                    let status_i32 = ctx
                        .builder
                        .build_int_truncate(status_field, i32_type, "status_i32")
                        .unwrap();

                    // Store status, body, content_type into error struct
                    if let Ok(ep) = ctx.builder.build_struct_gep(
                        error_struct_type_inner,
                        error_struct_mem,
                        0,
                        "err_status_ptr",
                    ) {
                        let _ = ctx.builder.build_store(ep, status_i32);
                    }
                    if let Ok(ep) = ctx.builder.build_struct_gep(
                        error_struct_type_inner,
                        error_struct_mem,
                        1,
                        "err_body_ptr",
                    ) {
                        let _ = ctx.builder.build_store(ep, body_field_ptr);
                    }
                    if let Ok(ep) = ctx.builder.build_struct_gep(
                        error_struct_type_inner,
                        error_struct_mem,
                        2,
                        "err_ct_ptr",
                    ) {
                        let _ = ctx.builder.build_store(ep, ct_field_ptr);
                    }

                    // Store in DooResult with tag=1 (Error)
                    if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        0,
                        "passthrough_tag_ptr",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(tag_ptr, i64_type.const_int(1, false));
                    }
                    if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        1,
                        "passthrough_value_ptr",
                    ) {
                        let _ = ctx.builder.build_store(value_ptr, error_struct_mem);
                    }
                    if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        2,
                        "passthrough_owner_ptr",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(owner_ptr, i32_type.const_int(1, false));
                    }
                    ctx.builder.build_unconditional_branch(merge_block).ok();

                    // ERROR PATH: Map error enum variant to HTTP status and build error response
                    ctx.builder.position_at_end(err_block);

                    // Get error enum name and variant index from the value pointer
                    // The value pointer points to a struct { i32 variant_index, ptr payload }
                    let error_struct_type = ctx
                        .context
                        .struct_type(&[i32_type.into(), ptr_type.into()], false);

                    let variant_index = ctx
                        .builder
                        .build_struct_gep(error_struct_type, value, 0, "variant_idx_ptr")
                        .ok()
                        .and_then(|gep| ctx.builder.build_load(i32_type, gep, "variant_idx").ok())
                        .map(|v| v.into_int_value())
                        .unwrap_or_else(|| i32_type.const_zero());

                    // Get enum metadata to get variant names
                    let enum_name_str = error_type_name
                        .as_ref()
                        .expect("ICE: error type has no name — cannot generate error handler");
                    let variant_names = if let Some(TypeKind::Enum { variants, .. }) =
                        error_type_id.and_then(|tid| ctx.get_type_kind(tid))
                    {
                        variants
                            .iter()
                            .map(|(name, _)| name.clone())
                            .collect::<Vec<_>>()
                    } else {
                        vec!["Unknown".to_string()]
                    };

                    // Build variant name lookup using a simple select/switch approach
                    // For enums with 2 variants (like AuthError), just use a select instruction
                    // For larger enums, we'd build a proper switch, but for now this is simpler
                    let variant_name_ptr = if variant_names.len() == 2 {
                        // Simple case: use select for binary choice
                        // select i1 (variant_idx == 0), ptr "Unauthorized", ptr "Forbidden"
                        let is_zero = ctx
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                variant_index,
                                i32_type.const_zero(),
                                "is_variant_0",
                            )
                            .unwrap();
                        let name0 = ctx.const_string(&variant_names[0]);
                        let name1 = ctx.const_string(&variant_names[1]);
                        ctx.builder
                            .build_select(is_zero, name0, name1, "variant_name")
                            .ok()
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ptr_type.const_null())
                    } else {
                        // Fallback: just use first variant name (should build proper switch for 3+ variants)
                        ctx.const_string(
                            &variant_names
                                .get(0)
                                .cloned()
                                .unwrap_or_else(|| "Unknown".to_string()),
                        )
                    };

                    // Call doohttp_error_variant_to_status to get HTTP status code
                    let error_mapping_fn = ctx
                        .module
                        .get_function(http_pkg::DOOHTTP_ERROR_VARIANT_TO_STATUS)
                        .unwrap_or_else(|| {
                            let fn_type = i32_type.fn_type(
                                &[ptr_type.into(), ptr_type.into(), i32_type.into()],
                                false,
                            );
                            ctx.module.add_function(
                                http_pkg::DOOHTTP_ERROR_VARIANT_TO_STATUS,
                                fn_type,
                                None,
                            )
                        });

                    let enum_name_cstr = ctx.const_string(enum_name_str);

                    let http_status = ctx
                        .builder
                        .build_call(
                            error_mapping_fn,
                            &[
                                enum_name_cstr.into(),
                                variant_name_ptr.into(),
                                variant_index.into(),
                            ],
                            "http_status",
                        )
                        .ok()
                        .and_then(|cs| cs.try_as_basic_value().basic())
                        .map(|v| v.into_int_value())
                        .unwrap_or_else(|| i32_type.const_int(500, false));

                    // Get the doohttp_build_rfc7807_error function to create proper error JSON
                    let build_error_fn = ctx
                        .module
                        .get_function(http_pkg::DOOHTTP_BUILD_RFC7807_ERROR)
                        .unwrap_or_else(|| {
                            let fn_type =
                                ptr_type.fn_type(&[i32_type.into(), ptr_type.into()], false);
                            ctx.module.add_function(
                                http_pkg::DOOHTTP_BUILD_RFC7807_ERROR,
                                fn_type,
                                None,
                            )
                        });

                    // Build RFC 7807 error JSON using the helper
                    let error_json_ptr = ctx
                        .builder
                        .build_call(
                            build_error_fn,
                            &[http_status.into(), variant_name_ptr.into()],
                            "error_json",
                        )
                        .ok()
                        .and_then(|cs| cs.try_as_basic_value().basic())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());

                    // Build error response struct { i32 status, ptr body, ptr content_type }
                    let error_response_type = ctx
                        .context
                        .struct_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);
                    let error_response_size = i64_type.const_int(24, false);
                    let error_response_ptr = ctx
                        .builder
                        .build_call(malloc_fn, &[error_response_size.into()], "error_response")
                        .ok()
                        .and_then(|cs| cs.try_as_basic_value().basic())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());

                    if let Ok(status_ptr) = ctx.builder.build_struct_gep(
                        error_response_type,
                        error_response_ptr,
                        0,
                        "err_status_ptr",
                    ) {
                        let _ = ctx.builder.build_store(status_ptr, http_status);
                    }
                    if let Ok(body_ptr) = ctx.builder.build_struct_gep(
                        error_response_type,
                        error_response_ptr,
                        1,
                        "err_body_ptr",
                    ) {
                        let _ = ctx.builder.build_store(body_ptr, error_json_ptr);
                    }
                    let json_content_type = ctx.const_string("application/json");
                    if let Ok(ct_ptr) = ctx.builder.build_struct_gep(
                        error_response_type,
                        error_response_ptr,
                        2,
                        "err_ct_ptr",
                    ) {
                        let _ = ctx.builder.build_store(ct_ptr, json_content_type);
                    }

                    // Store in DooResult with tag=1 (Err)
                    if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        0,
                        "err_tag_ptr",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(tag_ptr, i64_type.const_int(1, false));
                    }
                    if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        1,
                        "err_value_ptr",
                    ) {
                        let _ = ctx.builder.build_store(value_ptr, error_response_ptr);
                    }
                    if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        2,
                        "err_owner_ptr",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(owner_ptr, i32_type.const_int(1, false));
                    }
                    ctx.builder.build_unconditional_branch(merge_block).ok();

                    // MERGE: Return the DooResult
                    ctx.builder.position_at_end(merge_block);
                } else {
                    // Fallback: treat as pointer
                    if debug {
                        doo_debug!(
                            "CODEGEN",
                            "Warning: Failed to extract struct value for {}",
                            user_func_name
                        );
                    }
                    let response_ptr = if val.is_pointer_value() {
                        val.into_pointer_value()
                    } else {
                        ptr_type.const_null()
                    };

                    // Check if return type is Response - if so, check status + extract
                    let should_extract_body = return_type_name.as_deref() == Some("Response");

                    if should_extract_body {
                        // Response struct: { i64 Status, ptr Body, ptr ContentType }
                        // Must check Status to properly propagate errors through middleware chain
                        let response_struct_type = ctx.context.struct_type(
                            &[i64_type.into(), ptr_type.into(), ptr_type.into()],
                            false,
                        );

                        // Read Status field (index 0)
                        let status_val = ctx
                            .builder
                            .build_struct_gep(
                                response_struct_type,
                                response_ptr,
                                0,
                                "status_field_ptr",
                            )
                            .ok()
                            .and_then(|gep| {
                                ctx.builder
                                    .build_load(i64_type, gep, "response_status")
                                    .ok()
                            })
                            .map(|v| v.into_int_value())
                            .unwrap_or_else(|| i64_type.const_int(200, false));

                        // Read Body field (index 1)
                        let body_val = ctx
                            .builder
                            .build_struct_gep(
                                response_struct_type,
                                response_ptr,
                                1,
                                "body_field_ptr",
                            )
                            .ok()
                            .and_then(|gep| {
                                ctx.builder.build_load(ptr_type, gep, "response_body").ok()
                            })
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ptr_type.const_null());

                        // Check if status >= 400 (error response from inner middleware)
                        let is_error = ctx
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::SGE,
                                status_val,
                                i64_type.const_int(400, false),
                                "is_error_status",
                            )
                            .unwrap();

                        let Some(parent) = ctx.current_function() else {
                            return wrapper_fn;
                        };
                        let resp_ok_block = ctx.context.append_basic_block(parent, "resp_ok");
                        let resp_err_block = ctx.context.append_basic_block(parent, "resp_err");
                        let resp_merge_block = ctx.context.append_basic_block(parent, "resp_merge");

                        ctx.builder
                            .build_conditional_branch(is_error, resp_err_block, resp_ok_block)
                            .ok();

                        // OK PATH: status < 400 → DooResult{tag=0, value=body}
                        ctx.builder.position_at_end(resp_ok_block);
                        if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                            result_struct_type,
                            doo_result_ptr,
                            0,
                            "ok_tag",
                        ) {
                            let _ = ctx.builder.build_store(tag_ptr, i64_type.const_zero());
                        }
                        if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                            result_struct_type,
                            doo_result_ptr,
                            1,
                            "ok_value",
                        ) {
                            let _ = ctx.builder.build_store(value_ptr, body_val);
                        }
                        if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                            result_struct_type,
                            doo_result_ptr,
                            2,
                            "ok_owner",
                        ) {
                            let _ = ctx
                                .builder
                                .build_store(owner_ptr, i32_type.const_int(1, false));
                        }
                        ctx.builder
                            .build_unconditional_branch(resp_merge_block)
                            .ok();

                        // ERR PATH: status >= 400 → DooResult{tag=1, value=error_response}
                        ctx.builder.position_at_end(resp_err_block);
                        let error_struct_type = ctx.context.struct_type(
                            &[i32_type.into(), ptr_type.into(), ptr_type.into()],
                            false,
                        );

                        let error_struct_size = i64_type.const_int(24, false);
                        let malloc_fn = ctx
                            .module
                            .get_function(ffi_names::MALLOC)
                            .expect("ICE: malloc not declared in LLVM module");
                        let error_struct_ptr = ctx
                            .builder
                            .build_call(malloc_fn, &[error_struct_size.into()], "error_response")
                            .ok()
                            .and_then(|cv| cv.try_as_basic_value().basic())
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ptr_type.const_null());

                        let status_i32 = ctx
                            .builder
                            .build_int_truncate(status_val, i32_type, "status_i32")
                            .expect("ICE: failed to truncate HTTP status to i32");
                        if let Ok(gep) = ctx.builder.build_struct_gep(
                            error_struct_type,
                            error_struct_ptr,
                            0,
                            "err_status",
                        ) {
                            let _ = ctx.builder.build_store(gep, status_i32);
                        }
                        if let Ok(gep) = ctx.builder.build_struct_gep(
                            error_struct_type,
                            error_struct_ptr,
                            1,
                            "err_body",
                        ) {
                            let _ = ctx.builder.build_store(gep, body_val);
                        }
                        let ct_val = ctx
                            .builder
                            .build_struct_gep(response_struct_type, response_ptr, 2, "ct_field_ptr")
                            .ok()
                            .and_then(|gep| {
                                ctx.builder.build_load(ptr_type, gep, "response_ct").ok()
                            })
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ctx.const_string("application/json"));
                        if let Ok(gep) = ctx.builder.build_struct_gep(
                            error_struct_type,
                            error_struct_ptr,
                            2,
                            "err_ct",
                        ) {
                            let _ = ctx.builder.build_store(gep, ct_val);
                        }

                        if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                            result_struct_type,
                            doo_result_ptr,
                            0,
                            "err_tag",
                        ) {
                            let _ = ctx
                                .builder
                                .build_store(tag_ptr, i64_type.const_int(1, false));
                        }
                        if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                            result_struct_type,
                            doo_result_ptr,
                            1,
                            "err_value",
                        ) {
                            let _ = ctx.builder.build_store(value_ptr, error_struct_ptr);
                        }
                        if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                            result_struct_type,
                            doo_result_ptr,
                            2,
                            "err_owner",
                        ) {
                            let _ = ctx
                                .builder
                                .build_store(owner_ptr, i32_type.const_int(1, false));
                        }
                        ctx.builder
                            .build_unconditional_branch(resp_merge_block)
                            .ok();

                        // MERGE
                        ctx.builder.position_at_end(resp_merge_block);
                    } else {
                        // Serialize the Response struct to JSON
                        let serialize_fn = ctx
                            .module
                            .get_function(http_pkg::DOOHTTP_SERIALIZE_STRUCT_TO_JSON)
                            .unwrap_or_else(|| {
                                let fn_type =
                                    ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                                ctx.module.add_function(
                                    http_pkg::DOOHTTP_SERIALIZE_STRUCT_TO_JSON,
                                    fn_type,
                                    None,
                                )
                            });

                        let handler_name_str = ctx
                            .builder
                            .build_global_string_ptr(user_func_name, "middleware_name_fallback")
                            .map(|g| g.as_pointer_value())
                            .unwrap_or_else(|_| ptr_type.const_null());

                        let result_value = ctx
                            .builder
                            .build_call(
                                serialize_fn,
                                &[response_ptr.into(), handler_name_str.into()],
                                "serialized_fallback",
                            )
                            .ok()
                            .and_then(|cs| cs.try_as_basic_value().basic())
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ptr_type.const_null());

                        if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                            result_struct_type,
                            doo_result_ptr,
                            0,
                            "tag_ptr",
                        ) {
                            let _ = ctx
                                .builder
                                .build_store(tag_ptr, i64_type.const_int(0, false));
                        }
                        if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                            result_struct_type,
                            doo_result_ptr,
                            1,
                            "value_ptr",
                        ) {
                            let _ = ctx.builder.build_store(value_ptr, result_value);
                        }
                        if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                            result_struct_type,
                            doo_result_ptr,
                            2,
                            "owner_ptr",
                        ) {
                            let _ = ctx
                                .builder
                                .build_store(owner_ptr, i32_type.const_int(1, false));
                        }
                    }
                }
            } else {
                // User returned Response directly (pointer)
                // Response struct layout: { i64 Status, ptr Body, ptr ContentType }
                let response_ptr = if val.is_pointer_value() {
                    val.into_pointer_value()
                } else {
                    ptr_type.const_null()
                };

                // Check if return type name is "Response" - if so, check status + extract
                // Otherwise serialize as before (for other return types)
                let should_extract_body = return_type_name.as_deref() == Some("Response");

                if should_extract_body {
                    // Response struct: { i64 Status, ptr Body, ptr ContentType }
                    // Must check Status to properly propagate errors through middleware chain
                    let response_struct_type = ctx
                        .context
                        .struct_type(&[i64_type.into(), ptr_type.into(), ptr_type.into()], false);

                    // Read Status field (index 0)
                    let status_val = ctx
                        .builder
                        .build_struct_gep(response_struct_type, response_ptr, 0, "status_field_ptr")
                        .ok()
                        .and_then(|gep| {
                            ctx.builder
                                .build_load(i64_type, gep, "response_status")
                                .ok()
                        })
                        .map(|v| v.into_int_value())
                        .unwrap_or_else(|| i64_type.const_int(200, false));

                    // Read Body field (index 1)
                    let body_val = ctx
                        .builder
                        .build_struct_gep(response_struct_type, response_ptr, 1, "body_field_ptr")
                        .ok()
                        .and_then(|gep| ctx.builder.build_load(ptr_type, gep, "response_body").ok())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());

                    // Check if status >= 400 (error response from inner middleware)
                    let is_error = ctx
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::SGE,
                            status_val,
                            i64_type.const_int(400, false),
                            "is_error_status",
                        )
                        .unwrap();

                    let Some(parent) = ctx.current_function() else {
                        return wrapper_fn;
                    };
                    let resp_ok_block = ctx.context.append_basic_block(parent, "resp_ok");
                    let resp_err_block = ctx.context.append_basic_block(parent, "resp_err");
                    let resp_merge_block = ctx.context.append_basic_block(parent, "resp_merge");

                    ctx.builder
                        .build_conditional_branch(is_error, resp_err_block, resp_ok_block)
                        .ok();

                    // OK PATH: status < 400 → DooResult{tag=0, value=body}
                    ctx.builder.position_at_end(resp_ok_block);
                    if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        0,
                        "ok_tag",
                    ) {
                        let _ = ctx.builder.build_store(tag_ptr, i64_type.const_zero());
                    }
                    if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        1,
                        "ok_value",
                    ) {
                        let _ = ctx.builder.build_store(value_ptr, body_val);
                    }
                    if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        2,
                        "ok_owner",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(owner_ptr, i32_type.const_int(1, false));
                    }
                    ctx.builder
                        .build_unconditional_branch(resp_merge_block)
                        .ok();

                    // ERR PATH: status >= 400 → DooResult{tag=1, value=error_response}
                    // Build error response struct { i32 status, ptr body, ptr ct }
                    ctx.builder.position_at_end(resp_err_block);
                    let error_struct_type = ctx
                        .context
                        .struct_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);

                    let error_struct_size = i64_type.const_int(24, false);
                    let malloc_fn = ctx
                        .module
                        .get_function(ffi_names::MALLOC)
                        .expect("ICE: malloc not declared in LLVM module");
                    let error_struct_ptr = ctx
                        .builder
                        .build_call(malloc_fn, &[error_struct_size.into()], "error_response")
                        .ok()
                        .and_then(|cv| cv.try_as_basic_value().basic())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());

                    // Truncate i64 status to i32 for error struct
                    let status_i32 = ctx
                        .builder
                        .build_int_truncate(status_val, i32_type, "status_i32")
                        .expect("ICE: failed to truncate HTTP status to i32");
                    if let Ok(gep) = ctx.builder.build_struct_gep(
                        error_struct_type,
                        error_struct_ptr,
                        0,
                        "err_status",
                    ) {
                        let _ = ctx.builder.build_store(gep, status_i32);
                    }
                    if let Ok(gep) = ctx.builder.build_struct_gep(
                        error_struct_type,
                        error_struct_ptr,
                        1,
                        "err_body",
                    ) {
                        let _ = ctx.builder.build_store(gep, body_val);
                    }
                    // Read ContentType field (index 2) from original Response
                    let ct_val = ctx
                        .builder
                        .build_struct_gep(response_struct_type, response_ptr, 2, "ct_field_ptr")
                        .ok()
                        .and_then(|gep| ctx.builder.build_load(ptr_type, gep, "response_ct").ok())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ctx.const_string("application/json"));
                    if let Ok(gep) = ctx.builder.build_struct_gep(
                        error_struct_type,
                        error_struct_ptr,
                        2,
                        "err_ct",
                    ) {
                        let _ = ctx.builder.build_store(gep, ct_val);
                    }

                    if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        0,
                        "err_tag",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(tag_ptr, i64_type.const_int(1, false));
                    }
                    if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        1,
                        "err_value",
                    ) {
                        let _ = ctx.builder.build_store(value_ptr, error_struct_ptr);
                    }
                    if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        2,
                        "err_owner",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(owner_ptr, i32_type.const_int(1, false));
                    }
                    ctx.builder
                        .build_unconditional_branch(resp_merge_block)
                        .ok();

                    // MERGE
                    ctx.builder.position_at_end(resp_merge_block);
                } else {
                    // Non-Response return type: serialize to JSON and store with tag=0
                    let serialize_fn = ctx
                        .module
                        .get_function(http_pkg::DOOHTTP_SERIALIZE_STRUCT_TO_JSON)
                        .unwrap_or_else(|| {
                            let fn_type =
                                ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                            ctx.module.add_function(
                                http_pkg::DOOHTTP_SERIALIZE_STRUCT_TO_JSON,
                                fn_type,
                                None,
                            )
                        });

                    let handler_name_str = ctx
                        .builder
                        .build_global_string_ptr(user_func_name, "middleware_name_direct")
                        .map(|g| g.as_pointer_value())
                        .unwrap_or_else(|_| ptr_type.const_null());

                    let result_value = ctx
                        .builder
                        .build_call(
                            serialize_fn,
                            &[response_ptr.into(), handler_name_str.into()],
                            "serialized_direct",
                        )
                        .ok()
                        .and_then(|cs| cs.try_as_basic_value().basic())
                        .map(|v| v.into_pointer_value())
                        .unwrap_or_else(|| ptr_type.const_null());

                    // Store in DooResult with tag=0 (Ok)
                    if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        0,
                        "tag_ptr",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(tag_ptr, i64_type.const_int(0, false));
                    }
                    if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        1,
                        "value_ptr",
                    ) {
                        let _ = ctx.builder.build_store(value_ptr, result_value);
                    }
                    if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                        result_struct_type,
                        doo_result_ptr,
                        2,
                        "owner_ptr",
                    ) {
                        let _ = ctx
                            .builder
                            .build_store(owner_ptr, i32_type.const_int(1, false));
                    }
                }
            }
        } else {
            // No result - return error
            if let Ok(tag_ptr) =
                ctx.builder
                    .build_struct_gep(result_struct_type, doo_result_ptr, 0, "tag_ptr")
            {
                let _ = ctx
                    .builder
                    .build_store(tag_ptr, i64_type.const_int(1, false));
            }
            if let Ok(value_ptr) =
                ctx.builder
                    .build_struct_gep(result_struct_type, doo_result_ptr, 1, "value_ptr")
            {
                let _ = ctx.builder.build_store(value_ptr, ptr_type.const_null());
            }
            if let Ok(owner_ptr) =
                ctx.builder
                    .build_struct_gep(result_struct_type, doo_result_ptr, 2, "owner_ptr")
            {
                let _ = ctx
                    .builder
                    .build_store(owner_ptr, i32_type.const_int(1, false));
            }
        }

        let _ = ctx.builder.build_return(Some(&doo_result_ptr));

        // Restore position
        if let Some(block) = current_block {
            ctx.builder.position_at_end(block);
        }

        return wrapper_fn;
    }

    // Check if handler returns a Result type (has error type in signature)
    // This handles non-middleware handlers like GetFeed that return [Post] ! DatabaseError
    let handler_error_type_id = ctx.get_function_error_type(user_func_name);
    let handler_returns_result = handler_error_type_id.is_some()
        && user_return_type.map_or(false, |rt| {
            if let inkwell::types::BasicTypeEnum::StructType(st) = rt {
                st.count_fields() == 2 // Result<T, E> is { i64 tag, ptr value }
            } else {
                false
            }
        });

    if handler_returns_result {
        // Handler returns Result<T, Error> - need to extract Ok value or handle Err
        // Allocate DooResult on heap
        let result_size = i64_type.const_int(24, false);
        let doo_result_ptr = ctx
            .builder
            .build_call(malloc_fn, &[result_size.into()], "doo_result")
            .ok()
            .and_then(|cs| cs.try_as_basic_value().basic())
            .map(|v| v.into_pointer_value())
            .unwrap_or_else(|| ptr_type.const_null());

        if let Some(val) = user_result {
            if let Ok(user_result_struct) = val.try_into() {
                let user_result_struct: inkwell::values::StructValue = user_result_struct;

                // Extract i64 tag (0 = Ok, 1 = Err)
                let tag = ctx
                    .builder
                    .build_extract_value(user_result_struct, 0, "result_tag")
                    .map(|v| v.into_int_value())
                    .unwrap_or_else(|_| i64_type.const_int(0, false));

                // Extract ptr value directly (no int_to_ptr needed)
                let value = ctx
                    .builder
                    .build_extract_value(user_result_struct, 1, "result_value_ptr")
                    .map(|v| v.into_pointer_value())
                    .unwrap_or_else(|_| ptr_type.const_null());

                // Create blocks for Ok and Err paths
                let Some(parent) = ctx.current_function() else {
                    return wrapper_fn;
                };
                let ok_block = ctx.context.append_basic_block(parent, "handler_ok");
                let err_block = ctx.context.append_basic_block(parent, "handler_err");

                // Branch based on tag (0 = Ok, non-zero = Err)
                let is_err = ctx
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        tag,
                        i64_type.const_zero(),
                        "is_err",
                    )
                    .unwrap();
                ctx.builder
                    .build_conditional_branch(is_err, err_block, ok_block)
                    .ok();

                // OK PATH: Serialize the result to JSON
                ctx.builder.position_at_end(ok_block);

                // For array results [T], we need to serialize the array
                // The value pointer points to the array data
                // Call doohttp_serialize_struct_to_json or similar
                let serialize_fn = ctx
                    .module
                    .get_function(http_pkg::DOOHTTP_SERIALIZE_STRUCT_TO_JSON)
                    .unwrap_or_else(|| {
                        let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                        ctx.module.add_function(
                            http_pkg::DOOHTTP_SERIALIZE_STRUCT_TO_JSON,
                            fn_type,
                            None,
                        )
                    });

                let handler_name_str = ctx
                    .builder
                    .build_global_string_ptr(user_func_name, "handler_name_for_serialize")
                    .map(|g| g.as_pointer_value())
                    .unwrap_or_else(|_| ptr_type.const_null());

                let json_ptr = ctx
                    .builder
                    .build_call(
                        serialize_fn,
                        &[value.into(), handler_name_str.into()],
                        "serialized_json",
                    )
                    .ok()
                    .and_then(|cs| cs.try_as_basic_value().basic())
                    .map(|v| v.into_pointer_value())
                    .unwrap_or_else(|| ptr_type.const_null());

                // Store in DooResult with tag=0 (Ok)
                if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                    result_struct_type,
                    doo_result_ptr,
                    0,
                    "ok_tag_ptr",
                ) {
                    let _ = ctx.builder.build_store(tag_ptr, i64_type.const_zero());
                }
                if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                    result_struct_type,
                    doo_result_ptr,
                    1,
                    "ok_value_ptr",
                ) {
                    let _ = ctx.builder.build_store(value_ptr, json_ptr);
                }
                if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                    result_struct_type,
                    doo_result_ptr,
                    2,
                    "ok_owner_ptr",
                ) {
                    let _ = ctx
                        .builder
                        .build_store(owner_ptr, i32_type.const_int(1, false));
                }
                let _ = ctx.builder.build_return(Some(&doo_result_ptr));

                // ERROR PATH: Build error response with actual error message
                ctx.builder.position_at_end(err_block);

                // Call doohttp_format_error_as_json to format the error message from the result
                // The error value from `return Err "message"` is already a char* pointer.
                // No extra dereference needed — use it directly.
                let format_error_fn = ctx
                    .module
                    .get_function(http_pkg::DOOHTTP_FORMAT_ERROR_AS_JSON)
                    .unwrap_or_else(|| {
                        let fn_type = ptr_type.fn_type(&[ptr_type.into()], false);
                        ctx.module.add_function(
                            http_pkg::DOOHTTP_FORMAT_ERROR_AS_JSON,
                            fn_type,
                            None,
                        )
                    });

                // Error value IS the string pointer directly (from `return Err "..."`)
                let error_msg_ptr = value;

                let error_json_str = ctx
                    .builder
                    .build_call(
                        format_error_fn,
                        &[error_msg_ptr.into()],
                        "formatted_error_json",
                    )
                    .ok()
                    .and_then(|cs| cs.try_as_basic_value().basic())
                    .map(|v| v.into_pointer_value())
                    .unwrap_or_else(|| ctx.const_string("{\"error\": \"Internal server error\"}"));

                // Build error response struct { status=500, body, content_type }
                let error_response_type = ctx
                    .context
                    .struct_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);
                let error_response_size = i64_type.const_int(24, false);
                let error_response_ptr = ctx
                    .builder
                    .build_call(malloc_fn, &[error_response_size.into()], "error_response")
                    .ok()
                    .and_then(|cs| cs.try_as_basic_value().basic())
                    .map(|v| v.into_pointer_value())
                    .unwrap_or_else(|| ptr_type.const_null());

                // Set status = 500
                if let Ok(status_ptr) = ctx.builder.build_struct_gep(
                    error_response_type,
                    error_response_ptr,
                    0,
                    "status_ptr",
                ) {
                    let _ = ctx
                        .builder
                        .build_store(status_ptr, i32_type.const_int(500, false));
                }
                // Set body
                if let Ok(body_ptr) = ctx.builder.build_struct_gep(
                    error_response_type,
                    error_response_ptr,
                    1,
                    "body_ptr",
                ) {
                    let _ = ctx.builder.build_store(body_ptr, error_json_str);
                }
                // Set content_type
                let json_content_type = ctx.const_string("application/json");
                if let Ok(ct_ptr) = ctx.builder.build_struct_gep(
                    error_response_type,
                    error_response_ptr,
                    2,
                    "ct_ptr",
                ) {
                    let _ = ctx.builder.build_store(ct_ptr, json_content_type);
                }

                // Build DooResult for error: { tag=1, value=error_response, owner=1 }
                if let Ok(tag_ptr) = ctx.builder.build_struct_gep(
                    result_struct_type,
                    doo_result_ptr,
                    0,
                    "error_tag_ptr",
                ) {
                    let _ = ctx
                        .builder
                        .build_store(tag_ptr, i64_type.const_int(1, false));
                }
                if let Ok(value_ptr) = ctx.builder.build_struct_gep(
                    result_struct_type,
                    doo_result_ptr,
                    1,
                    "error_value_ptr",
                ) {
                    let _ = ctx.builder.build_store(value_ptr, error_response_ptr);
                }
                if let Ok(owner_ptr) = ctx.builder.build_struct_gep(
                    result_struct_type,
                    doo_result_ptr,
                    2,
                    "error_owner_ptr",
                ) {
                    let _ = ctx
                        .builder
                        .build_store(owner_ptr, i32_type.const_int(1, false));
                }
                let _ = ctx.builder.build_return(Some(&doo_result_ptr));

                // Restore position
                if let Some(block) = current_block {
                    ctx.builder.position_at_end(block);
                }

                return wrapper_fn;
            }
        }

        // Fallback if we couldn't extract struct - return null result
        if let Ok(tag_ptr) =
            ctx.builder
                .build_struct_gep(result_struct_type, doo_result_ptr, 0, "tag_ptr")
        {
            let _ = ctx
                .builder
                .build_store(tag_ptr, i64_type.const_int(1, false));
        }
        if let Ok(value_ptr) =
            ctx.builder
                .build_struct_gep(result_struct_type, doo_result_ptr, 1, "value_ptr")
        {
            let _ = ctx.builder.build_store(value_ptr, ptr_type.const_null());
        }
        if let Ok(owner_ptr) =
            ctx.builder
                .build_struct_gep(result_struct_type, doo_result_ptr, 2, "owner_ptr")
        {
            let _ = ctx
                .builder
                .build_store(owner_ptr, i32_type.const_int(1, false));
        }
        let _ = ctx.builder.build_return(Some(&doo_result_ptr));

        // Restore position
        if let Some(block) = current_block {
            ctx.builder.position_at_end(block);
        }

        return wrapper_fn;
    }

    // If user returns a struct, serialize it to JSON
    let final_result = if returns_struct {
        if let Some(val) = user_result {
            if val.is_pointer_value() {
                let struct_ptr = val.into_pointer_value();

                // Get or declare doohttp_serialize_struct_to_json
                let serialize_fn = ctx
                    .module
                    .get_function(http_pkg::DOOHTTP_SERIALIZE_STRUCT_TO_JSON)
                    .unwrap_or_else(|| {
                        let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                        ctx.module.add_function(
                            http_pkg::DOOHTTP_SERIALIZE_STRUCT_TO_JSON,
                            fn_type,
                            None,
                        )
                    });

                // Create handler name string
                let handler_name_str = ctx
                    .builder
                    .build_global_string_ptr(user_func_name, "handler_name_for_serialize")
                    .map(|g| g.as_pointer_value())
                    .unwrap_or_else(|_| ptr_type.const_null());

                // Call serialization function
                let json_ptr = ctx
                    .builder
                    .build_call(
                        serialize_fn,
                        &[struct_ptr.into(), handler_name_str.into()],
                        "serialized_json",
                    )
                    .ok()
                    .and_then(|cs| cs.try_as_basic_value().basic())
                    .map(|v| v.into_pointer_value())
                    .unwrap_or_else(|| ptr_type.const_null());

                Some(json_ptr.into())
            } else {
                user_result
            }
        } else {
            user_result
        }
    } else {
        user_result
    };

    // Build success result
    let result_size = i64_type.const_int(
        result_struct_type
            .size_of()
            .unwrap()
            .get_zero_extended_constant()
            .unwrap_or(24),
        false,
    );
    let result_ptr = ctx
        .builder
        .build_call(malloc_fn, &[result_size.into()], "result_alloc")
        .ok()
        .and_then(|cs| cs.try_as_basic_value().basic())
        .map(|v| v.into_pointer_value())
        .unwrap_or_else(|| ptr_type.const_null());

    // Set tag = 0 (Ok)
    let tag_ptr = ctx
        .builder
        .build_struct_gep(result_struct_type, result_ptr, 0, "tag_ptr")
        .ok();
    if let Some(tag_ptr) = tag_ptr {
        let _ = ctx
            .builder
            .build_store(tag_ptr, ctx.i64_type().const_int(0, false));
    }

    // Set value = user result (as pointer)
    let value_ptr = ctx
        .builder
        .build_struct_gep(result_struct_type, result_ptr, 1, "value_ptr")
        .ok();
    if let Some(value_ptr) = value_ptr {
        let result_as_ptr = match final_result {
            Some(val) if val.is_pointer_value() => val.into_pointer_value(),
            Some(val) if val.is_int_value() => {
                // Convert int to pointer — with safety check for invalid addresses.
                let int_val = val.into_int_value();
                let is_positive = ctx.builder.build_int_compare(
                    inkwell::IntPredicate::SGT,
                    int_val,
                    int_val.get_type().const_zero(),
                    "is_valid_addr",
                );
                if let Ok(is_valid) = is_positive {
                    if let Ok(as_ptr) =
                        ctx.builder
                            .build_int_to_ptr(int_val, ptr_type, "int_to_ptr")
                    {
                        let null_ptr = ptr_type.const_null();
                        ctx.builder
                            .build_select(is_valid, as_ptr, null_ptr, "safe_int_to_ptr")
                            .ok()
                            .map(|v| v.into_pointer_value())
                            .unwrap_or_else(|| ptr_type.const_null())
                    } else {
                        ptr_type.const_null()
                    }
                } else {
                    ctx.builder
                        .build_int_to_ptr(int_val, ptr_type, "int_to_ptr_fallback")
                        .unwrap_or_else(|_| ptr_type.const_null())
                }
            }
            _ => ptr_type.const_null(),
        };
        let _ = ctx.builder.build_store(value_ptr, result_as_ptr);
    }

    // Set owner = 1 (FFI owns)
    let owner_ptr = ctx
        .builder
        .build_struct_gep(result_struct_type, result_ptr, 2, "owner_ptr")
        .ok();
    if let Some(owner_ptr) = owner_ptr {
        let _ = ctx
            .builder
            .build_store(owner_ptr, ctx.i32_type().const_int(1, false));
    }

    // Return the result pointer
    let _ = ctx.builder.build_return(Some(&result_ptr));

    // Restore position
    if let Some(block) = current_block {
        ctx.builder.position_at_end(block);
    }

    wrapper_fn
}

/// Create a dummy wrapper that returns null (for error cases)
fn create_dummy_wrapper<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    wrapper_name: &str,
) -> FunctionValue<'ctx> {
    let ptr_type = ctx.ptr_type();
    let wrapper_fn_type = ptr_type.fn_type(&[ptr_type.into()], false);
    let wrapper_fn = ctx.module.add_function(wrapper_name, wrapper_fn_type, None);

    let entry = ctx.context.append_basic_block(wrapper_fn, "entry");
    ctx.builder.position_at_end(entry);
    let _ = ctx.builder.build_return(Some(&ptr_type.const_null()));

    wrapper_fn
}
