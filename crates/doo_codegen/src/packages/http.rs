//! HTTP package codegen hooks.
//!
//! All HTTP-specific codegen behavior is isolated here:
//! - Handler wrapper generation (request parsing, response building)
//! - Middleware registration (user-defined middleware function pointer setup)
//! - Metadata registration (struct/enum schema for validation)
//! - Middleware constants and detection (JWT, CORS, RateLimit)
//!
//! This module delegates to `call_wrappers` and `call_metadata` for the
//! actual LLVM IR generation. The logic here is purely about WHEN to invoke
//! those generators, based on HTTP-specific symbol patterns.

use crate::context::CodegenContext;
use crate::instructions::calls::call_metadata;
use crate::instructions::calls::call_wrappers;
use doo_core::doo_debug;
use doo_mir::{MirConst, MirOperand};
use inkwell::values::FunctionValue;

// ============================================================================
// HTTP FFI Symbol Constants (Package-Owned)
// ============================================================================
// These constants define the FFI symbol names for the HTTP package.
// They live here (not in doo_core) because the compiler core should not
// know about HTTP-specific symbols. Only the HTTP package codegen uses them.

pub(crate) const DOO_HTTP_GET_SERVER_INSTANCE: &str = "doo_http_get_server_instance";
pub(crate) const DOO_HTTP_AUTH: &str = "doo_http_auth";
pub(crate) const DOO_HTTP_AUTH_WITH_WEBHOOKS: &str = "doo_http_auth_with_webhooks";
pub(crate) const DOO_HTTP_CRUD: &str = "doo_http_crud";
pub(crate) const DOO_HTTP_CRUD_WITH_WEBHOOKS: &str = "doo_http_crud_with_webhooks";
pub(crate) const DOO_HTTP_REGISTER_ROUTE_WEBHOOK: &str = "doo_http_register_route_webhook";
pub(crate) const DOO_HTTP_REGISTER_MIDDLEWARE: &str = "doo_http_register_middleware";
pub(crate) const DOO_HTTP_REGISTER_HANDLER_WITH_METADATA: &str =
    "doo_http_register_handler_with_metadata";
pub(crate) const DOO_HTTP_REGISTER_STRUCT_METADATA: &str = "doo_http_register_struct_metadata";
pub(crate) const DOO_HTTP_REGISTER_ENUM_METADATA: &str = "doo_http_register_enum_metadata";
pub(crate) const DOO_HTTP_REGISTER_POLICY: &str = "doo_http_register_policy";

// HTTP Serialization Helpers (doohttp_ prefix — compiled into codegen wrappers)
pub(crate) const DOOHTTP_POPULATE_STRUCT_FROM_REQUEST: &str =
    "doohttp_populate_struct_from_request";
pub(crate) const DOOHTTP_LAST_ERROR_STATUS: &str = "doohttp_last_error_status";
pub(crate) const DOOHTTP_LAST_ERROR_JSON: &str = "doohttp_last_error_json";
pub(crate) const DOOHTTP_ERROR_VARIANT_TO_STATUS: &str = "doohttp_error_variant_to_status";
pub(crate) const DOOHTTP_BUILD_RFC7807_ERROR: &str = "doohttp_build_rfc7807_error";
pub(crate) const DOOHTTP_SERIALIZE_STRUCT_TO_JSON: &str = "doohttp_serialize_struct_to_json";
pub(crate) const DOOHTTP_FORMAT_ERROR_AS_JSON: &str = "doohttp_format_error_as_json";

// ============================================================================
// HTTP Middleware Constants
// ============================================================================
// These are HTTP-package-specific. They define which middleware names the
// HTTP runtime handles natively (no compiler wrapper generation needed).
// JWT is included because the HTTP runtime has a built-in JWT handler.
// CORS and RateLimit are similarly built-in HTTP features.
//
// Third-party auth packages would NOT be listed here — they'd register
// their middleware functions normally through the wrapper generation path.

/// JWT middleware identifier — matches Doo's `Jwt()` function
const MIDDLEWARE_JWT: &str = "Jwt";

/// CORS middleware identifier
const MIDDLEWARE_CORS: &str = "cors";

/// Rate limit middleware identifier
const MIDDLEWARE_RATELIMIT: &str = "ratelimit";

/// Logger middleware identifier
const MIDDLEWARE_LOGGER: &str = "logger";

/// All built-in HTTP middleware names that the runtime handles natively.
/// The compiler skips wrapper generation for these — they don't need
/// user-defined function pointers.
const BUILTIN_MIDDLEWARES: &[&str] = &[
    MIDDLEWARE_JWT,
    MIDDLEWARE_CORS,
    MIDDLEWARE_RATELIMIT,
    MIDDLEWARE_LOGGER,
];

