//! Builtin Methods Dispatcher - Uses Central MethodRegistry
//!
//! ## Single Source of Truth
//!
//! All method definitions are in `doo_core::methods` - this module only
//! **implements** the codegen for those methods. No method names hardcoded here.
//!
//! ## Architecture
//!
//! - `doo_core::methods` - Defines what methods exist (SINGLE SOURCE)
//! - This module - Generates LLVM IR for method implementations
//! - `collections.rs` - Handles MIR data instructions (ArrayCreate, MapGet, etc.)

mod array;
mod json;
mod map;
mod string;

use doo_core::constants::ffi_names;

pub use array::ArrayBuiltins;
pub use json::JsonBuiltins;
pub use map::MapBuiltins;
pub use string::StringBuiltins;

use crate::context::CodegenContext;
use doo_core::types::TypeKind;
use inkwell::values::BasicValueEnum;

/// Dispatch a method call based on receiver type.
/// Uses TypeRegistry to determine type kind for proper routing.
pub fn dispatch_method<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: Option<&str>,
    receiver: BasicValueEnum<'ctx>,
    receiver_type: doo_core::types::TypeId,
    method: &str,
    args: &[BasicValueEnum<'ctx>],
) -> Option<BasicValueEnum<'ctx>> {
    // Get type kind from central TypeRegistry
    let type_kind = ctx.get_type_kind(receiver_type)?;

    // Route to appropriate handler based on type kind
    match type_kind {
        TypeKind::Str => {
            if receiver.is_pointer_value() {
                StringBuiltins::dispatch(ctx, dest, receiver.into_pointer_value(), method, args)
            } else {
                None
            }
        }
        TypeKind::Array { .. } => {
            if receiver.is_pointer_value() {
                ArrayBuiltins::dispatch(
                    ctx,
                    dest,
                    None,
                    receiver_type,
                    receiver.into_pointer_value(),
                    method,
                    args,
                )
            } else {
                None
            }
        }
        TypeKind::Map { .. } => {
            if receiver.is_pointer_value() {
                MapBuiltins::dispatch(
                    ctx,
                    dest,
                    None,
                    receiver_type,
                    receiver.into_pointer_value(),
                    method,
                    args,
                )
            } else {
                None
            }
        }
        TypeKind::Int => {
            // Int methods like toChar
            emit_int_method(ctx, dest, receiver, method)
        }
        _ => None,
    }
}

/// Int builtin methods
fn emit_int_method<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: Option<&str>,
    receiver: BasicValueEnum<'ctx>,
    method: &str,
) -> Option<BasicValueEnum<'ctx>> {
    let result = match method {
        "toChar" => {
            if !receiver.is_int_value() {
                return None;
            }
            let char_code = receiver.into_int_value();

            // Allocate 2 bytes for single char + null
            let malloc = ctx.module.get_function(ffi_names::MALLOC)?;
            let size = ctx.context.i64_type().const_int(2, false);
            let ptr = ctx
                .builder
                .build_call(malloc, &[size.into()], "char_str")
                .ok()?
                .try_as_basic_value()
                .basic()?
                .into_pointer_value();

            // Truncate to i8 and store
            let char_i8 = ctx
                .builder
                .build_int_truncate(char_code, ctx.context.i8_type(), "char")
                .ok()?;
            ctx.builder.build_store(ptr, char_i8).ok()?;

            // Null terminate
            let null_ptr = unsafe {
                ctx.builder
                    .build_gep(
                        ctx.context.i8_type(),
                        ptr,
                        &[ctx.context.i64_type().const_int(1, false)],
                        "null_ptr",
                    )
                    .ok()?
            };
            ctx.builder
                .build_store(null_ptr, ctx.context.i8_type().const_int(0, false))
                .ok()?;

            Some(ptr.into())
        }
        _ => None,
    };

    if let (Some(val), Some(dest_name)) = (result, dest) {
        ctx.set_temp(dest_name, val);
    }

    result
}

/// Black box builtin - prevents LLVM from optimizing away the value
/// Implemented using inline assembly with side effects
pub fn emit_black_box<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    value: BasicValueEnum<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    if let BasicValueEnum::IntValue(int_val) = value {
        // Create inline assembly with empty body but side effects
        let asm_type = int_val
            .get_type()
            .fn_type(&[int_val.get_type().into()], false);
        let asm = ctx.context.create_inline_asm(
            asm_type,
            "".to_string(),     // empty asm
            "=r,0".to_string(), // output = input
            true,               // has side effects
            false,              // no alignment
            None,
            false,
        );

        // Use build_indirect_call since create_inline_asm returns PointerValue
        let result = ctx
            .builder
            .build_indirect_call(asm_type, asm, &[int_val.into()], "blackbox")
            .ok()?
            .try_as_basic_value()
            .basic()?;
        Some(result)
    } else {
        Some(value)
    }
}
