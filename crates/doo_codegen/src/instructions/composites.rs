//! Composite/Aggregate Instruction Handler
//!
//! Handles: TupleCreate, TupleGet, StructCreate, FieldGet, FieldSet

use super::InstructionHandler;
use crate::context::CodegenContext;
use crate::utils::operand_to_value;
use doo_core::doo_debug;
use doo_core::types::{builtin, TypeKind};
use doo_mir::{MirInstr, MirInstrKind, MirOperand};
use inkwell::values::BasicValueEnum;

/// Composite instruction handler.
pub struct CompositeHandler;

impl CompositeHandler {
    /// Extract variable name from MirOperand for struct type lookup.
    fn get_operand_name(operand: &MirOperand) -> Option<&str> {
        match operand {
            MirOperand::Local(name) | MirOperand::Temp(name) => Some(name.as_str()),
            _ => None,
        }
    }
}

impl<'ctx> InstructionHandler<'ctx> for CompositeHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(
            instr.kind,
            MirInstrKind::TupleCreate { .. }
                | MirInstrKind::TupleGet { .. }
                | MirInstrKind::StructCreate { .. }
                | MirInstrKind::FieldGet { .. }
                | MirInstrKind::FieldSet { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::TupleCreate { dest, elements } => {
                let types: Vec<_> = elements.iter().map(|_| ctx.i64_type().into()).collect();
                let tuple_type = ctx.context.struct_type(&types, false);

                // CRITICAL: Use heap allocation (malloc) instead of stack allocation (alloca).
                // When a tuple is wrapped in WrapOk and returned from a function, the stack
                // frame is deallocated before the caller reads the tuple → dangling pointer.
                // Heap allocation ensures the tuple survives the function return.
                let ptr_type = ctx.ptr_type();
                let i64_type = ctx.context.i64_type();
                let malloc_fn = ctx
                    .module
                    .get_function(doo_core::constants::ffi_names::MALLOC)
                    .unwrap_or_else(|| {
                        let fn_ty = ptr_type.fn_type(&[i64_type.into()], false);
                        ctx.module
                            .add_function(doo_core::constants::ffi_names::MALLOC, fn_ty, None)
                    });
                let tuple_size = tuple_type
                    .size_of()
                    .map(|v| v.get_zero_extended_constant().unwrap_or(64))
                    .unwrap_or((elements.len() * 8) as u64);
                let heap_ptr = ctx
                    .builder
                    .build_call(
                        malloc_fn,
                        &[i64_type.const_int(tuple_size.max(16), false).into()],
                        "tuple_alloc",
                    )
                    .ok()?
                    .try_as_basic_value()
                    .basic()?
                    .into_pointer_value();

                for (i, elem) in elements.iter().enumerate() {
                    if let Some(val) = operand_to_value(ctx, elem) {
                        if let Ok(ptr) = ctx.builder.build_struct_gep(
                            tuple_type,
                            heap_ptr,
                            i as u32,
                            "field_ptr",
                        ) {
                            ctx.builder.build_store(ptr, val).ok();
                        }
                    }
                }

                ctx.set_temp(dest, heap_ptr.into());
                Some(heap_ptr.into())
            }

            MirInstrKind::TupleGet {
                dest,
                tuple,
                index,
                tuple_type,
            } => {
                if let Some(tup) = operand_to_value(ctx, tuple) {
                    let tup: inkwell::values::BasicValueEnum = tup;
                    if tup.is_pointer_value() {
                        let ptr = tup.into_pointer_value();

                        // Build the tuple type from registry if we have the type info
                        // First, extract element TypeIds to avoid borrowing issues
                        let elem_type_ids: Option<Vec<doo_core::types::TypeId>> =
                            if let Some(type_id) = tuple_type {
                                if let Some(type_info) = ctx.type_registry.get(*type_id) {
                                    if let TypeKind::Tuple { elements } = &type_info.kind {
                                        Some(elements.clone())
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                        let (tuple_ty, elem_type) = if let Some(elem_ids) = elem_type_ids {
                            // Build LLVM types for all elements
                            let element_types: Vec<inkwell::types::BasicTypeEnum> =
                                elem_ids.iter().map(|tid| ctx.get_llvm_type(*tid)).collect();
                            let tuple_struct = ctx.context.struct_type(&element_types, false);
                            let elem_llvm = ctx.get_llvm_type(elem_ids[*index]);
                            (tuple_struct, elem_llvm)
                        } else {
                            // CRITICAL FIX: When tuple_type is not available, use the dest variable's
                            // known type from variable_types (populated from func.locals in builder.rs).
                            // This is the single source of truth for variable types.
                            // Fallback to i64 only if dest type is also unknown.
                            let dest_type_id = ctx.get_variable_type(dest);
                            let elem_llvm = match dest_type_id {
                                Some(tid) => ctx.get_llvm_type(tid),
                                None => ctx.i64_type().into(),
                            };
                            // For the tuple struct type without full type info, use ptr for each element
                            // since most complex types (structs, arrays, etc.) are passed as pointers.
                            // This is a conservative estimate that won't cause type mismatches.
                            let num_elements = (*index + 1).max(2); // At least 2 elements
                            let element_types: Vec<inkwell::types::BasicTypeEnum> = (0
                                ..num_elements)
                                .map(|i| {
                                    if i == *index {
                                        elem_llvm
                                    } else {
                                        // Other elements: use ptr as conservative default
                                        ctx.ptr_type().into()
                                    }
                                })
                                .collect();
                            let tuple_struct = ctx.context.struct_type(&element_types, false);
                            (tuple_struct, elem_llvm)
                        };

                        if let Ok(field_ptr) =
                            ctx.builder
                                .build_struct_gep(tuple_ty, ptr, *index as u32, "field")
                        {
                            if let Ok(val) = ctx.builder.build_load(elem_type, field_ptr, dest) {
                                ctx.set_temp(dest, val);

                                // CRITICAL: Propagate struct type association for nested struct access.
                                // If dest is a struct type (or TypeRef to struct), register it so
                                // FieldGet/FieldSet work correctly. Must resolve TypeRef to get actual struct name.
                                if let Some(tid) = ctx.get_variable_type(dest) {
                                    if let Some(kind) = ctx.get_type_kind(tid) {
                                        match kind {
                                            TypeKind::Struct { name, .. } => {
                                                ctx.set_temp_struct_type(dest, &name);
                                            }
                                            // CRITICAL: Resolve TypeRef to the actual struct type
                                            TypeKind::TypeRef { name: ref_name } => {
                                                if let Some(resolved_tid) =
                                                    ctx.type_registry.lookup(&ref_name)
                                                {
                                                    if let Some(TypeKind::Struct { name, .. }) =
                                                        ctx.get_type_kind(resolved_tid)
                                                    {
                                                        ctx.set_temp_struct_type(dest, &name);
                                                    }
                                                }
                                            }
                                            // For Optional<Struct> or Optional<TypeRef>, extract inner type
                                            TypeKind::Optional { inner } => {
                                                if let Some(inner_kind) = ctx.get_type_kind(inner) {
                                                    match inner_kind {
                                                        TypeKind::Struct { name, .. } => {
                                                            ctx.set_temp_struct_type(dest, &name);
                                                        }
                                                        TypeKind::TypeRef { name: ref_name } => {
                                                            if let Some(resolved_tid) =
                                                                ctx.type_registry.lookup(&ref_name)
                                                            {
                                                                if let Some(TypeKind::Struct {
                                                                    name,
                                                                    ..
                                                                }) =
                                                                    ctx.get_type_kind(resolved_tid)
                                                                {
                                                                    ctx.set_temp_struct_type(
                                                                        dest, &name,
                                                                    );
                                                                }
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }

                                return Some(val);
                            }
                        }
                    }
                }
                None
            }

            MirInstrKind::StructCreate {
                dest,
                struct_name,
                fields,
            } => {
                // For anonymous object literals (__anon), create a HashMap via FFI
                // instead of a raw struct. This ensures proper interop with FFI functions
                // that expect HashMap<String, String> (e.g., cors/ratelimit config).
                // This is the single source of truth for object-literal-to-map conversion.
                if struct_name == "__anon" {
                    let ptr_type = ctx.ptr_type();
                    let i64_type = ctx.context.i64_type();

                    // Get or declare doo_map_new, doo_map_set, doo_map_set_str_array
                    let map_new_fn = ctx.module.get_function("doo_map_new").unwrap_or_else(|| {
                        let fn_ty = ptr_type.fn_type(&[], false);
                        ctx.module.add_function("doo_map_new", fn_ty, None)
                    });
                    let map_set_fn = ctx.module.get_function("doo_map_set").unwrap_or_else(|| {
                        let void_ty = ctx.context.void_type();
                        let fn_ty = void_ty
                            .fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
                        ctx.module.add_function("doo_map_set", fn_ty, None)
                    });
                    let map_set_arr_fn = ctx
                        .module
                        .get_function("doo_map_set_str_array")
                        .unwrap_or_else(|| {
                            let void_ty = ctx.context.void_type();
                            let fn_ty = void_ty.fn_type(
                                &[ptr_type.into(), ptr_type.into(), ptr_type.into()],
                                false,
                            );
                            ctx.module
                                .add_function("doo_map_set_str_array", fn_ty, None)
                        });

                    // Get or declare sprintf for int/bool to string conversion
                    let sprintf_fn = ctx.module.get_function("sprintf").unwrap_or_else(|| {
                        let i32_type = ctx.i32_type();
                        let fn_ty = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
                        ctx.module.add_function("sprintf", fn_ty, None)
                    });

                    // Create the map
                    let map_ptr = ctx
                        .builder
                        .build_call(map_new_fn, &[], "map_new")
                        .ok()?
                        .try_as_basic_value()
                        .basic()?
                        .into_pointer_value();

                    // Insert each field into the map
                    for (field_name, value) in fields.iter() {
                        let key_str = ctx.const_string(field_name);

                        // Determine if this value is an array (tracked by array_element_types)
                        let is_array = match value {
                            MirOperand::Temp(name) => {
                                ctx.array_element_types.contains_key(name.as_str())
                            }
                            MirOperand::Local(name) => {
                                ctx.array_element_types.contains_key(name.as_str())
                            }
                            _ => false,
                        };

                        if is_array {
                            // Array value: use doo_map_set_str_array which reads the Doo array
                            // and joins elements with commas
                            if let Some(val) = operand_to_value(ctx, value) {
                                ctx.builder
                                    .build_call(
                                        map_set_arr_fn,
                                        &[map_ptr.into(), key_str.into(), val.into()],
                                        "",
                                    )
                                    .ok();
                            }
                        } else if let MirOperand::Const(doo_mir::MirConst::Bool(b)) = value {
                            // Bool value: convert to "true"/"false" string
                            let bool_str = if *b { "true" } else { "false" };
                            let val_str = ctx.const_string(bool_str);
                            ctx.builder
                                .build_call(
                                    map_set_fn,
                                    &[map_ptr.into(), key_str.into(), val_str.into()],
                                    "",
                                )
                                .ok();
                        } else if let MirOperand::Const(doo_mir::MirConst::Int(n)) = value {
                            // Int value: convert to string
                            let i8_type = ctx.context.i8_type();
                            let buffer = ctx
                                .builder
                                .build_array_alloca(
                                    i8_type,
                                    i64_type.const_int(24, false),
                                    "int_buf",
                                )
                                .unwrap();
                            let fmt = ctx.const_string("%lld");
                            ctx.builder
                                .build_call(
                                    sprintf_fn,
                                    &[
                                        buffer.into(),
                                        fmt.into(),
                                        i64_type.const_int(*n as u64, false).into(),
                                    ],
                                    "",
                                )
                                .ok();
                            ctx.builder
                                .build_call(
                                    map_set_fn,
                                    &[map_ptr.into(), key_str.into(), buffer.into()],
                                    "",
                                )
                                .ok();
                        } else if let MirOperand::Const(doo_mir::MirConst::Str(s)) = value {
                            // String constant: pass directly
                            let val_str = ctx.const_string(s);
                            ctx.builder
                                .build_call(
                                    map_set_fn,
                                    &[map_ptr.into(), key_str.into(), val_str.into()],
                                    "",
                                )
                                .ok();
                        } else {
                            // Other values (runtime temps): resolve and convert
                            if let Some(val) = operand_to_value(ctx, value) {
                                if val.is_pointer_value() {
                                    // Pointer value (could be string) — pass as string
                                    ctx.builder
                                        .build_call(
                                            map_set_fn,
                                            &[map_ptr.into(), key_str.into(), val.into()],
                                            "",
                                        )
                                        .ok();
                                } else if val.is_int_value() {
                                    // Runtime int value: sprintf to string
                                    let i8_type = ctx.context.i8_type();
                                    let buffer = ctx
                                        .builder
                                        .build_array_alloca(
                                            i8_type,
                                            i64_type.const_int(24, false),
                                            "int_buf",
                                        )
                                        .unwrap();
                                    let fmt = ctx.const_string("%lld");
                                    ctx.builder
                                        .build_call(
                                            sprintf_fn,
                                            &[buffer.into(), fmt.into(), val.into()],
                                            "",
                                        )
                                        .ok();
                                    ctx.builder
                                        .build_call(
                                            map_set_fn,
                                            &[map_ptr.into(), key_str.into(), buffer.into()],
                                            "",
                                        )
                                        .ok();
                                } else {
                                    // Float or other: convert via sprintf %g
                                    let i8_type = ctx.context.i8_type();
                                    let buffer = ctx
                                        .builder
                                        .build_array_alloca(
                                            i8_type,
                                            i64_type.const_int(32, false),
                                            "float_buf",
                                        )
                                        .unwrap();
                                    let fmt = ctx.const_string("%g");
                                    ctx.builder
                                        .build_call(
                                            sprintf_fn,
                                            &[buffer.into(), fmt.into(), val.into()],
                                            "",
                                        )
                                        .ok();
                                    ctx.builder
                                        .build_call(
                                            map_set_fn,
                                            &[map_ptr.into(), key_str.into(), buffer.into()],
                                            "",
                                        )
                                        .ok();
                                }
                            }
                        }
                    }

                    ctx.set_temp(dest, map_ptr.into());
                    return Some(map_ptr.into());
                }

                // Named struct: normal struct creation path
                // Collect field names in order for metadata
                let field_names: Vec<String> =
                    fields.iter().map(|(name, _)| name.clone()).collect();

                // Register struct metadata for field lookups
                ctx.register_struct_metadata(struct_name, field_names);

                // Build LLVM struct type with correct field types from type registry
                // Use get_llvm_type for consistent type mapping across all code paths
                let field_types: Vec<inkwell::types::BasicTypeEnum> =
                    if let Some(field_type_ids) = ctx.get_struct_field_types(struct_name) {
                        field_type_ids
                            .iter()
                            .map(|type_id| ctx.get_llvm_type(*type_id))
                            .collect()
                    } else {
                        // Fallback: use get_llvm_type based on operand types
                        fields.iter().map(|_| ctx.i64_type().into()).collect()
                    };
                let struct_type = ctx.get_struct_type(struct_name, &field_types);

                // Calculate struct size using LLVM's size_of() for correct padding
                // CRITICAL FIX: The old calculation (fields.len() * 8) was WRONG because:
                // - Enum fields are { i32, ptr } = 16 bytes (not 8)
                // - Struct padding can vary based on field alignment requirements
                let struct_size = struct_type
                    .size_of()
                    .map(|v| v.get_zero_extended_constant().unwrap_or(64))
                    .unwrap_or((fields.len() * 8) as u64); // Fallback for safety
                let i64_type = ctx.context.i64_type();
                let ptr_type = ctx.ptr_type();

                // Get or declare malloc for heap allocation
                let malloc_fn = ctx
                    .module
                    .get_function(doo_core::constants::ffi_names::MALLOC)
                    .unwrap_or_else(|| {
                        let fn_ty = ptr_type.fn_type(&[i64_type.into()], false);
                        ctx.module
                            .add_function(doo_core::constants::ffi_names::MALLOC, fn_ty, None)
                    });

                // Heap allocate the struct (so Drop can free it)
                let struct_ptr = ctx
                    .builder
                    .build_call(
                        malloc_fn,
                        &[i64_type.const_int(struct_size.max(16), false).into()], // min 16 bytes
                        dest,
                    )
                    .ok()?
                    .try_as_basic_value()
                    .basic()?
                    .into_pointer_value();

                // Get field type IDs for proper boxing
                let field_type_ids = ctx.get_struct_field_types(struct_name);

                // Store field values
                for (i, (_, value)) in fields.iter().enumerate() {
                    if let Some(val) = operand_to_value(ctx, value) {
                        if let Ok(ptr) = ctx.builder.build_struct_gep(
                            struct_type,
                            struct_ptr,
                            i as u32,
                            "field_ptr",
                        ) {
                            // Store value directly - enum struct types are now stored by value
                            ctx.builder.build_store(ptr, val).ok();
                        }
                    }
                }

                // Track that this dest variable holds a struct of this type
                ctx.set_temp_struct_type(dest, struct_name);
                ctx.set_temp(dest, struct_ptr.into());
                Some(struct_ptr.into())
            }

            MirInstrKind::FieldGet {
                dest,
                object,
                field,
            } => {
                let debug = std::env::var("DOO_DEBUG").is_ok();
                if debug {
                    let blk = ctx
                        .builder
                        .get_insert_block()
                        .map(|b| b.get_name().to_string_lossy().to_string());
                    doo_debug!("CODEGEN", "FieldGet {} in block {:?}", dest, blk);
                }
                if let Some(obj_ptr) = operand_to_value(ctx, object) {
                    if obj_ptr.is_pointer_value() {
                        let ptr = obj_ptr.into_pointer_value();

                        // Get struct name from the object operand
                        // CRITICAL FIX: Try multiple sources for struct type info:
                        // 1. temp_struct_type (runtime tracking)
                        // 2. variable_type -> TypeId -> TypeKind::Struct
                        // 3. variable_type -> TypeId -> TypeKind::TypeRef -> resolved Struct
                        let var_name = Self::get_operand_name(object);
                        let struct_name = var_name.and_then(|name| {
                            // First try temp_struct_type (most specific)
                            if let Some(st) = ctx.get_temp_struct_type(name).cloned() {
                                return Some(st);
                            }
                            // Then try variable_type -> TypeKind
                            if let Some(type_id) = ctx.get_variable_type(name) {
                                if let Some(kind) = ctx.get_type_kind(type_id) {
                                    match kind {
                                        TypeKind::Struct { name, .. } => return Some(name),
                                        TypeKind::TypeRef { name: ref_name } => {
                                            // Resolve TypeRef to the actual struct type
                                            if let Some(resolved_tid) =
                                                ctx.type_registry.lookup(&ref_name)
                                            {
                                                if let Some(TypeKind::Struct { name, .. }) =
                                                    ctx.get_type_kind(resolved_tid)
                                                {
                                                    return Some(name);
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            None
                        });

                        if debug {
                            if struct_name.is_none() {
                                doo_debug!("CODEGEN", "WARNING: FieldGet {} has no struct type for {:?} (var_name={:?})", 
                                    dest, object, var_name);
                            } else {
                                doo_debug!(
                                    "CODEGEN",
                                    "FieldGet {} using struct_name={:?} for field={}",
                                    dest,
                                    struct_name,
                                    field
                                );
                            }
                        }

                        if let Some(struct_name) = struct_name {
                            // Look up field index by name from struct metadata
                            let field_index =
                                ctx.get_field_index(&struct_name, field).unwrap_or_else(|| {
                                    // Fallback: try parsing field as numeric index
                                    field.parse::<u32>().unwrap_or(0)
                                });
                            if debug {
                                doo_debug!(
                                    "CODEGEN",
                                    "FieldGet {} field_index={} for {}.{}",
                                    dest,
                                    field_index,
                                    struct_name,
                                    field
                                );
                            }

                            // Get the struct type from cache, or build it from type registry
                            // This handles imported/cross-module types
                            if let Some(struct_type) = ctx.get_or_build_struct_type(&struct_name) {
                                if let Ok(field_ptr) = ctx.builder.build_struct_gep(
                                    struct_type,
                                    ptr,
                                    field_index,
                                    "field_ptr",
                                ) {
                                    // Determine the correct LLVM type based on field's Doo type
                                    let field_type_id =
                                        ctx.get_struct_field_type(&struct_name, field);

                                    // Check if field is a nested struct - if so, propagate struct type info
                                    // Also handle TypeRef for nested structs
                                    let nested_struct_name = field_type_id.and_then(|tid| {
                                        // Try direct struct lookup first
                                        if let Some(name) = ctx.get_struct_name_from_type_id(tid) {
                                            return Some(name);
                                        }
                                        // Try resolving TypeRef
                                        if let Some(TypeKind::TypeRef { name: ref_name }) =
                                            ctx.get_type_kind(tid)
                                        {
                                            if let Some(resolved_tid) =
                                                ctx.type_registry.lookup(&ref_name)
                                            {
                                                return ctx
                                                    .get_struct_name_from_type_id(resolved_tid);
                                            }
                                        }
                                        None
                                    });

                                    let load_type: inkwell::types::BasicTypeEnum =
                                        match field_type_id {
                                            Some(t) if t == builtin::STR => ctx.ptr_type().into(),
                                            Some(t) if t == builtin::FLOAT => ctx.f64_type().into(),
                                            Some(t) if t == builtin::BOOL => ctx.bool_type().into(),
                                            Some(t)
                                                if ctx
                                                    .get_struct_name_from_type_id(t)
                                                    .is_some() =>
                                            {
                                                // Nested struct - load as pointer
                                                ctx.ptr_type().into()
                                            }
                                            // Handle TypeRef pointing to struct
                                            Some(t)
                                                if matches!(
                                                    ctx.get_type_kind(t),
                                                    Some(TypeKind::TypeRef { .. })
                                                ) && nested_struct_name.is_some() =>
                                            {
                                                ctx.ptr_type().into()
                                            }
                                            Some(t)
                                                if matches!(
                                                    ctx.get_type_kind(t),
                                                    Some(TypeKind::Enum { .. })
                                                ) =>
                                            {
                                                // Enum field - load as enum struct type { i32, ptr }
                                                ctx.get_llvm_type(t)
                                            }
                                            // CRITICAL: Arrays and Maps are stored as pointers
                                            // This is essential for field access like self.Tasks.push()
                                            Some(t)
                                                if matches!(
                                                    ctx.get_type_kind(t),
                                                    Some(TypeKind::Array { .. })
                                                ) =>
                                            {
                                                ctx.ptr_type().into()
                                            }
                                            Some(t)
                                                if matches!(
                                                    ctx.get_type_kind(t),
                                                    Some(TypeKind::Map { .. })
                                                ) =>
                                            {
                                                ctx.ptr_type().into()
                                            }
                                            _ => ctx.i64_type().into(), // Int and default
                                        };

                                    if let Ok(val) =
                                        ctx.builder.build_load(load_type, field_ptr, dest)
                                    {
                                        ctx.set_temp(dest, val);

                                        // CRITICAL: If field is a nested struct, propagate struct type
                                        // This enables chained field access like user.address.street
                                        if let Some(nested_name) = nested_struct_name {
                                            if debug {
                                                doo_debug!(
                                                    "CODEGEN",
                                                    "FieldGet {} setting nested struct type to {}",
                                                    dest,
                                                    nested_name
                                                );
                                            }
                                            ctx.set_temp_struct_type(dest, &nested_name);
                                        }

                                        return Some(val);
                                    }
                                }
                            }
                        } else {
                            // Fallback: numeric index with default struct type
                            let idx = field.parse::<u32>().unwrap_or(0);
                            // Try to find any cached struct type
                            if let Some(struct_type) = ctx.lookup_struct_type("_default") {
                                if let Ok(field_ptr) =
                                    ctx.builder
                                        .build_struct_gep(struct_type, ptr, idx, "field_ptr")
                                {
                                    if let Ok(val) =
                                        ctx.builder.build_load(ctx.i64_type(), field_ptr, dest)
                                    {
                                        ctx.set_temp(dest, val);
                                        return Some(val);
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }

            MirInstrKind::FieldSet {
                object,
                field,
                value,
            } => {
                if let (Some(obj_ptr), Some(val)) =
                    (operand_to_value(ctx, object), operand_to_value(ctx, value))
                {
                    if obj_ptr.is_pointer_value() {
                        let ptr = obj_ptr.into_pointer_value();

                        // Get struct name from the object operand
                        let struct_name = Self::get_operand_name(object)
                            .and_then(|var_name| ctx.get_temp_struct_type(var_name).cloned());

                        if let Some(struct_name) = struct_name {
                            // Look up field index by name from struct metadata
                            let field_index =
                                ctx.get_field_index(&struct_name, field).unwrap_or_else(|| {
                                    // Fallback: try parsing field as numeric index
                                    field.parse::<u32>().unwrap_or(0)
                                });

                            // Get the struct type from cache
                            if let Some(struct_type) = ctx.lookup_struct_type(&struct_name) {
                                if let Ok(field_ptr) = ctx.builder.build_struct_gep(
                                    struct_type,
                                    ptr,
                                    field_index,
                                    "field_ptr",
                                ) {
                                    ctx.builder.build_store(field_ptr, val).ok();
                                }
                            }
                        } else {
                            // Fallback: numeric index with default struct type
                            let idx = field.parse::<u32>().unwrap_or(0);
                            if let Some(struct_type) = ctx.lookup_struct_type("_default") {
                                if let Ok(field_ptr) =
                                    ctx.builder
                                        .build_struct_gep(struct_type, ptr, idx, "field_ptr")
                                {
                                    ctx.builder.build_store(field_ptr, val).ok();
                                }
                            }
                        }
                    }
                }
                None
            }

            _ => None,
        }
    }
}