/// Check if a middleware name is handled natively by the HTTP runtime.
#[inline]
pub(crate) fn is_builtin_middleware(name: &str) -> bool {
    BUILTIN_MIDDLEWARES.contains(&name)
}

/// Check if a middleware name is the JWT auth middleware.
/// Used by RouteContext to enable user_id injection in handler wrappers.
#[inline]
pub(crate) fn is_auth_middleware(name: &str) -> bool {
    name == MIDDLEWARE_JWT
}

/// Handle FuncRef wrapping for HTTP package symbols.
///
/// Generates handler wrappers with proper HTTP context (route params, JWT, etc.).
/// Called when an FFI function from the `doo_http` library receives a FuncRef arg.
pub(crate) fn wrap_func_ref<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    symbol: &str,
    func_name: &str,
    args: &[MirOperand],
) -> FunctionValue<'ctx> {
    let route_context = call_wrappers::extract_route_context(symbol, args);

    let wrapper = call_wrappers::get_or_generate_handler_wrapper_with_context(
        ctx,
        func_name,
        symbol,
        &route_context,
    );

    // Register handler metadata for route registrations
    // (enables runtime request validation against Doo struct schemas)
    let is_route_registration = symbol.ends_with("_fn") || symbol.ends_with("_with_middleware");
    if is_route_registration {
        call_metadata::emit_handler_metadata_registration(ctx, func_name, &wrapper);
    }

    wrapper
}

/// Pre-call hooks for HTTP package.
///
/// Handles:
/// 1. Auth/CRUD struct metadata registration — emits struct/enum schemas
///    so the HTTP runtime can validate incoming request data.
/// 2. Policy metadata registration — emits RBAC policies bound to structs.
/// 3. Middleware registration — registers user-defined middleware function
///    pointers with the HTTP runtime.
pub(crate) fn pre_call<'ctx>(ctx: &mut CodegenContext<'ctx>, symbol: &str, args: &[MirOperand]) {
    // Struct metadata for auth/crud endpoints (including webhook variants)
    if symbol == DOO_HTTP_AUTH || symbol == DOO_HTTP_AUTH_WITH_WEBHOOKS
        || symbol == DOO_HTTP_CRUD || symbol == DOO_HTTP_CRUD_WITH_WEBHOOKS
    {
        call_metadata::emit_struct_metadata_registration_for_auth_crud(ctx, symbol, args);
        // Emit RBAC policy metadata if a policy is registered for this struct
        call_metadata::emit_policy_metadata_if_present(ctx, args);
    }

    // Middleware registration for *_with_middleware routes
    if symbol.ends_with("_with_middleware") && args.len() >= 4 {
        register_user_middleware(ctx, symbol, args);
    }
}

/// Register user-defined middleware functions with the HTTP runtime.
///
/// When a route uses `*_with_middleware`, the middleware names string is parsed
/// and each user-defined middleware gets a wrapper function registered.
/// Built-in middlewares (CORS, RateLimit) are skipped — they have native
/// handlers in the runtime.
fn register_user_middleware<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    symbol: &str,
    args: &[MirOperand],
) {
    let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();

    // arg[2] is the middleware names string (e.g., "AuthMiddleware,AdminMiddleware")
    if let MirOperand::Const(MirConst::Str(middleware_str)) = &args[2] {
        for mw_name in middleware_str.split(',').map(|s| s.trim()) {
            if mw_name.is_empty() {
                continue;
            }

            // Skip built-in middlewares — they register themselves in the runtime
            if is_builtin_middleware(mw_name) {
                if debug {
                }
                continue;
            }

            // Generate wrapper for user-defined middleware and register it
            let wrapper = call_wrappers::get_or_generate_handler_wrapper(ctx, mw_name, symbol);

            // Call doo_http_register_middleware(name, fn_ptr)
            let register_fn = ctx
                .module
                .get_function(DOO_HTTP_REGISTER_MIDDLEWARE)
                .unwrap_or_else(|| {
                    let ptr_type = ctx.ptr_type();
                    let fn_type = ctx
                        .context
                        .void_type()
                        .fn_type(&[ptr_type.into(), ptr_type.into()], false);
                    ctx.module
                        .add_function(DOO_HTTP_REGISTER_MIDDLEWARE, fn_type, None)
                });

            let mw_name_str = ctx.const_string(mw_name);
            let _ = ctx.builder.build_call(
                register_fn,
                &[
                    mw_name_str.into(),
                    wrapper.as_global_value().as_pointer_value().into(),
                ],
                "register_mw",
            );

            if debug {
            }
        }
    }
}
