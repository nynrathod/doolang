//! Call Utilities — shared helpers for call instruction codegen.
//!
//! Contains operand conversion, type coercion, and result struct loading.

use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use doo_mir::sym::resolve;
use doo_mir::{MirConst, MirOperand};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, PointerValue};
/// Convert MirOperand to LLVM value.
pub(crate) fn operand_to_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    operand: &MirOperand,
) -> Option<BasicValueEnum<'ctx>> {
    match operand {
        MirOperand::Const(c) => Some(const_to_value(ctx, c)),
        MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
            let name_str = resolve(*name);
            // Check if this is a static global — directly read from OnceLock global
            if let Some(_) = ctx.static_globals.get(&name_str) {
                if let Some(once_lock) = ctx.module.get_global(&name_str) {
                    let once_lock_ptr = once_lock.as_pointer_value();
                    let once_lock_type = ctx.context.struct_type(
                        &[
                            ctx.context.bool_type().into(),
                            ctx.context.ptr_type(inkwell::AddressSpace::default()).into(),
                        ],
                        false,
                    );
                    let ptr_field = ctx.builder.build_struct_gep(
                        once_lock_type,
                        once_lock_ptr,
                        1,
                        "static_get_ptr",
                    );
                    if let Ok(ptr_field) = ptr_field {
                        let loaded = ctx.builder.build_load(
                            ctx.context.ptr_type(inkwell::AddressSpace::default()),
                            ptr_field,
                            "static_val",
                        ).ok();
                        if let Some(loaded_val) = loaded {
                            return Some(loaded_val);
                        }
                    }
                }
                return Some(
                    ctx.context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                );
            }
            // Check if this is a type name — return null ptr for static method calls
            if ctx.type_registry.lookup(&name_str).is_some() {
                return Some(
                    ctx.context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                );
            }
            // First try to get as a value (local variable, temp, etc.)
            if let Some(val) = ctx.get_value(&name_str) {
                return Some(val);
            }
            // Fall back to function reference - convert function to pointer value
            if let Some(func) = ctx.get_function(&name_str) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
        MirOperand::FuncRef(name) => {
            let name_str = resolve(*name);
            // Explicit function reference - return function as pointer value
            // Used when passing functions to FFI (e.g., app.get("/users", getUserHandler))
            if let Some(func) = ctx.get_function(&name_str) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
    }
}

/// Coerce an argument value to match the expected function parameter type.
///
/// This handles type mismatches between how values are produced (e.g., enum StructValues)
/// and how function parameters are declared (e.g., pointers for composite types).
pub(crate) fn coerce_arg_to_param_type<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    expected_type: Option<BasicTypeEnum<'ctx>>,
) -> inkwell::values::BasicMetadataValueEnum<'ctx> {
    // If no expected type info, pass value as-is
    let Some(expected) = expected_type else {
        return val.into();
    };

    // If types already match, pass as-is
    if val.get_type() == expected {
        return val.into();
    }

    // Special case: StructValue passed where pointer is expected
    // This happens with enums: EnumCreate returns { i32, ptr } but function params expect ptr
    if val.is_struct_value() && expected.is_pointer_type() {
        // Box the struct value: allocate, store, return pointer
        let alloca = ctx
            .alloca_in_entry_block(val.get_type(), "arg_box")
            .unwrap();
        ctx.builder.build_store(alloca, val).ok();
        return alloca.into();
    }

    // Special case: PointerValue passed where struct is expected
    // This happens when JSON.parse returns a pointer to enum but function expects struct by value
    // OR when a concrete struct is passed where an interface fat pointer {ptr, ptr} is expected.
    if val.is_pointer_value() && expected.is_struct_type() {
        let struct_type = expected.into_struct_type();
        let field_types = struct_type.get_field_types();

        // Check if this is an interface fat pointer: { i8*, i8* }
        // Interface fat pointers have exactly 2 pointer fields.
        let is_interface_fat_ptr = field_types.len() == 2
            && field_types[0].is_pointer_type()
            && field_types[1].is_pointer_type();

        if is_interface_fat_ptr {
            // Build interface fat pointer from concrete struct pointer.
            // { data_ptr, vtable_ptr } where vtable_ptr = null for now.
            let data_ptr = val.into_pointer_value();
            let i8_ptr_type = ctx.context.i8_type().ptr_type(inkwell::AddressSpace::default());
            let data_i8_ptr = ctx.builder
                .build_pointer_cast(data_ptr, i8_ptr_type, "iface_data")
                .ok()
                .unwrap_or_else(|| i8_ptr_type.const_null());
            let vtable_ptr = i8_ptr_type.const_null();

            let mut fat_ptr = struct_type.get_undef();
            if let Ok(s) = ctx.builder.build_insert_value(fat_ptr, data_i8_ptr, 0, "iface_data_field") {
                fat_ptr = s.into_struct_value();
            }
            if let Ok(s) = ctx.builder.build_insert_value(fat_ptr, vtable_ptr, 1, "iface_vtable_field") {
                fat_ptr = s.into_struct_value();
            }
            return fat_ptr.into();
        }

        // Normal case: load struct from pointer
        let loaded = ctx
            .builder
            .build_load(expected, val.into_pointer_value(), "arg_load")
            .ok();
        if let Some(v) = loaded {
            return v.into();
        }
    }

    // CRITICAL FIX: IntValue passed where pointer is expected
    // This can happen when:
    // 1. Struct type info is lost during tuple extraction (TupleGet fallback)
    // 2. Field load defaults to i64 when struct type lookup fails
    // The value is actually a pointer stored as i64, convert it back.
    // SAFETY: Validate the integer is a plausible user-space address before inttoptr.
    // Zero and negative values (like -1/0xFFFFFFFFFFFFFFFF) are NOT valid addresses.
    if val.is_int_value() && expected.is_pointer_type() {
        let int_val = val.into_int_value();
        let ptr_type = expected.into_pointer_type();
        let is_positive = ctx.builder.build_int_compare(
            inkwell::IntPredicate::SGT,
            int_val,
            int_val.get_type().const_zero(),
            "is_valid_addr",
        );
        if let Ok(is_valid) = is_positive {
            let null_ptr = ptr_type.const_null();
            if let Ok(as_ptr) = ctx
                .builder
                .build_int_to_ptr(int_val, ptr_type, "int_to_ptr_coerce")
            {
                if let Ok(result) =
                    ctx.builder
                        .build_select(is_valid, as_ptr, null_ptr, "safe_coerce")
                {
                    return result.into();
                }
            }
        }
        // Fallback: raw inttoptr
        if let Ok(ptr) =
            ctx.builder
                .build_int_to_ptr(int_val, ptr_type, "int_to_ptr_coerce_fallback")
        {
            return ptr.into();
        }
    }

    // Default: pass value as-is
    val.into()
}

