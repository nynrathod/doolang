//! Composite/Aggregate Instruction Handler
//!
//! Handles: TupleCreate, TupleGet, StructCreate, FieldGet, FieldSet

use super::InstructionHandler;
use crate::context::CodegenContext;
use crate::utils::operand_to_value;
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_core::types::{builtin, TypeKind};
use doo_mir::sym::resolve;
use doo_mir::{MirInstr, MirInstrKind, MirOperand};
use inkwell::values::BasicValueEnum;

/// Composite instruction handler.
pub struct CompositeHandler;

impl CompositeHandler {
    /// Extract variable name from MirOperand for struct type lookup.
    fn get_operand_name(operand: &MirOperand) -> Option<String> {
        match operand {
            MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
                Some(resolve(*name))
            }
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
                let tuple_size_val = tuple_type
                    .size_of()
                    .unwrap_or_else(|| {
                        // Fallback: tuples are simpler — sum of 8-byte fields
                        let size = ((elements.len() as u64) * 8).max(16);
                        i64_type.const_int(size, false)
                    });
                let heap_ptr = ctx
                    .builder
                    .build_call(
                        malloc_fn,
                        &[tuple_size_val.into()],
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

                ctx.set_temp(&resolve(*dest), heap_ptr.into());
                Some(heap_ptr.into())
            }

            MirInstrKind::TupleGet {
                dest,
                tuple,
                index,
                tuple_type,
            } => {
                let dest_str = resolve(*dest);
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
                            let dest_type_id = ctx.get_variable_type(&dest_str);
                            let elem_llvm = match dest_type_id {
                                Some(tid) => ctx.get_llvm_type(tid),
                                None => ctx.i64_type().into(),
                            };
                            let num_elements = (*index + 1).max(2);
                            let element_types: Vec<inkwell::types::BasicTypeEnum> = (0
                                ..num_elements)
                                .map(|i| {
                                    if i == *index {
                                        elem_llvm
                                    } else {
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
                            if let Ok(val) = ctx.builder.build_load(elem_type, field_ptr, &dest_str) {
                                ctx.set_temp(&dest_str, val);

                                if let Some(tid) = ctx.get_variable_type(&dest_str) {
                                    if let Some(kind) = ctx.get_type_kind(tid) {
                                        match kind {
                                            TypeKind::Struct { name, .. } => {
                                                ctx.set_temp_struct_type(&dest_str, &name);
                                            }
                                            TypeKind::TypeRef { name: ref_name } => {
                                                if let Some(resolved_tid) =
                                                    ctx.type_registry.lookup(&ref_name)
                                                {
                                                    if let Some(TypeKind::Struct { name, .. }) =
                                                        ctx.get_type_kind(resolved_tid)
                                                    {
                                                        ctx.set_temp_struct_type(&dest_str, &name);
                                                    }
                                                }
                                            }
                                            TypeKind::Optional { inner } => {
                                                if let Some(inner_kind) = ctx.get_type_kind(inner) {
                                                    match inner_kind {
                                                        TypeKind::Struct { name, .. } => {
                                                            ctx.set_temp_struct_type(&dest_str, &name);
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
                                                                        &dest_str, &name,
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
                if ffi_names::is_object_lit(&resolve(*struct_name)) {
                    let ptr_type = ctx.ptr_type();
                    let i64_type = ctx.context.i64_type();

                    // Get or declare doo_map_new, doo_map_set, doo_map_set_str_array
                    let map_new_fn = ctx.module.get_function(ffi_names::DOO_MAP_NEW).unwrap_or_else(|| {
                        let fn_ty = ptr_type.fn_type(&[], false);
                        ctx.module.add_function(ffi_names::DOO_MAP_NEW, fn_ty, None)
                    });
                    let map_set_fn = ctx.module.get_function(ffi_names::DOO_MAP_SET).unwrap_or_else(|| {
                        let void_ty = ctx.context.void_type();
                        let fn_ty = void_ty
                            .fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
                        ctx.module.add_function(ffi_names::DOO_MAP_SET, fn_ty, None)
                    });
                    let map_set_arr_fn = ctx
                        .module
                        .get_function(ffi_names::DOO_MAP_SET_STR_ARRAY)
                        .unwrap_or_else(|| {
                            let void_ty = ctx.context.void_type();
                            let fn_ty = void_ty.fn_type(
                                &[ptr_type.into(), ptr_type.into(), ptr_type.into()],
                                false,
                            );
                            ctx.module
                                .add_function(ffi_names::DOO_MAP_SET_STR_ARRAY, fn_ty, None)
                        });

                    // Get or declare sprintf for int/bool to string conversion
                    let sprintf_fn = ctx.module.get_function(ffi_names::SPRINTF).unwrap_or_else(|| {
                        let i32_type = ctx.i32_type();
                        let fn_ty = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
                        ctx.module.add_function(ffi_names::SPRINTF, fn_ty, None)
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
                        let key_str = ctx.const_string(&resolve(*field_name));

                        // Determine if this value is an array (tracked by array_element_types)
                        let is_array = match value {
                            MirOperand::Temp(name) => {
                                ctx.array_element_types.contains_key(&resolve(*name))
                            }
                            MirOperand::Local(name) => {
                                ctx.array_element_types.contains_key(&resolve(*name))
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
                                    // Float or other: convert via doo_format_float (ryu)
                                    let ptr_type = ctx
                                        .context
                                        .i8_type()
                                        .ptr_type(inkwell::AddressSpace::default());
                                    let f64_type = ctx.f64_type();
                                    let format_fn = ctx
                                        .module
                                        .get_function(ffi_names::DOO_FORMAT_FLOAT)
                                        .unwrap_or_else(|| {
                                            let fn_ty =
                                                ptr_type.fn_type(&[f64_type.into()], false);
                                            ctx.module.add_function(
                                                ffi_names::DOO_FORMAT_FLOAT,
                                                fn_ty,
                                                None,
                                            )
                                        });
                                    let str_ptr = ctx
                                        .builder
                                        .build_call(
                                            format_fn,
                                            &[val.into()],
                                            "fmt_float",
                                        )
                                        .ok()
                                        .and_then(|v| v.try_as_basic_value().basic());
                                    if let Some(str_ptr) = str_ptr {
                                        ctx.builder
                                            .build_call(
                                                map_set_fn,
                                                &[
                                                    map_ptr.into(),
                                                    key_str.into(),
                                                    str_ptr.into(),
                                                ],
                                                "",
                                            )
                                            .ok();
                                    }
                                }
                            }
                        }
                    }

                    ctx.set_temp(&resolve(*dest), map_ptr.into());
                    return Some(map_ptr.into());
                }

                // Named struct: normal struct creation path
                // Collect field names in order for metadata
                let dest_str = resolve(*dest);
                let sname = resolve(*struct_name);
                let field_names: Vec<String> =
                    fields.iter().map(|(name, _)| resolve(*name)).collect();

                // Register struct metadata for field lookups
                ctx.register_struct_metadata(&sname, field_names);

                // Build LLVM struct type with correct field types from type registry
                // Use get_llvm_type for consistent type mapping across all code paths
                let field_types: Vec<inkwell::types::BasicTypeEnum> =
                    if let Some(field_type_ids) = ctx.get_struct_field_types(&sname) {
                        field_type_ids
                            .iter()
                            .map(|type_id| ctx.get_llvm_type(*type_id))
                            .collect()
                    } else {
                        // Fallback: use get_llvm_type based on operand types
                        fields.iter().map(|_| ctx.i64_type().into()).collect()
                    };
                let struct_type = ctx.get_struct_type(&sname, &field_types);

                // CRITICAL: Use LLVM's size_of() IntValue DIRECTLY as malloc argument.
                // DO NOT extract via get_zero_extended_constant() — it fails on ConstantExpr
                // (sizeof returns ConstantExpr, not ConstantInt), causing wrong fallback sizes.
                // This was the root cause of heap buffer overflow for structs with enum fields.
                let i64_type = ctx.context.i64_type();
                let struct_size_val = struct_type.size_of().unwrap_or_else(|| {
                    // Fallback: compute size accounting for enum fields (16 bytes each)
                    let mut offset: u64 = 0;
                    if let Some(field_type_ids) = ctx.get_struct_field_types(&sname) {
                        for type_id in &field_type_ids {
                            let field_size = match ctx.get_type_kind(*type_id) {
                                Some(doo_core::types::TypeKind::Enum { .. }) => 16,
                                _ => 8,
                            };
                            offset = (offset + 7) & !7;
                            offset += field_size;
                        }
                    } else {
                        offset = (fields.len() as u64) * 8;
                    }
                    let padded = ((offset + 7) & !7).max(16);
                    i64_type.const_int(padded, false)
                });
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
                        &[struct_size_val.into()],
                        &dest_str,
                    )
                    .ok()?
                    .try_as_basic_value()
                    .basic()?
                    .into_pointer_value();

                // Get field type IDs for proper boxing
                let _field_type_ids = ctx.get_struct_field_types(&sname);

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
                ctx.set_temp_struct_type(&dest_str, &sname);
                ctx.set_temp(&dest_str, struct_ptr.into());
                Some(struct_ptr.into())
            }

            MirInstrKind::FieldGet {
                dest,
                object,
                field,
            } => {
                let dest_str = resolve(*dest);
                let field_str = resolve(*field);
                let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();
                if debug {
                    let blk = ctx
                        .builder
                        .get_insert_block()
                        .map(|b| b.get_name().to_string_lossy().to_string());
                    doo_debug!("CODEGEN", "FieldGet {} in block {:?}", dest_str, blk);
                }
                if let Some(obj_ptr) = operand_to_value(ctx, object) {
                    if obj_ptr.is_pointer_value() {
                        let ptr = obj_ptr.into_pointer_value();

                        let var_name = Self::get_operand_name(object);
                        let struct_name = var_name.and_then(|name| {
                            if let Some(st) = ctx.get_temp_struct_type(&name).cloned() {
                                return Some(st);
                            }
                            if let Some(type_id) = ctx.get_variable_type(&name) {
                                if let Some(kind) = ctx.get_type_kind(type_id) {
                                    match kind {
                                        TypeKind::Struct { name, .. } => return Some(name),
                                        TypeKind::TypeRef { name: ref_name } => {
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
                                doo_debug!("CODEGEN", "WARNING: FieldGet {} has no struct type for {:?}", 
                                    dest_str, object);
                            } else {
                                doo_debug!(
                                    "CODEGEN",
                                    "FieldGet {} using struct_name={:?} for field={}",
                                    dest_str,
                                    struct_name,
                                    field_str
                                );
                            }
                        }

                        if let Some(struct_name) = struct_name {
                            let field_index =
                                ctx.get_field_index(&struct_name, &field_str).unwrap_or_else(|| {
                                    field_str.parse::<u32>().unwrap_or(0)
                                });
                            if debug {
                                doo_debug!(
                                    "CODEGEN",
                                    "FieldGet {} field_index={} for {}.{}",
                                    dest_str,
                                    field_index,
                                    struct_name,
                                    field_str
                                );
                            }

                            if let Some(struct_type) = ctx.get_or_build_struct_type(&struct_name) {
                                if let Ok(field_ptr) = ctx.builder.build_struct_gep(
                                    struct_type,
                                    ptr,
                                    field_index,
                                    "field_ptr",
                                ) {
                                    let field_type_id =
                                        ctx.get_struct_field_type(&struct_name, &field_str);

                                    let nested_struct_name = field_type_id.and_then(|tid| {
                                        if let Some(name) = ctx.get_struct_name_from_type_id(tid) {
                                            return Some(name);
                                        }
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
                                                ctx.ptr_type().into()
                                            }
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
                                                ctx.get_llvm_type(t)
                                            }
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
                                            _ => ctx.i64_type().into(),
                                        };

                                    if let Ok(val) =
                                        ctx.builder.build_load(load_type, field_ptr, &dest_str)
                                    {
                                        ctx.set_temp(&dest_str, val);

                                        if let Some(nested_name) = nested_struct_name {
                                            if debug {
                                                doo_debug!(
                                                    "CODEGEN",
                                                    "FieldGet {} setting nested struct type to {}",
                                                    dest_str,
                                                    nested_name
                                                );
                                            }
                                            ctx.set_temp_struct_type(&dest_str, &nested_name);
                                        }

                                        return Some(val);
                                    }
                                }
                            }
                        } else {
                            let idx = field_str.parse::<u32>().unwrap_or(0);
                            if let Some(struct_type) = ctx.lookup_struct_type("_default") {
                                if let Ok(field_ptr) =
                                    ctx.builder
                                        .build_struct_gep(struct_type, ptr, idx, "field_ptr")
                                {
                                    if let Ok(val) =
                                        ctx.builder.build_load(ctx.i64_type(), field_ptr, &dest_str)
                                    {
                                        ctx.set_temp(&dest_str, val);
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
                let field_str = resolve(*field);
                if let (Some(obj_ptr), Some(val)) =
                    (operand_to_value(ctx, object), operand_to_value(ctx, value))
                {
                    if obj_ptr.is_pointer_value() {
                        let ptr = obj_ptr.into_pointer_value();

                        // Get struct name from the object operand
                        let struct_name = Self::get_operand_name(object)
                            .and_then(|var_name| ctx.get_temp_struct_type(&var_name).cloned());

                        if let Some(struct_name) = struct_name {
                            // Look up field index by name from struct metadata
                            let field_index =
                                ctx.get_field_index(&struct_name, &field_str).unwrap_or_else(|| {
                                    // Fallback: try parsing field as numeric index
                                    field_str.parse::<u32>().unwrap_or(0)
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
                            let idx = field_str.parse::<u32>().unwrap_or(0);
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
