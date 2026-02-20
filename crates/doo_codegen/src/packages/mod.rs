//! Package-aware codegen dispatch system.
//!
//! This module provides the dispatch layer between generic codegen (`call_ffi.rs`)
//! and package-specific codegen (http, websocket, database, etc.).
//!
//! # Architecture
//!
//! The compiler core is **completely package-agnostic**. All package-specific
//! behavior (handler wrappers, middleware, metadata, type conversions) is
//! isolated in this module and dispatched based on the FFI library name.
//!
//! # Adding a New Package
//!
//! 1. Create a new sub-module (e.g., `redis.rs`)
//! 2. Implement `wrap_func_ref`, `pre_call`, and/or `convert_arg` functions
//! 3. Add a match arm in the dispatch functions below
//! 4. **No changes needed** in `call_ffi.rs`, `call_wrappers.rs`, or generic codegen!
//!
//! # Library Name Resolution
//!
//! Library names come from `@extern` declarations:
//! ```doo
//! @extern("doo_http", "server_new")
//! ```
//! maps symbol `doo_http_server_new` → library `doo_http`.
//!
//! For symbols without an `@extern` (C stdlib, runtime), the library is inferred
//! from the `doo_{library}_{function}` naming convention via `infer_library()`.

pub(crate) mod database;
pub(crate) mod http;
pub(crate) mod websocket;

use crate::context::CodegenContext;
use doo_mir::MirOperand;
use inkwell::values::FunctionValue;

/// Infer the package library name from an FFI symbol using naming convention.
///
/// Convention: `doo_{library}_{function}` → library = `doo_{library}`
///
/// This is a fallback for symbols not in the `ffi_library_map`.
/// Returns empty string for C stdlib and non-Doo symbols.
pub fn infer_library(symbol: &str) -> &str {
    if let Some(rest) = symbol.strip_prefix("doo_") {
        if let Some(pos) = rest.find('_') {
            // Return "doo_{library}" — everything up to the second underscore
            return &symbol[..4 + pos];
        }
    }
    ""
}

/// Resolve the library for an FFI symbol.
///
/// Priority 1: `ffi_library_map` (from `@extern` declarations — authoritative)
/// Priority 2: Infer from `doo_{library}_{function}` naming convention (fallback)
pub fn resolve_library<'a>(ctx: &'a CodegenContext, symbol: &str) -> String {
    if let Some(lib) = ctx.get_ffi_library(symbol) {
        return lib.to_string();
    }
    infer_library(symbol).to_string()
}

/// Wrap a FuncRef argument for the appropriate package.
///
/// Returns `Some(wrapper_fn)` if the package has special wrapper generation
/// (e.g., HTTP handler wrappers with request parsing, WS event wrappers).
/// Returns `None` for unknown packages — caller should use generic passthrough.
pub fn wrap_func_ref<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    library: &str,
    symbol: &str,
    func_name: &str,
    args: &[MirOperand],
) -> Option<FunctionValue<'ctx>> {
    match library {
        "doo_http" => Some(http::wrap_func_ref(ctx, symbol, func_name, args)),
        "doo_ws" => Some(websocket::wrap_func_ref(ctx, symbol, func_name)),
        _ => None, // Generic passthrough (raw function pointer)
    }
}

/// Execute pre-call hooks for the appropriate package.
///
/// Called before building FFI call arguments. Handles:
/// - HTTP: auth/crud metadata registration, middleware registration
/// - Other packages: no-op (third-party packages handle setup in their FFI Rust code)
pub fn pre_call<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    library: &str,
    symbol: &str,
    args: &[MirOperand],
) {
    match library {
        "doo_http" => http::pre_call(ctx, symbol, args),
        _ => {} // No pre-call hooks for other packages
    }
}

/// Check if an argument needs package-specific conversion.
///
/// Returns `Some(converted_value)` if the package handles this arg conversion
/// (e.g., DB enum→JSON serialization). Returns `None` for normal conversion.
pub fn convert_arg<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    library: &str,
    symbol: &str,
    arg_index: usize,
    operand: &MirOperand,
) -> Option<inkwell::values::BasicMetadataValueEnum<'ctx>> {
    match library {
        "doo_db" => database::convert_arg(ctx, symbol, arg_index, operand),
        _ => None, // Normal conversion
    }
}
