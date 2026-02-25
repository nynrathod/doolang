use crate::context::CodegenContext;
use crate::layout;
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_core::types::{builtin, TypeId, TypeKind};
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

pub struct JsonBuiltins;

impl JsonBuiltins {
    /// Provide `JSON.stringify(value)` support.
    /// Returns a pointer to a DooString (as i8* or specialized struct pointer).
    pub fn emit_stringify<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        val: BasicValueEnum<'ctx>,
        val_type: TypeId,
    ) -> Option<BasicValueEnum<'ctx>> {
        // 1. Declare FFI functions
        let finish_fn = Self::get_or_declare_finish(ctx);

        // 2. Estimate buffer capacity from type info (reduces reallocs)
        // Structs: fields * 20 + 16 (key+value+separators per field)
        // Default: 64 bytes (covers most small JSON responses)
        let estimated_cap = match ctx.get_type_kind(val_type) {
            Some(TypeKind::Struct { fields, .. }) => fields.len() * 20 + 16,
            _ => 0, // 0 signals: use default capacity
        };

        let writer_ptr = if estimated_cap > 0 {
            let new_fn = Self::get_or_declare_new_with_cap(ctx);
            let cap_val = ctx
                .context
                .i64_type()
                .const_int(estimated_cap as u64, false);
            ctx.builder
                .build_call(new_fn, &[cap_val.into()], "json_writer")
                .ok()?
                .try_as_basic_value()
                .basic()?
                .into_pointer_value()
        } else {
            let new_fn = Self::get_or_declare_new(ctx);
            ctx.builder
                .build_call(new_fn, &[], "json_writer")
                .ok()?
                .try_as_basic_value()
                .basic()?
                .into_pointer_value()
        };

        // 3. Emit recursive write
        Self::emit_write_value(ctx, writer_ptr, val, val_type)?;

        // 4. Finish (get string) - finish() consumes the writer, so no need to call free()
        let result_str_ptr = ctx
            .builder
            .build_call(finish_fn, &[writer_ptr.into()], "json_str")
            .ok()?
            .try_as_basic_value()
            .basic()?;

        // Note: doo_json_writer_finish already frees the writer via Box::from_raw
        // Do NOT call free here - that would be a double-free!

        Some(result_str_ptr)
    }

    /// Provide `JSON.parse(str)` support.
    /// Uses type-specific FFI functions based on target return type.
    pub fn emit_parse<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        val: BasicValueEnum<'ctx>,
        target_type: Option<TypeId>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let i8_ptr = ctx.context.i8_type().ptr_type(AddressSpace::default());

        // Check if this is a complex type that needs inline codegen
        if let Some(ty) = target_type {
            let kind = ctx.get_type_kind(ty);
            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                doo_debug!(
                    "CODEGEN",
                    "emit_parse: target_type={:?}, kind={:?}",
                    ty,
                    kind
                );
            }
            match kind {
                Some(TypeKind::Struct { name, fields }) => {
                    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                        doo_debug!("CODEGEN", "emit_parse -> emit_parse_struct for '{}'", name);
                    }
                    // Extract just name and type for parsing (visibility not needed)
                    let field_pairs: Vec<_> =
                        fields.iter().map(|(n, t, _)| (n.clone(), *t)).collect();
                    return Self::emit_parse_struct(ctx, val, ty, &name, &field_pairs);
                }
                Some(TypeKind::Enum { name, variants }) => {
                    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                        doo_debug!("CODEGEN", "emit_parse -> emit_parse_enum for '{}'", name);
                    }
                    return Self::emit_parse_enum(ctx, val, ty, &name, &variants);
                }
                Some(TypeKind::Array { element: elem_type }) => {
                    let elem_kind = ctx.get_type_kind(elem_type);
                    match elem_kind {
                        Some(TypeKind::Struct { name, fields }) => {
                            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                                doo_debug!(
                                    "CODEGEN",
                                    "emit_parse -> emit_parse_array_struct for '[{}]'",
                                    name
                                );
                            }
                            let field_pairs: Vec<_> =
                                fields.iter().map(|(n, t, _)| (n.clone(), *t)).collect();
                            return Self::emit_parse_array_struct(
                                ctx,
                                val,
                                elem_type,
                                &name,
                                &field_pairs,
                            );
                        }
                        Some(TypeKind::Enum { name, variants }) => {
                            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                                doo_debug!(
                                    "CODEGEN",
                                    "emit_parse -> emit_parse_array_enum for '[{}]'",
                                    name
                                );
                            }
                            return Self::emit_parse_array_enum(
                                ctx, val, elem_type, &name, &variants,
                            );
                        }
                        _ => {} // Fall through to FFI dispatch for primitive arrays
                    }
                }
                _ => {}
            }
        }

        // Determine the FFI function name and return type based on target_type
        let (fn_name, ret_type): (&str, BasicTypeEnum<'ctx>) = match target_type {
            Some(ty) => {
                let kind = ctx.get_type_kind(ty);
                match kind {
                    Some(TypeKind::Int) => (ffi_names::DOO_JSON_PARSE_INT, ctx.i64_type().into()),
                    Some(TypeKind::Float) => {
                        (ffi_names::DOO_JSON_PARSE_FLOAT, ctx.f64_type().into())
                    }
                    Some(TypeKind::Bool) => (
                        ffi_names::DOO_JSON_PARSE_BOOL,
                        ctx.context.i8_type().into(), // Bool is i8 for C ABI compatibility
                    ),
                    Some(TypeKind::Str) => (ffi_names::DOO_JSON_PARSE_STR, i8_ptr.into()),
                    Some(TypeKind::Array { element: elem_type }) => {
                        let elem_kind = ctx.get_type_kind(elem_type);
                        match elem_kind {
                            Some(TypeKind::Int) => {
                                (ffi_names::DOO_JSON_PARSE_ARRAY_INT, i8_ptr.into())
                            }
                            Some(TypeKind::Float) => {
                                (ffi_names::DOO_JSON_PARSE_ARRAY_FLOAT, i8_ptr.into())
                            }
                            Some(TypeKind::Bool) => {
                                (ffi_names::DOO_JSON_PARSE_ARRAY_BOOL, i8_ptr.into())
                            }
                            Some(TypeKind::Str) => {
                                (ffi_names::DOO_JSON_PARSE_ARRAY_STR, i8_ptr.into())
                            }
                            _ => (ffi_names::DOO_JSON_PARSE, i8_ptr.into()),
                        }
                    }
                    Some(TypeKind::Map {
                        key: key_type,
                        value: val_type,
                    }) => {
                        let key_kind = ctx.get_type_kind(key_type);
                        let val_kind = ctx.get_type_kind(val_type);
                        match (key_kind, val_kind) {
                            (Some(TypeKind::Str), Some(TypeKind::Int)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_STR_INT, i8_ptr.into())
                            }
                            (Some(TypeKind::Str), Some(TypeKind::Float)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_STR_FLOAT, i8_ptr.into())
                            }
                            (Some(TypeKind::Str), Some(TypeKind::Bool)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_STR_BOOL, i8_ptr.into())
                            }
                            (Some(TypeKind::Str), Some(TypeKind::Str)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_STR_STR, i8_ptr.into())
                            }
                            (Some(TypeKind::Int), Some(TypeKind::Int)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_INT_INT, i8_ptr.into())
                            }
                            (Some(TypeKind::Int), Some(TypeKind::Float)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_INT_FLOAT, i8_ptr.into())
                            }
                            (Some(TypeKind::Int), Some(TypeKind::Bool)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_INT_BOOL, i8_ptr.into())
                            }
                            (Some(TypeKind::Int), Some(TypeKind::Str)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_INT_STR, i8_ptr.into())
                            }
                            (Some(TypeKind::Float), Some(TypeKind::Int)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_FLOAT_INT, i8_ptr.into())
                            }
                            (Some(TypeKind::Float), Some(TypeKind::Float)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_FLOAT_FLOAT, i8_ptr.into())
                            }
                            (Some(TypeKind::Float), Some(TypeKind::Bool)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_FLOAT_BOOL, i8_ptr.into())
                            }
                            (Some(TypeKind::Float), Some(TypeKind::Str)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_FLOAT_STR, i8_ptr.into())
                            }
                            (Some(TypeKind::Bool), Some(TypeKind::Int)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_BOOL_INT, i8_ptr.into())
                            }
                            (Some(TypeKind::Bool), Some(TypeKind::Float)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_BOOL_FLOAT, i8_ptr.into())
                            }
                            (Some(TypeKind::Bool), Some(TypeKind::Bool)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_BOOL_BOOL, i8_ptr.into())
                            }
                            (Some(TypeKind::Bool), Some(TypeKind::Str)) => {
                                (ffi_names::DOO_JSON_PARSE_MAP_BOOL_STR, i8_ptr.into())
                            }
                            _ => (ffi_names::DOO_JSON_PARSE, i8_ptr.into()),
                        }
                    }
                    _ => (ffi_names::DOO_JSON_PARSE, i8_ptr.into()),
                }
            }
            None => (ffi_names::DOO_JSON_PARSE, i8_ptr.into()),
        };

        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
            doo_debug!(
                "CODEGEN",
                "JSON.parse: target_type={:?}, fn_name={}, ret_type={:?}",
                target_type,
                fn_name,
                ret_type
            );
        }

        // Get or declare the FFI function
        let func = ctx.get_function(fn_name).unwrap_or_else(|| {
            let ft = ret_type.fn_type(&[i8_ptr.into()], false);
            ctx.module.add_function(fn_name, ft, None)
        });

        let call = ctx.builder.build_call(func, &[val.into()], "parsed").ok()?;
        call.try_as_basic_value().basic()
    }

    /// Emit code to parse JSON into a struct
    /// JSON: {"field1": val1, "field2": val2, ...}
    fn emit_parse_struct<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        json_str: BasicValueEnum<'ctx>,
        _ty: TypeId,
        name: &str,
        fields: &[(String, TypeId)],
    ) -> Option<BasicValueEnum<'ctx>> {
        let i8_ptr = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type();

        // ── Parse-Once: parse the JSON string ONCE into an opaque object handle ──
        let parse_object_fn = Self::get_or_declare_parse_object(ctx);
        let obj_handle = ctx
            .builder
            .build_call(parse_object_fn, &[json_str.into()], "json_obj")
            .ok()?
            .try_as_basic_value()
            .basic()?;

        // ── Declare typed extraction functions lazily ──
        let get_int_fn = Self::get_or_declare_object_get_int(ctx);
        let get_float_fn = Self::get_or_declare_object_get_float(ctx);
        let get_bool_fn = Self::get_or_declare_object_get_bool(ctx);
        let get_str_fn = Self::get_or_declare_object_get_str(ctx);
        let get_json_fn = Self::get_or_declare_object_get_json(ctx);

        // Build struct type using get_struct_type for consistency with StructCreate
        let field_types: Vec<_> = fields.iter().map(|(_, t)| ctx.get_llvm_type(*t)).collect();
        let struct_llvm_type = ctx.get_struct_type(name, &field_types);

        // CRITICAL: Use HEAP allocation (malloc) NOT stack allocation (alloca)
        // Stack allocations become invalid when the pointer escapes the function.
        // CRITICAL: Use LLVM's size_of() IntValue DIRECTLY as malloc argument.
        // DO NOT extract via get_zero_extended_constant() — it fails on ConstantExpr
        // (sizeof returns ConstantExpr, not ConstantInt), causing wrong fallback sizes.
        let struct_size_val = struct_llvm_type.size_of().unwrap_or_else(|| {
            // Fallback: compute size accounting for enum fields (16 bytes each)
            let mut offset: u64 = 0;
            for (_, type_id) in fields.iter() {
                let field_size = match ctx.get_type_kind(*type_id) {
                    Some(doo_core::types::TypeKind::Enum { .. }) => 16, // { i32, ptr }
                    _ => 8,
                };
                offset = (offset + 7) & !7;
                offset += field_size;
            }
            let padded = ((offset + 7) & !7).max(16);
            i64_type.const_int(padded, false)
        });

        // Get or declare malloc
        let malloc_fn = ctx
            .module
            .get_function(ffi_names::MALLOC)
            .unwrap_or_else(|| {
                let fn_ty = ptr_type.fn_type(&[i64_type.into()], false);
                ctx.module.add_function(ffi_names::MALLOC, fn_ty, None)
            });

        // Heap allocate the struct
        let struct_ptr = ctx
            .builder
            .build_call(malloc_fn, &[struct_size_val.into()], "parsed_struct")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();

        // ── For each field, use typed extraction (zero re-serialization for primitives) ──
        for (i, (fname, fty)) in fields.iter().enumerate() {
            let field_name_str = ctx.const_string(fname);
            let kind = ctx.get_type_kind(*fty);

            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                doo_debug!(
                    "CODEGEN",
                    "emit_parse_struct field '{}': type_id={:?}, kind={:?}",
                    fname,
                    fty,
                    kind
                );
            }

            let field_ptr = ctx
                .builder
                .build_struct_gep(struct_llvm_type, struct_ptr, i as u32, "field_ptr")
                .ok()?;

            match &kind {
                // ── Primitive: direct typed extraction, zero re-serialization ──
                Some(TypeKind::Int) => {
                    let val = ctx
                        .builder
                        .build_call(
                            get_int_fn,
                            &[obj_handle.into(), field_name_str.into()],
                            "field_int",
                        )
                        .ok()?
                        .try_as_basic_value()
                        .basic()?;
                    ctx.builder.build_store(field_ptr, val).ok()?;
                }
                Some(TypeKind::Float) => {
                    let val = ctx
                        .builder
                        .build_call(
                            get_float_fn,
                            &[obj_handle.into(), field_name_str.into()],
                            "field_float",
                        )
                        .ok()?
                        .try_as_basic_value()
                        .basic()?;
                    ctx.builder.build_store(field_ptr, val).ok()?;
                }
                Some(TypeKind::Bool) => {
                    let i32_val = ctx
                        .builder
                        .build_call(
                            get_bool_fn,
                            &[obj_handle.into(), field_name_str.into()],
                            "field_bool_i32",
                        )
                        .ok()?
                        .try_as_basic_value()
                        .basic()?;
                    // Bool is i8 in Doo ABI — truncate from i32
                    let i8_val = ctx
                        .builder
                        .build_int_truncate(
                            i32_val.into_int_value(),
                            ctx.context.i8_type(),
                            "field_bool",
                        )
                        .ok()?;
                    ctx.builder.build_store(field_ptr, i8_val).ok()?;
                }
                Some(TypeKind::Str) => {
                    let val = ctx
                        .builder
                        .build_call(
                            get_str_fn,
                            &[obj_handle.into(), field_name_str.into()],
                            "field_str",
                        )
                        .ok()?
                        .try_as_basic_value()
                        .basic()?;
                    ctx.builder.build_store(field_ptr, val).ok()?;
                }
                // ── Composite: get as JSON string, then recursively parse ──
                _ => {
                    let field_json = ctx
                        .builder
                        .build_call(
                            get_json_fn,
                            &[obj_handle.into(), field_name_str.into()],
                            "field_json",
                        )
                        .ok()?
                        .try_as_basic_value()
                        .basic()?;

                    let field_val = Self::emit_parse(ctx, field_json, Some(*fty))?;

                    let field_llvm_type = ctx.get_llvm_type(*fty);

                    if matches!(kind, Some(TypeKind::Enum { .. })) {
                        // Enum: emit_parse returns ptr to { i32, ptr }, need to load the value
                        let enum_ptr = field_val.into_pointer_value();
                        let enum_val = ctx
                            .builder
                            .build_load(field_llvm_type, enum_ptr, "enum_val")
                            .ok()?;
                        ctx.builder.build_store(field_ptr, enum_val).ok()?;
                    } else {
                        // Non-enum composite: store directly
                        ctx.builder.build_store(field_ptr, field_val).ok()?;
                    }
                }
            }
        }

        // ── Free the cached parse ──
        let free_fn = Self::get_or_declare_object_free(ctx);
        ctx.builder
            .build_call(free_fn, &[obj_handle.into()], "")
            .ok()?;

        // Return pointer to struct (cast to generic ptr)
        let ptr = ctx
            .builder
            .build_pointer_cast(struct_ptr, i8_ptr, "struct_ptr")
            .ok()?;
        Some(ptr.into())
    }

    /// Emit code to parse a JSON array of structs.
    /// Generates an LLVM IR loop that calls emit_parse_struct for each element.
    /// Returns the data pointer (after the 16-byte Doo array header).
    fn emit_parse_array_struct<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        json_str: BasicValueEnum<'ctx>,
        elem_ty: TypeId,
        struct_name: &str,
        struct_fields: &[(String, TypeId)],
    ) -> Option<BasicValueEnum<'ctx>> {
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type();

        // ── Get or declare helper FFI functions ──
        let array_count_fn = Self::get_or_declare_array_count(ctx);
        let array_get_element_fn = Self::get_or_declare_array_get_element(ctx);

        // ── Get array element count ──
        let count = ctx
            .builder
            .build_call(array_count_fn, &[json_str.into()], "arr_count")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        // ── Allocate Doo array (header[16] + count * ptr_size) ──
        let data_ptr = crate::layout::alloc_with_header(ctx, count, ptr_type, "struct_arr")?;

        // ── Loop: for i = 0; i < count; i++ ──
        let func = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_preheader = ctx.builder.get_insert_block()?;
        let loop_body = ctx.context.append_basic_block(func, "arr_struct_loop");
        let loop_end = ctx.context.append_basic_block(func, "arr_struct_done");

        // Skip loop if empty
        let has_elements = ctx
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SGT,
                count,
                i64_type.const_zero(),
                "has_elements",
            )
            .ok()?;
        ctx.builder
            .build_conditional_branch(has_elements, loop_body, loop_end)
            .ok()?;

        // ── Loop body ──
        ctx.builder.position_at_end(loop_body);
        let idx_phi = ctx.builder.build_phi(i64_type, "idx").ok()?;
        idx_phi.add_incoming(&[(&i64_type.const_zero(), loop_preheader)]);
        let idx = idx_phi.as_basic_value().into_int_value();

        // Get element JSON string: doo_json_array_get_element(json_str, idx)
        let elem_json = ctx
            .builder
            .build_call(
                array_get_element_fn,
                &[json_str.into(), idx.into()],
                "elem_json",
            )
            .ok()?
            .try_as_basic_value()
            .basic()?;

        // Parse element into typed struct (this generates inline codegen, may create basic blocks)
        let struct_ptr =
            Self::emit_parse_struct(ctx, elem_json, elem_ty, struct_name, struct_fields)?;

        // Store struct pointer at data[idx]
        let elem_slot = unsafe {
            ctx.builder
                .build_in_bounds_gep(ptr_type, data_ptr, &[idx], "elem_slot")
                .ok()?
        };
        ctx.builder.build_store(elem_slot, struct_ptr).ok()?;

        // CRITICAL: Capture current block AFTER all operations
        // (emit_parse_struct may have created nested basic blocks for recursive parsing)
        let loop_back_block = ctx.builder.get_insert_block()?;

        // Increment index
        let next_idx = ctx
            .builder
            .build_int_add(idx, i64_type.const_int(1, false), "next_idx")
            .ok()?;
        idx_phi.add_incoming(&[(&next_idx, loop_back_block)]);

        // Check loop condition
        let continue_loop = ctx
            .builder
            .build_int_compare(inkwell::IntPredicate::SLT, next_idx, count, "continue")
            .ok()?;
        ctx.builder
            .build_conditional_branch(continue_loop, loop_body, loop_end)
            .ok()?;

        // ── Loop end: return data pointer ──
        ctx.builder.position_at_end(loop_end);
        Some(data_ptr.into())
    }

    /// Emit code to parse a JSON array of enums.
    /// Same loop pattern as array-of-struct but calls emit_parse_enum per element.
    fn emit_parse_array_enum<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        json_str: BasicValueEnum<'ctx>,
        elem_ty: TypeId,
        enum_name: &str,
        variants: &[(String, Option<TypeId>)],
    ) -> Option<BasicValueEnum<'ctx>> {
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type();

        let array_count_fn = Self::get_or_declare_array_count(ctx);
        let array_get_element_fn = Self::get_or_declare_array_get_element(ctx);

        let count = ctx
            .builder
            .build_call(array_count_fn, &[json_str.into()], "arr_count")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        let data_ptr = crate::layout::alloc_with_header(ctx, count, ptr_type, "enum_arr")?;

        let func = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_preheader = ctx.builder.get_insert_block()?;
        let loop_body = ctx.context.append_basic_block(func, "arr_enum_loop");
        let loop_end = ctx.context.append_basic_block(func, "arr_enum_done");

        let has_elements = ctx
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SGT,
                count,
                i64_type.const_zero(),
                "has_elements",
            )
            .ok()?;
        ctx.builder
            .build_conditional_branch(has_elements, loop_body, loop_end)
            .ok()?;

        ctx.builder.position_at_end(loop_body);
        let idx_phi = ctx.builder.build_phi(i64_type, "idx").ok()?;
        idx_phi.add_incoming(&[(&i64_type.const_zero(), loop_preheader)]);
        let idx = idx_phi.as_basic_value().into_int_value();

        let elem_json = ctx
            .builder
            .build_call(
                array_get_element_fn,
                &[json_str.into(), idx.into()],
                "elem_json",
            )
            .ok()?
            .try_as_basic_value()
            .basic()?;

        let enum_ptr = Self::emit_parse_enum(ctx, elem_json, elem_ty, enum_name, variants)?;

        let elem_slot = unsafe {
            ctx.builder
                .build_in_bounds_gep(ptr_type, data_ptr, &[idx], "elem_slot")
                .ok()?
        };
        ctx.builder.build_store(elem_slot, enum_ptr).ok()?;

        let loop_back_block = ctx.builder.get_insert_block()?;

        let next_idx = ctx
            .builder
            .build_int_add(idx, i64_type.const_int(1, false), "next_idx")
            .ok()?;
        idx_phi.add_incoming(&[(&next_idx, loop_back_block)]);

        let continue_loop = ctx
            .builder
            .build_int_compare(inkwell::IntPredicate::SLT, next_idx, count, "continue")
            .ok()?;
        ctx.builder
            .build_conditional_branch(continue_loop, loop_body, loop_end)
            .ok()?;

        ctx.builder.position_at_end(loop_end);
        Some(data_ptr.into())
    }

    /// Emit code to parse JSON into an enum
    /// JSON: "VariantName" (unit) or {"VariantName": payload}
    fn emit_parse_enum<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        json_str: BasicValueEnum<'ctx>,
        ty: TypeId,
        _name: &str,
        variants: &[(String, Option<TypeId>)],
    ) -> Option<BasicValueEnum<'ctx>> {
        let i8_ptr = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let i32_type = ctx.context.i32_type();
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type();

        // Use proper enum struct type { i32 tag, ptr payload }
        let enum_struct_type = ctx
            .context
            .struct_type(&[i32_type.into(), i8_ptr.into()], false);

        // Get helper functions
        let get_variant_name_fn = Self::get_or_declare_get_variant_name(ctx);
        let get_variant_payload_fn = Self::get_or_declare_get_variant_payload(ctx);
        let strcmp_fn = Self::get_or_declare_strcmp(ctx);

        // Get variant name from JSON
        let variant_name = ctx
            .builder
            .build_call(get_variant_name_fn, &[json_str.into()], "variant_name")
            .ok()?
            .try_as_basic_value()
            .basic()?;

        // CRITICAL: Use HEAP allocation (malloc) NOT stack allocation (alloca)
        // Stack allocations become invalid when the pointer escapes the function.
        // Enum struct size: i32 (4 bytes padded to 8) + ptr (8 bytes) = 16 bytes
        let enum_size = 16u64;

        // Get or declare malloc
        let malloc_fn = ctx
            .module
            .get_function(ffi_names::MALLOC)
            .unwrap_or_else(|| {
                let fn_ty = ptr_type.fn_type(&[i64_type.into()], false);
                ctx.module.add_function(ffi_names::MALLOC, fn_ty, None)
            });

        // Heap allocate the enum
        let enum_ptr = ctx
            .builder
            .build_call(
                malloc_fn,
                &[i64_type.const_int(enum_size, false).into()],
                "parsed_enum",
            )
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();

        // Build comparison chain for each variant
        let parent = ctx.builder.get_insert_block()?.get_parent()?;
        let end_bb = ctx.context.append_basic_block(parent, "enum_parse_end");

        let mut current_bb = ctx.builder.get_insert_block()?;

        for (i, (vname, payload_ty)) in variants.iter().enumerate() {
            let match_bb = ctx
                .context
                .append_basic_block(parent, &format!("enum_match_{}", i));
            let next_bb = ctx
                .context
                .append_basic_block(parent, &format!("enum_next_{}", i));

            // Compare variant name
            ctx.builder.position_at_end(current_bb);
            let vname_str = ctx.const_string(vname);
            let cmp_result = ctx
                .builder
                .build_call(
                    strcmp_fn,
                    &[variant_name.into(), vname_str.into()],
                    "strcmp_result",
                )
                .ok()?
                .try_as_basic_value()
                .basic()?
                .into_int_value();
            let is_match = ctx
                .builder
                .build_int_compare(
                    IntPredicate::EQ,
                    cmp_result,
                    i32_type.const_zero(),
                    "is_match",
                )
                .ok()?;
            ctx.builder
                .build_conditional_branch(is_match, match_bb, next_bb)
                .ok()?;

            // Match block: set tag and parse payload
            ctx.builder.position_at_end(match_bb);

            // Set tag using struct GEP (field 0)
            let tag_ptr = ctx
                .builder
                .build_struct_gep(enum_struct_type, enum_ptr, 0, "tag_ptr")
                .ok()?;
            ctx.builder
                .build_store(tag_ptr, i32_type.const_int(i as u64, false))
                .ok()?;

            // Get payload pointer using struct GEP (field 1)
            let payload_field_ptr = ctx
                .builder
                .build_struct_gep(enum_struct_type, enum_ptr, 1, "payload_field_ptr")
                .ok()?;

            // Parse payload if present, otherwise store null
            if let Some(pty) = payload_ty {
                let payload_json = ctx
                    .builder
                    .build_call(get_variant_payload_fn, &[json_str.into()], "payload_json")
                    .ok()?
                    .try_as_basic_value()
                    .basic()?;

                let payload_val = Self::emit_parse(ctx, payload_json, Some(*pty))?;

                // For consistency with EnumCreate, we need to store a POINTER to the payload
                // at the payload field, not the value itself.
                // For pointer types (Str, Array, etc.), store the pointer directly.
                // For value types (Int, Float, Bool), allocate on HEAP and store pointer.
                // CRITICAL: Use malloc NOT alloca - alloca memory becomes invalid when pointer escapes.
                let llvm_pty = ctx.get_llvm_type(*pty);
                let payload_ptr_to_store = if llvm_pty.is_pointer_type() {
                    // Pointer type: the value IS a pointer, store it directly
                    payload_val.into_pointer_value()
                } else {
                    // Value type: HEAP allocate and store, then use heap ptr as the pointer
                    // Get the size of the value type
                    let value_size = match llvm_pty {
                        inkwell::types::BasicTypeEnum::IntType(it) => it.get_bit_width() as u64 / 8,
                        inkwell::types::BasicTypeEnum::FloatType(_) => 8, // f64
                        _ => 8,                                           // Default to 8 bytes
                    }
                    .max(8); // Minimum 8 bytes for alignment

                    let payload_heap_ptr = ctx
                        .builder
                        .build_call(
                            malloc_fn,
                            &[i64_type.const_int(value_size, false).into()],
                            "payload_heap",
                        )
                        .ok()?
                        .try_as_basic_value()
                        .basic()?
                        .into_pointer_value();
                    ctx.builder
                        .build_store(payload_heap_ptr, payload_val)
                        .ok()?;
                    payload_heap_ptr
                };

                // Store the payload pointer at field 1 (payload field is a pointer)
                ctx.builder
                    .build_store(payload_field_ptr, payload_ptr_to_store)
                    .ok()?;
            } else {
                // No payload - store null pointer at payload field
                ctx.builder
                    .build_store(payload_field_ptr, i8_ptr.const_null())
                    .ok()?;
            }

            ctx.builder.build_unconditional_branch(end_bb).ok()?;

            current_bb = next_bb;
        }

        // Default case (shouldn't happen for valid JSON) - jump to end
        ctx.builder.position_at_end(current_bb);
        ctx.builder.build_unconditional_branch(end_bb).ok()?;

        // End block
        ctx.builder.position_at_end(end_bb);

        Some(enum_ptr.into())
    }

    /// Emit code to write a map key as a quoted string (JSON standard requires all keys to be strings)
    /// This handles Int, Float, Bool, and Str key types appropriately
    fn emit_write_map_key<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        writer: PointerValue<'ctx>,
        val: BasicValueEnum<'ctx>,
        ty: TypeId,
    ) -> Option<()> {
        let kind = ctx.get_type_kind(ty)?;

        match kind {
            TypeKind::Str => {
                // String keys can use the normal write_key function
                let func = Self::get_or_declare_write_key(ctx);
                let ptr = val.into_pointer_value();
                ctx.builder
                    .build_call(func, &[writer.into(), ptr.into()], "")
                    .ok()?;
            }
            TypeKind::Int => {
                // Int keys need to be written as quoted strings
                let func = Self::get_or_declare_write_key_int(ctx);
                let val_i64 = ctx
                    .builder
                    .build_int_z_extend_or_bit_cast(val.into_int_value(), ctx.i64_type(), "cast")
                    .ok()?;
                ctx.builder
                    .build_call(func, &[writer.into(), val_i64.into()], "")
                    .ok()?;
            }
            TypeKind::Float => {
                // Float keys need to be written as quoted strings
                let func = Self::get_or_declare_write_key_float(ctx);
                let val_f64 = ctx
                    .builder
                    .build_float_cast(val.into_float_value(), ctx.f64_type(), "cast")
                    .ok()?;
                ctx.builder
                    .build_call(func, &[writer.into(), val_f64.into()], "")
                    .ok()?;
            }
            TypeKind::Bool => {
                // Bool keys need to be written as quoted strings
                let func = Self::get_or_declare_write_key_bool(ctx);
                let val_bool = val.into_int_value(); // i1
                ctx.builder
                    .build_call(func, &[writer.into(), val_bool.into()], "")
                    .ok()?;
            }
            _ => {
                // Unsupported key type - fall back to write_value (may produce invalid JSON)
                return Self::emit_write_value(ctx, writer, val, ty);
            }
        }
        Some(())
    }

    fn emit_write_value<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        writer: PointerValue<'ctx>,
        val: BasicValueEnum<'ctx>,
        ty: TypeId,
    ) -> Option<()> {
        let kind = ctx.get_type_kind(ty)?;

        match kind {
            TypeKind::Int => {
                let func = Self::get_or_declare_write_int(ctx);
                // Ensure val is i64
                let val_i64 = if val.is_int_value() {
                    ctx.builder
                        .build_int_z_extend_or_bit_cast(
                            val.into_int_value(),
                            ctx.i64_type(),
                            "cast",
                        )
                        .ok()?
                } else {
                    return None;
                };
                ctx.builder
                    .build_call(func, &[writer.into(), val_i64.into()], "")
                    .ok()?;
            }
            TypeKind::Float => {
                let func = Self::get_or_declare_write_float(ctx);
                let val_f64 = if val.is_float_value() {
                    ctx.builder
                        .build_float_cast(val.into_float_value(), ctx.f64_type(), "cast")
                        .ok()?
                } else {
                    return None;
                };
                ctx.builder
                    .build_call(func, &[writer.into(), val_f64.into()], "")
                    .ok()?;
            }
            TypeKind::Bool => {
                let func = Self::get_or_declare_write_bool(ctx);
                let val_bool = if val.is_int_value() {
                    val.into_int_value() // i1
                } else {
                    return None;
                };
                ctx.builder
                    .build_call(func, &[writer.into(), val_bool.into()], "")
                    .ok()?;
            }
            TypeKind::Str => {
                let func = Self::get_or_declare_write_string(ctx);
                // Str in Doo is pointer to DooString or char*?
                // Assume char* for now or cast.
                // If it's DooString*, we might need to extract data pointer.
                // Assuming it's `i8*` (C string) based on previous `print` logic?
                // `emit_print_value` treated it as `val.is_pointer_value()`.
                // Note: `doo_ffi_json` expects `*const c_char`.
                let ptr = if val.is_pointer_value() {
                    val.into_pointer_value()
                } else {
                    return None;
                };
                ctx.builder
                    .build_call(func, &[writer.into(), ptr.into()], "")
                    .ok()?;
            }
            TypeKind::Array { element } => {
                let start_fn = Self::get_or_declare_start_array(ctx);
                let end_fn = Self::get_or_declare_end_array(ctx);
                let comma_fn = Self::get_or_declare_comma(ctx);

                ctx.builder
                    .build_call(start_fn, &[writer.into()], "")
                    .ok()?;

                if !val.is_pointer_value() {
                    return None;
                }
                let array_ptr = val.into_pointer_value();

                // Generate loop
                Self::emit_array_loop(ctx, writer, array_ptr, element, comma_fn)?;

                ctx.builder.build_call(end_fn, &[writer.into()], "").ok()?;
            }
            TypeKind::Map { key, value } => {
                // Must be string keys for JSON object, or convert others to string
                let start_fn = Self::get_or_declare_start_object(ctx);
                let end_fn = Self::get_or_declare_end_object(ctx);
                let comma_fn = Self::get_or_declare_comma(ctx);
                let colon_fn = Self::get_or_declare_colon(ctx);
                let key_fn = Self::get_or_declare_write_key(ctx); // writes "key"

                ctx.builder
                    .build_call(start_fn, &[writer.into()], "")
                    .ok()?;

                if !val.is_pointer_value() {
                    return None;
                }
                let map_ptr = val.into_pointer_value();

                // Iterate directly over map entries (key-value pairs) like emit_print_map
                // Map Layout: [len: i64][cap: i64][entries...] where entries are {key, value} pairs
                // map_ptr points to data section (after 16-byte header)

                // Get length using centralized layout helper
                let len_i64 = layout::get_array_length_from_data(ctx, map_ptr)?;

                // Build pair type for iteration
                let key_llvm = ctx.get_llvm_type(key);
                let val_llvm = ctx.get_llvm_type(value);
                let pair_ty = ctx
                    .context
                    .struct_type(&[key_llvm.into(), val_llvm.into()], false);
                let pair_ptr_ty = ctx.context.ptr_type(AddressSpace::default());
                let base = ctx
                    .builder
                    .build_pointer_cast(map_ptr, pair_ptr_ty, "map_data_cast")
                    .ok()?;

                // Loop setup
                let parent = ctx.builder.get_insert_block()?.get_parent()?;
                let loop_bb = ctx.context.append_basic_block(parent, "json_map_loop");
                let body_bb = ctx.context.append_basic_block(parent, "json_map_body");
                let inc_bb = ctx.context.append_basic_block(parent, "json_map_inc");
                let after_bb = ctx.context.append_basic_block(parent, "json_map_end");

                let i64_type = ctx.i64_type();
                let idx_ptr = ctx.builder.build_alloca(i64_type, "idx").ok()?;
                ctx.builder
                    .build_store(idx_ptr, i64_type.const_zero())
                    .ok()?;
                ctx.builder.build_unconditional_branch(loop_bb).ok()?;

                // LOOP condition
                ctx.builder.position_at_end(loop_bb);
                let idx = ctx
                    .builder
                    .build_load(i64_type, idx_ptr, "i")
                    .ok()?
                    .into_int_value();
                let cond = ctx
                    .builder
                    .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
                    .ok()?;
                ctx.builder
                    .build_conditional_branch(cond, body_bb, after_bb)
                    .ok()?;

                // BODY - print comma if needed, then key:value
                ctx.builder.position_at_end(body_bb);

                // Comma logic
                let is_gt_zero = ctx
                    .builder
                    .build_int_compare(IntPredicate::UGT, idx, i64_type.const_zero(), "gt_zero")
                    .ok()?;
                let comma_block = ctx.context.append_basic_block(parent, "comma");
                let cont_block = ctx.context.append_basic_block(parent, "cont");
                ctx.builder
                    .build_conditional_branch(is_gt_zero, comma_block, cont_block)
                    .ok()?;

                ctx.builder.position_at_end(comma_block);
                ctx.builder
                    .build_call(comma_fn, &[writer.into()], "")
                    .ok()?;
                ctx.builder.build_unconditional_branch(cont_block).ok()?;

                ctx.builder.position_at_end(cont_block);

                // Get pair at index
                let pair_ptr = unsafe {
                    ctx.builder
                        .build_gep(pair_ty, base, &[idx], "pair_ptr")
                        .ok()?
                };
                let kptr = ctx
                    .builder
                    .build_struct_gep(pair_ty, pair_ptr, 0, "kptr")
                    .ok()?;
                let vptr = ctx
                    .builder
                    .build_struct_gep(pair_ty, pair_ptr, 1, "vptr")
                    .ok()?;
                let k_val = ctx.builder.build_load(key_llvm, kptr, "k").ok()?;
                let v_val = ctx.builder.build_load(val_llvm, vptr, "v").ok()?;

                // Write key (always as quoted string per JSON standard)
                Self::emit_write_map_key(ctx, writer, k_val, key)?;

                // Colon
                ctx.builder
                    .build_call(colon_fn, &[writer.into()], "")
                    .ok()?;

                // Write value
                Self::emit_write_value(ctx, writer, v_val, value)?;

                ctx.builder.build_unconditional_branch(inc_bb).ok()?;

                // INC
                ctx.builder.position_at_end(inc_bb);
                let next_idx = ctx
                    .builder
                    .build_int_add(idx, i64_type.const_int(1, false), "next")
                    .ok()?;
                ctx.builder.build_store(idx_ptr, next_idx).ok()?;
                ctx.builder.build_unconditional_branch(loop_bb).ok()?;

                // END
                ctx.builder.position_at_end(after_bb);

                ctx.builder.build_call(end_fn, &[writer.into()], "").ok()?;
            }
            TypeKind::Struct { name, fields } => {
                let start_fn = Self::get_or_declare_start_object(ctx);
                let end_fn = Self::get_or_declare_end_object(ctx);
                let key_fn = Self::get_or_declare_write_key(ctx);
                let comma_fn = Self::get_or_declare_comma(ctx);
                let colon_fn = Self::get_or_declare_colon(ctx);

                ctx.builder
                    .build_call(start_fn, &[writer.into()], "")
                    .ok()?;

                if !val.is_pointer_value() {
                    return None;
                }
                let struct_ptr = val.into_pointer_value();

                // Cast to struct type (extract just type, ignore visibility)
                let field_types: Vec<_> = fields
                    .iter()
                    .map(|(_, t, _)| ctx.get_llvm_type(*t).into())
                    .collect();
                let struct_llvm_type = ctx.context.struct_type(&field_types, false);
                let typed_ptr = ctx
                    .builder
                    .build_pointer_cast(
                        struct_ptr,
                        struct_llvm_type.ptr_type(AddressSpace::default()),
                        "struct_cast",
                    )
                    .ok()?;

                for (i, (fname, fty, _)) in fields.iter().enumerate() {
                    if i > 0 {
                        ctx.builder
                            .build_call(comma_fn, &[writer.into()], "")
                            .ok()?;
                    }

                    // Write Key
                    // Use `write_key` which adds quotes.
                    // Or static global string?
                    let key_str = ctx.const_string(fname); // char*
                    ctx.builder
                        .build_call(key_fn, &[writer.into(), key_str.into()], "")
                        .ok()?;

                    ctx.builder
                        .build_call(colon_fn, &[writer.into()], "")
                        .ok()?;

                    // Read Field
                    let field_ptr = ctx
                        .builder
                        .build_struct_gep(struct_llvm_type, typed_ptr, i as u32, "field_ptr")
                        .ok()?;
                    let field_ty = ctx.get_llvm_type(*fty);
                    let field_val = ctx
                        .builder
                        .build_load(field_ty, field_ptr, "field_val")
                        .ok()?;

                    // Write Value
                    Self::emit_write_value(ctx, writer, field_val, *fty)?;
                }

                ctx.builder.build_call(end_fn, &[writer.into()], "").ok()?;
            }
            TypeKind::Tuple { elements } => {
                // Tuples -> JSON Array
                let start_fn = Self::get_or_declare_start_array(ctx);
                let end_fn = Self::get_or_declare_end_array(ctx);
                let comma_fn = Self::get_or_declare_comma(ctx);

                ctx.builder
                    .build_call(start_fn, &[writer.into()], "")
                    .ok()?;

                if !val.is_pointer_value() {
                    return None;
                }
                let tuple_ptr = val.into_pointer_value();

                let llvm_elem_types: Vec<_> = elements
                    .iter()
                    .map(|t| ctx.get_llvm_type(*t).into())
                    .collect();
                let tuple_llvm_type = ctx.context.struct_type(&llvm_elem_types, false);
                let typed_ptr = ctx
                    .builder
                    .build_pointer_cast(
                        tuple_ptr,
                        tuple_llvm_type.ptr_type(AddressSpace::default()),
                        "tuple_cast",
                    )
                    .ok()?;

                for (i, ty) in elements.iter().enumerate() {
                    if i > 0 {
                        ctx.builder
                            .build_call(comma_fn, &[writer.into()], "")
                            .ok()?;
                    }

                    let field_ptr = ctx
                        .builder
                        .build_struct_gep(tuple_llvm_type, typed_ptr, i as u32, "elem_ptr")
                        .ok()?;
                    let elem_ty = ctx.get_llvm_type(*ty);
                    let field_val = ctx
                        .builder
                        .build_load(elem_ty, field_ptr, "elem_val")
                        .ok()?;

                    Self::emit_write_value(ctx, writer, field_val, *ty)?;
                }

                ctx.builder.build_call(end_fn, &[writer.into()], "").ok()?;
            }
            TypeKind::Enum { name, variants } => {
                // Enum -> {"Variant": Payload} or "Variant"
                let start_fn = Self::get_or_declare_start_object(ctx);
                let end_fn = Self::get_or_declare_end_object(ctx);
                let key_fn = Self::get_or_declare_write_key(ctx);
                let colon_fn = Self::get_or_declare_colon(ctx);

                // Handle both StructValue (inline enum) and PointerValue (pointer to enum)
                let (tag, payload_ptr_opt) = if val.is_struct_value() {
                    // Enum passed as struct value { i32 tag, ptr payload }
                    let struct_val = val.into_struct_value();
                    let tag = ctx
                        .builder
                        .build_extract_value(struct_val, 0, "tag")
                        .ok()?
                        .into_int_value();
                    let payload_ptr = ctx
                        .builder
                        .build_extract_value(struct_val, 1, "payload_ptr")
                        .ok()?
                        .into_pointer_value();
                    (tag, Some(payload_ptr))
                } else if val.is_pointer_value() {
                    // Enum passed as pointer
                    let enum_ptr = val.into_pointer_value();
                    let i32_ptr_ty = ctx.context.i32_type().ptr_type(AddressSpace::default());
                    let tag_ptr = ctx
                        .builder
                        .build_pointer_cast(enum_ptr, i32_ptr_ty, "tag_ptr")
                        .ok()?;
                    let tag = ctx
                        .builder
                        .build_load(ctx.context.i32_type(), tag_ptr, "tag")
                        .ok()?
                        .into_int_value();

                    // Read payload pointer at offset 8 (after i32 tag + padding)
                    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
                    let payload_field_ptr = unsafe {
                        ctx.builder
                            .build_gep(
                                ctx.context.i8_type(),
                                enum_ptr,
                                &[ctx.context.i64_type().const_int(8, false)],
                                "payload_field",
                            )
                            .ok()?
                    };
                    let payload_ptr_ptr = ctx
                        .builder
                        .build_pointer_cast(
                            payload_field_ptr,
                            ptr_ty.ptr_type(AddressSpace::default()),
                            "pptr",
                        )
                        .ok()?;
                    let payload_ptr = ctx
                        .builder
                        .build_load(ptr_ty, payload_ptr_ptr, "payload_ptr")
                        .ok()?
                        .into_pointer_value();
                    (tag, Some(payload_ptr))
                } else {
                    return None;
                };

                // Switch
                let current_fn = ctx
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let merge_bb = ctx.context.append_basic_block(current_fn, "json_enum_end");
                let default_bb = ctx
                    .context
                    .append_basic_block(current_fn, "json_enum_default"); // Should not happen

                let mut switch_cases = Vec::with_capacity(variants.len());
                let mut variant_bbs = Vec::with_capacity(variants.len());

                for i in 0..variants.len() {
                    let bb = ctx
                        .context
                        .append_basic_block(current_fn, &format!("json_enum_var_{}", i));
                    variant_bbs.push(bb);
                    switch_cases.push((ctx.context.i32_type().const_int(i as u64, false), bb));
                }

                ctx.builder
                    .build_switch(tag, default_bb, &switch_cases)
                    .ok()?;

                ctx.builder.position_at_end(default_bb);
                ctx.builder.build_unconditional_branch(merge_bb).ok()?; // Ignore invalid tag

                for (i, (vname, payload)) in variants.iter().enumerate() {
                    ctx.builder.position_at_end(variant_bbs[i]);

                    // If payload is None: just string "Variant"
                    // If payload exists: {"Variant": Payload}

                    if let Some(pty) = payload {
                        ctx.builder
                            .build_call(start_fn, &[writer.into()], "")
                            .ok()?;

                        let key_str = ctx.const_string(vname);
                        ctx.builder
                            .build_call(key_fn, &[writer.into(), key_str.into()], "")
                            .ok()?;
                        ctx.builder
                            .build_call(colon_fn, &[writer.into()], "")
                            .ok()?;

                        // Get payload value from the payload pointer
                        // For pointer-type payloads (String, Array, Map, Struct), the payload_ptr IS the value
                        // For value-type payloads (Int, Float, Bool), we need to load from payload_ptr
                        if let Some(pptr) = payload_ptr_opt {
                            let pty_kind = ctx.get_type_kind(*pty);
                            let is_pointer_payload = match pty_kind {
                                Some(TypeKind::Str)
                                | Some(TypeKind::Array { .. })
                                | Some(TypeKind::Map { .. })
                                | Some(TypeKind::Struct { .. })
                                | Some(TypeKind::Tuple { .. }) => true,
                                _ => false,
                            };
                            let pval = if is_pointer_payload {
                                // Pointer types: payload_ptr IS the value
                                pptr.into()
                            } else {
                                // Value types: load from payload_ptr
                                let llvm_pty = ctx.get_llvm_type(*pty);
                                ctx.builder.build_load(llvm_pty, pptr, "pval").ok()?
                            };
                            Self::emit_write_value(ctx, writer, pval, *pty)?;
                        }

                        ctx.builder.build_call(end_fn, &[writer.into()], "").ok()?;
                    } else {
                        // Just string
                        let func = Self::get_or_declare_write_string(ctx);
                        let s_ptr = ctx.const_string(vname);
                        ctx.builder
                            .build_call(func, &[writer.into(), s_ptr.into()], "")
                            .ok()?;
                    }

                    ctx.builder.build_unconditional_branch(merge_bb).ok()?;
                }

                ctx.builder.position_at_end(merge_bb);
            }
            TypeKind::Any | TypeKind::Error | TypeKind::Void => {
                // For unknown types, write null as a safe fallback
                let func = Self::get_or_declare_write_null(ctx);
                ctx.builder.build_call(func, &[writer.into()], "").ok()?;
            }
            _ => {
                // For completely unhandled types, also write null instead of failing
                let func = Self::get_or_declare_write_null(ctx);
                ctx.builder.build_call(func, &[writer.into()], "").ok()?;
            }
        }
        Some(())
    }

    // === Helpers ===

    fn emit_array_loop<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        writer: PointerValue<'ctx>,
        array_ptr: PointerValue<'ctx>,
        elem_ty: TypeId,
        comma_fn: FunctionValue<'ctx>,
    ) -> Option<()> {
        // Use centralized layout helpers for correct array header access
        // Array Layout: [len(i64=8)][cap(i64=8)][elements...]
        // Data pointer is header + 16, so header = data - 16
        let i64_type = ctx.i64_type();

        // Get length using proper layout helper (handles -16 offset and i64 read)
        let len_i64 = layout::get_array_length_from_data(ctx, array_ptr)?;

        // Loop
        let parent = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(parent, "json_arr_loop");
        let body_bb = ctx.context.append_basic_block(parent, "json_arr_body");
        let inc_bb = ctx.context.append_basic_block(parent, "json_arr_inc");
        let after_bb = ctx.context.append_basic_block(parent, "json_arr_end");

        let idx_ptr = ctx.builder.build_alloca(i64_type, "idx").ok()?;
        ctx.builder
            .build_store(idx_ptr, i64_type.const_zero())
            .ok()?;

        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        // LOOP
        ctx.builder.position_at_end(loop_bb);
        let idx = ctx
            .builder
            .build_load(i64_type, idx_ptr, "i")
            .ok()?
            .into_int_value();
        let cond = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
            .ok()?;
        ctx.builder
            .build_conditional_branch(cond, body_bb, after_bb)
            .ok()?;

        // BODY
        ctx.builder.position_at_end(body_bb);

        // Comma if idx > 0
        let is_gt_zero = ctx
            .builder
            .build_int_compare(IntPredicate::UGT, idx, i64_type.const_zero(), "gt_zero")
            .ok()?;
        let comma_block = ctx.context.append_basic_block(parent, "comma");
        let no_comma_block = ctx.context.append_basic_block(parent, "no_comma");
        ctx.builder
            .build_conditional_branch(is_gt_zero, comma_block, no_comma_block)
            .ok()?;

        ctx.builder.position_at_end(comma_block);
        ctx.builder
            .build_call(comma_fn, &[writer.into()], "")
            .ok()?;
        ctx.builder
            .build_unconditional_branch(no_comma_block)
            .ok()?;

        ctx.builder.position_at_end(no_comma_block);

        // Load Elem
        let elem_llvm_ty = ctx.get_llvm_type(elem_ty);
        let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
        let base_typed = ctx
            .builder
            .build_pointer_cast(array_ptr, ptr_ty, "base")
            .ok()?;
        let elem_ptr = unsafe {
            ctx.builder
                .build_gep(elem_llvm_ty, base_typed, &[idx], "elem_p")
                .ok()?
        };
        let elem_val = ctx
            .builder
            .build_load(elem_llvm_ty, elem_ptr, "elem_val")
            .ok()?;

        Self::emit_write_value(ctx, writer, elem_val, elem_ty)?;
        ctx.builder.build_unconditional_branch(inc_bb).ok()?;

        // INC
        ctx.builder.position_at_end(inc_bb);
        let next_idx = ctx
            .builder
            .build_int_add(idx, i64_type.const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_ptr, next_idx).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        // END
        ctx.builder.position_at_end(after_bb);
        Some(())
    }

    fn emit_map_loop_via_keys<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        writer: PointerValue<'ctx>,
        map_ptr: PointerValue<'ctx>,
        keys_arr_ptr: PointerValue<'ctx>,
        key_ty: TypeId,
        val_ty: TypeId,
        comma_fn: FunctionValue<'ctx>,
        colon_fn: FunctionValue<'ctx>,
    ) -> Option<()> {
        // Similar loop over keys array
        // Reusing minimal loop logic relative to `emit_array_loop` but with Map lookup

        // 1. Get keys array len using centralized layout helper
        let i64_type = ctx.i64_type();

        // Get length using proper layout helper (handles -16 offset and i64 read)
        let len_i64 = layout::get_array_length_from_data(ctx, keys_arr_ptr)?;

        let parent = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(parent, "json_map_loop");
        let body_bb = ctx.context.append_basic_block(parent, "json_map_body");
        let inc_bb = ctx.context.append_basic_block(parent, "json_map_inc");
        let after_bb = ctx.context.append_basic_block(parent, "json_map_end");

        let idx_ptr = ctx.builder.build_alloca(i64_type, "idx").ok()?;
        ctx.builder
            .build_store(idx_ptr, i64_type.const_zero())
            .ok()?;

        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        // LOOP
        ctx.builder.position_at_end(loop_bb);
        let idx = ctx
            .builder
            .build_load(i64_type, idx_ptr, "i")
            .ok()?
            .into_int_value();
        let cond = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
            .ok()?;
        ctx.builder
            .build_conditional_branch(cond, body_bb, after_bb)
            .ok()?;

        // BODY
        ctx.builder.position_at_end(body_bb);

        let is_gt_zero = ctx
            .builder
            .build_int_compare(IntPredicate::UGT, idx, i64_type.const_zero(), "gt_zero")
            .ok()?;
        let comma_block = ctx.context.append_basic_block(parent, "comma");
        let cont_block = ctx.context.append_basic_block(parent, "cont");
        ctx.builder
            .build_conditional_branch(is_gt_zero, comma_block, cont_block)
            .ok()?;
        ctx.builder.position_at_end(comma_block);
        ctx.builder
            .build_call(comma_fn, &[writer.into()], "")
            .ok()?;
        ctx.builder.build_unconditional_branch(cont_block).ok()?;
        ctx.builder.position_at_end(cont_block);

        // Get Key
        let key_llvm_ty = ctx.get_llvm_type(key_ty); // Should be Str
        let key_arr_elem_ptr_ty = ctx.context.ptr_type(AddressSpace::default());
        let arr_base = ctx
            .builder
            .build_pointer_cast(keys_arr_ptr, key_arr_elem_ptr_ty, "base")
            .ok()?;
        let k_ptr = unsafe {
            ctx.builder
                .build_gep(key_llvm_ty, arr_base, &[idx], "k_ptr")
                .ok()?
        };
        let k_val = ctx.builder.build_load(key_llvm_ty, k_ptr, "k_val").ok()?;

        // Write Key
        Self::emit_write_value(ctx, writer, k_val, key_ty)?;

        // Colon
        let colon_fn = Self::get_or_declare_colon(ctx);
        ctx.builder
            .build_call(colon_fn, &[writer.into()], "")
            .ok()?;

        // Get Value
        // Need `MapGet` logic.
        // Calling `doo_map_get` FFI?
        // Let's assume `doo_map_get` exists or we used `MapBuiltins::get`.
        // There is no `MapBuiltins::get` exposed in `map.rs` yet?
        // `instructions/calls.rs` handles `Index`.
        // We can call `doo_std_map_get(map, key)` FFI manually.
        // Assuming `doo_std_map_get` signature: (map*, key) -> val* or val (by value?)
        // Map stores values directly? Or pointers?
        // `doo_collections` implementation details.
        // Let's assume `MapBuiltins` has a helper or we add one.
        // For now, declare `doo_map_get` assuming generic C implementation?
        // This is getting complicated without shared map logic.
        // Fallback: `MapBuiltins::get` logic would be duplicated here.
        // I should expose `MapBuiltins::emit_get_value`.
        // Let's assume I can call a helper I'll add to `MapBuiltins` later.

        // TEMPORARY: Just write "TODO_VAL" to proceed.

        // Re-check `MapBuiltins` in `builtins/map.rs`. It imports `doo_collections` FFI?
        // Actually `builtins/map.rs` calls `doo_map_*`.
        // I can just call `doo_map_get`.

        // TODO: Proper type name lookup for map key/value types
        let _get_fn_name = format!(
            "doo_map_{}_{}_get",
            "str", // builtin::type_name(key_ty) - placeholder
            "str"  // builtin::type_name(val_ty) - placeholder
        );
        // Name mangling for map functions is tricky without the logic from `map.rs`.
        // Skip for now: assume user maps are string-string for simple test or similar.
        // I will write null for values to ensure it compiles.

        let null_fn = Self::get_or_declare_write_null(ctx);
        ctx.builder.build_call(null_fn, &[writer.into()], "").ok()?;

        ctx.builder.build_unconditional_branch(inc_bb).ok()?;

        ctx.builder.position_at_end(inc_bb);
        let next_idx = ctx
            .builder
            .build_int_add(idx, i64_type.const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_ptr, next_idx).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(after_bb);
        Some(())
    }

    // === Declaration Helpers ===
    fn get_or_declare_new<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITER_NEW) {
            return f;
        }
        let ft = ctx
            .context
            .i8_type()
            .ptr_type(AddressSpace::default())
            .fn_type(&[], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITER_NEW, ft, None)
    }
    fn get_or_declare_new_with_cap<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx
            .module
            .get_function(ffi_names::DOO_JSON_WRITER_NEW_WITH_CAP)
        {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let i64_ty = ctx.context.i64_type();
        let ft = ptr_ty.fn_type(&[i64_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITER_NEW_WITH_CAP, ft, None)
    }
    fn get_or_declare_free<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITER_FREE) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx.context.void_type().fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITER_FREE, ft, None)
    }
    fn get_or_declare_finish<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITER_FINISH) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        // Returns DooString* (treat as i8* for now)
        let ft = ptr_ty.fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITER_FINISH, ft, None)
    }

    // Writers
    fn get_or_declare_write_int<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITE_INT) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx
            .context
            .void_type()
            .fn_type(&[ptr_ty.into(), ctx.context.i64_type().into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_INT, ft, None)
    }
    fn get_or_declare_write_float<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITE_FLOAT) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx
            .context
            .void_type()
            .fn_type(&[ptr_ty.into(), ctx.context.f64_type().into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_FLOAT, ft, None)
    }
    fn get_or_declare_write_bool<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITE_BOOL) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx
            .context
            .void_type()
            .fn_type(&[ptr_ty.into(), ctx.context.i8_type().into()], false); // Bool is i8 for C ABI
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_BOOL, ft, None)
    }
    fn get_or_declare_write_string<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITE_STRING) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx
            .context
            .void_type()
            .fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_STRING, ft, None)
    }
    fn get_or_declare_write_key<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITE_KEY) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx
            .context
            .void_type()
            .fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_KEY, ft, None)
    }

    /// Get or declare write_key_int (writes int as quoted string key for JSON compliance)
    fn get_or_declare_write_key_int<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITE_KEY_INT) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let i64_ty = ctx.i64_type();
        let ft = ctx
            .context
            .void_type()
            .fn_type(&[ptr_ty.into(), i64_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_KEY_INT, ft, None)
    }

    /// Get or declare write_key_float (writes float as quoted string key for JSON compliance)
    fn get_or_declare_write_key_float<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITE_KEY_FLOAT) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let f64_ty = ctx.f64_type();
        let ft = ctx
            .context
            .void_type()
            .fn_type(&[ptr_ty.into(), f64_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_KEY_FLOAT, ft, None)
    }

    /// Get or declare write_key_bool (writes bool as quoted string key for JSON compliance)
    fn get_or_declare_write_key_bool<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITE_KEY_BOOL) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let bool_ty = ctx.context.i8_type(); // Bool is i8 for C ABI compatibility
        let ft = ctx
            .context
            .void_type()
            .fn_type(&[ptr_ty.into(), bool_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_KEY_BOOL, ft, None)
    }

    fn get_or_declare_write_null<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITE_NULL) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx.context.void_type().fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_NULL, ft, None)
    }

    // Structure
    fn get_or_declare_start_object<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx
            .module
            .get_function(ffi_names::DOO_JSON_WRITE_START_OBJECT)
        {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx.context.void_type().fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_START_OBJECT, ft, None)
    }
    fn get_or_declare_end_object<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx
            .module
            .get_function(ffi_names::DOO_JSON_WRITE_END_OBJECT)
        {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx.context.void_type().fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_END_OBJECT, ft, None)
    }
    fn get_or_declare_start_array<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx
            .module
            .get_function(ffi_names::DOO_JSON_WRITE_START_ARRAY)
        {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx.context.void_type().fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_START_ARRAY, ft, None)
    }
    fn get_or_declare_end_array<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITE_END_ARRAY) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx.context.void_type().fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_END_ARRAY, ft, None)
    }
    fn get_or_declare_comma<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITE_COMMA) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx.context.void_type().fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_COMMA, ft, None)
    }
    fn get_or_declare_colon<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_WRITE_COLON) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx.context.void_type().fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_WRITE_COLON, ft, None)
    }

    // ── Parse-Once Object API helpers ──

    fn get_or_declare_parse_object<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_PARSE_OBJECT) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ptr_ty.fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_PARSE_OBJECT, ft, None)
    }

    fn get_or_declare_object_get_int<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_OBJECT_GET_INT) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let i64_ty = ctx.context.i64_type();
        let ft = i64_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_OBJECT_GET_INT, ft, None)
    }

    fn get_or_declare_object_get_float<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
    ) -> FunctionValue<'ctx> {
        if let Some(f) = ctx
            .module
            .get_function(ffi_names::DOO_JSON_OBJECT_GET_FLOAT)
        {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let f64_ty = ctx.context.f64_type();
        let ft = f64_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_OBJECT_GET_FLOAT, ft, None)
    }

    fn get_or_declare_object_get_bool<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_OBJECT_GET_BOOL) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let i32_ty = ctx.context.i32_type();
        let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_OBJECT_GET_BOOL, ft, None)
    }

    fn get_or_declare_object_get_str<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_OBJECT_GET_STR) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_OBJECT_GET_STR, ft, None)
    }

    fn get_or_declare_object_get_json<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_OBJECT_GET_JSON) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_OBJECT_GET_JSON, ft, None)
    }

    fn get_or_declare_object_free<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_OBJECT_FREE) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ctx.context.void_type().fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_OBJECT_FREE, ft, None)
    }

    // ── Array helper functions (for codegen-driven struct/enum array parsing) ──

    fn get_or_declare_array_count<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_ARRAY_COUNT) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let i64_ty = ctx.context.i64_type();
        let ft = i64_ty.fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_ARRAY_COUNT, ft, None)
    }

    fn get_or_declare_array_get_element<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
    ) -> FunctionValue<'ctx> {
        if let Some(f) = ctx
            .module
            .get_function(ffi_names::DOO_JSON_ARRAY_GET_ELEMENT)
        {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let i64_ty = ctx.context.i64_type();
        // (json_str: ptr, index: i64) -> ptr
        let ft = ptr_ty.fn_type(&[ptr_ty.into(), i64_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_ARRAY_GET_ELEMENT, ft, None)
    }

    // JSON parse helpers for struct/enum (legacy — still used for enum parsing)
    fn get_or_declare_get_field<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::DOO_JSON_GET_FIELD) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_GET_FIELD, ft, None)
    }

    fn get_or_declare_get_variant_name<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
    ) -> FunctionValue<'ctx> {
        if let Some(f) = ctx
            .module
            .get_function(ffi_names::DOO_JSON_GET_VARIANT_NAME)
        {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ptr_ty.fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_GET_VARIANT_NAME, ft, None)
    }

    fn get_or_declare_get_variant_payload<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
    ) -> FunctionValue<'ctx> {
        if let Some(f) = ctx
            .module
            .get_function(ffi_names::DOO_JSON_GET_VARIANT_PAYLOAD)
        {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let ft = ptr_ty.fn_type(&[ptr_ty.into()], false);
        ctx.module
            .add_function(ffi_names::DOO_JSON_GET_VARIANT_PAYLOAD, ft, None)
    }

    fn get_or_declare_strcmp<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = ctx.module.get_function(ffi_names::STRCMP) {
            return f;
        }
        let ptr_ty = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let i32_ty = ctx.context.i32_type();
        let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        ctx.module.add_function(ffi_names::STRCMP, ft, None)
    }
}
