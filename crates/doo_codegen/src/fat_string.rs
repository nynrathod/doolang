//! Fat String Infrastructure — {i8*, i64} string representation.
//!
//! Provides LLVM type definitions and helper functions for the fat string
//! representation: `%DooStr = type { i8*, i64 }` where field 0 is the data
//! pointer and field 1 is the length.
//!
//! ## Migration Strategy
//!
//! The current string representation is `i8*` (null-terminated C string).
//! This module provides the foundation for migrating to fat strings:
//!
//! 1. **Phase 1** (this module): Define type + helpers, available for new code
//! 2. **Phase 2**: Migrate string builtins (len, concat, slice, etc.)
//! 3. **Phase 3**: Migrate FFI boundary (add null-terminator on C calls)
//! 4. **Phase 4**: Update all string creation/manipulation in codegen
//!
//! ## Benefits
//!
//! - `string.len()` → O(1) instead of O(n) strlen
//! - Binary-safe (can contain null bytes)
//! - Compatible with Rust's `&str` layout for FFI

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::types::StructType;
use inkwell::values::{AnyValue, IntValue, PointerValue, StructValue};
use inkwell::AddressSpace;

/// Field index for the data pointer in the fat string struct.
pub const FAT_STR_PTR_FIELD: u32 = 0;

/// Field index for the length in the fat string struct.
pub const FAT_STR_LEN_FIELD: u32 = 1;

/// Get or create the `%DooStr = type { i8*, i64 }` struct type.
///
/// This is a named struct type that's cached in the LLVM context, so calling
/// this multiple times returns the same type.
pub fn fat_str_type<'ctx>(context: &'ctx Context) -> StructType<'ctx> {
    let ptr_type = context.ptr_type(AddressSpace::default()).into();
    let len_type = context.i64_type().into();
    context.struct_type(&[ptr_type, len_type], false)
}

/// Build a fat string value from a data pointer and length.
///
/// Returns a `{ i8*, i64 }` struct value.
pub fn build_fat_str<'ctx>(
    builder: &Builder<'ctx>,
    context: &'ctx Context,
    ptr: PointerValue<'ctx>,
    len: IntValue<'ctx>,
) -> Option<StructValue<'ctx>> {
    let str_type = fat_str_type(context);
    let mut val = str_type.get_undef();

    val = builder
        .build_insert_value(val, ptr, FAT_STR_PTR_FIELD, "str.ptr")
        .ok()?
        .into_struct_value();
    val = builder
        .build_insert_value(val, len, FAT_STR_LEN_FIELD, "str.len")
        .ok()?
        .into_struct_value();

    Some(val)
}

/// Extract the data pointer from a fat string.
pub fn extract_ptr<'ctx>(
    builder: &Builder<'ctx>,
    str_val: StructValue<'ctx>,
) -> Option<PointerValue<'ctx>> {
    builder
        .build_extract_value(str_val, FAT_STR_PTR_FIELD, "str.ptr")
        .ok()
        .map(|v| v.into_pointer_value())
}

/// Extract the length from a fat string.
pub fn extract_len<'ctx>(
    builder: &Builder<'ctx>,
    str_val: StructValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    builder
        .build_extract_value(str_val, FAT_STR_LEN_FIELD, "str.len")
        .ok()
        .map(|v| v.into_int_value())
}

/// Build a fat string from a null-terminated C string pointer.
///
/// Calls `strlen()` to get the length, then wraps in `{ ptr, len }`.
/// Used during migration from C-string representation.
pub fn from_c_string<'ctx>(
    builder: &Builder<'ctx>,
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    c_str: PointerValue<'ctx>,
) -> Option<StructValue<'ctx>> {
    // Declare strlen if not already present
    let strlen_fn = module.get_function("strlen").unwrap_or_else(|| {
        let fn_type = context
            .i64_type()
            .fn_type(&[context.ptr_type(AddressSpace::default()).into()], false);
        module.add_function("strlen", fn_type, None)
    });

    let call_result = builder
        .build_call(strlen_fn, &[c_str.into()], "str.c_len")
        .ok()?;
    let len = call_result.as_any_value_enum().into_int_value();

    build_fat_str(builder, context, c_str, len)
}