/// Convert MirConst to LLVM value.
pub(crate) fn const_to_value<'ctx>(
    ctx: &CodegenContext<'ctx>,
    c: &MirConst,
) -> BasicValueEnum<'ctx> {
    match c {
        MirConst::Int(v) => ctx.const_i64(*v).into(),
        MirConst::Float(v) => ctx.const_f64(*v).into(),
        MirConst::Bool(v) => ctx.const_bool(*v).into(),
        MirConst::Nil => ctx.const_i64(0).into(),
        MirConst::Str(s) => ctx.const_string(s).into(),
    }
}

// ============================================================================
// Result/Error Handling Helpers
// ============================================================================

/// Convert a value to a pointer representation for storing in Result payload.
/// - Pointers: pass through as-is
/// - Integers: heap-box (malloc+store) — NEVER use inttoptr (causes crashes
///   when the integer value is later treated as a real pointer by strlen/free)
/// - Floats: heap-box (malloc+store)
/// - Structs: heap-allocate and store
pub(crate) fn value_to_ptr<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
) -> Option<PointerValue<'ctx>> {
    if val.is_pointer_value() {
        // Already a pointer (string, array, map, struct)
        Some(val.into_pointer_value())
    } else if val.is_int_value() {
        // CRITICAL: Box the integer on the heap — do NOT use inttoptr.
        // inttoptr turns the integer VALUE into a pointer ADDRESS, so e.g.
        // a database ID of 1 becomes pointer 0x1. Any subsequent strlen/free
        // on that pointer causes an access violation crash.
        let int_val = val.into_int_value();
        let int_64 = if int_val.get_type().get_bit_width() == 64 {
            int_val
        } else {
            ctx.builder
                .build_int_z_extend(int_val, ctx.i64_type(), "ext")
                .ok()?
        };
        let malloc_fn = ctx
            .module
            .get_function(ffi_names::MALLOC)
            .unwrap_or_else(|| {
                let fn_ty = ctx.ptr_type().fn_type(&[ctx.i64_type().into()], false);
                ctx.module.add_function(ffi_names::MALLOC, fn_ty, None)
            });
        let heap_ptr = ctx
            .builder
            .build_call(
                malloc_fn,
                &[ctx.i64_type().const_int(8, false).into()],
                "int_box",
            )
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();
        ctx.builder.build_store(heap_ptr, int_64).ok()?;
        Some(heap_ptr)
    } else if val.is_float_value() {
        // CRITICAL: Box the float on the heap — same reasoning as integers.
        let float_val = val.into_float_value();
        let malloc_fn = ctx
            .module
            .get_function(ffi_names::MALLOC)
            .unwrap_or_else(|| {
                let fn_ty = ctx.ptr_type().fn_type(&[ctx.i64_type().into()], false);
                ctx.module.add_function(ffi_names::MALLOC, fn_ty, None)
            });
        let heap_ptr = ctx
            .builder
            .build_call(
                malloc_fn,
                &[ctx.i64_type().const_int(8, false).into()],
                "float_box",
            )
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();
        ctx.builder.build_store(heap_ptr, float_val).ok()?;
        Some(heap_ptr)
    } else if val.is_struct_value() {
        // Heap-allocate struct and return pointer
        let struct_val = val.into_struct_value();
        let struct_type = struct_val.get_type();
        let heap_ptr = ctx.builder.build_malloc(struct_type, "struct_heap").ok()?;
        ctx.builder.build_store(heap_ptr, struct_val).ok()?;
        Some(heap_ptr)
    } else {
        // Fallback: use null pointer
        Some(ctx.ptr_type().const_null())
    }
}

/// Load a Result struct from a value that may be a pointer or struct.
pub(crate) fn load_result_struct<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    result_val: BasicValueEnum<'ctx>,
) -> Option<inkwell::values::StructValue<'ctx>> {
    // Result struct layout: { i64 tag, ptr value }
    // Using ptr for payload preserves pointer provenance through LLVM optimizations
    let result_struct_type = ctx
        .context
        .struct_type(&[ctx.i64_type().into(), ctx.ptr_type().into()], false);

    if result_val.is_pointer_value() && !result_val.is_struct_value() {
        // Load from pointer
        let result_ptr = result_val.into_pointer_value();
        ctx.builder
            .build_load(result_struct_type, result_ptr, "result_struct_load")
            .ok()?
            .try_into()
            .ok()
    } else if result_val.is_struct_value() {
        // Already a struct value
        Some(result_val.into_struct_value())
    } else {
        // Not a Result - return None
        None
    }
}
