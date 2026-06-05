//! Metadata registration and error helpers for HTTP handler codegen.

use crate::context::CodegenContext;
use crate::packages::http as http_pkg;
use doo_core::doo_debug;
use doo_core::types::TypeKind;
use doo_mir::{MirConst, MirOperand};
use inkwell::values::FunctionValue;

/// Emit a call to doo_http_register_handler_with_metadata to register handler metadata.
/// This is called when an HTTP route is registered, allowing the FFI to validate
/// request bodies against the expected struct types.
pub(crate) fn emit_handler_metadata_registration<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    handler_name: &str,
    wrapper_fn: &FunctionValue<'ctx>,
) {
    let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();

    // Build metadata JSON from function parameter types
    let metadata_json = build_handler_metadata_json(ctx, handler_name);

    if debug {}

    // Get or declare doo_http_register_handler_with_metadata
    let void_type = ctx.context.void_type();
    let ptr_type = ctx.ptr_type();
    let fn_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);

    let register_fn = ctx
        .module
        .get_function(http_pkg::DOO_HTTP_REGISTER_HANDLER_WITH_METADATA)
        .unwrap_or_else(|| {
            ctx.module.add_function(
                http_pkg::DOO_HTTP_REGISTER_HANDLER_WITH_METADATA,
                fn_type,
                None,
            )
        });

    // Create string constants for handler name and metadata
    let handler_name_ptr = ctx.const_string(handler_name);
    let metadata_ptr = ctx.const_string(&metadata_json);

    // Get wrapper function pointer
    let wrapper_ptr = wrapper_fn.as_global_value().as_pointer_value();

    // Call doo_http_register_handler_with_metadata(name, wrapper_ptr, metadata_json)
    let _ = ctx.builder.build_call(
        register_fn,
        &[
            handler_name_ptr.into(),
            wrapper_ptr.into(),
            metadata_ptr.into(),
        ],
        "register_handler_meta",
    );

    // Register struct metadata (including decorators) for all parameter types
    // This is needed for decorator validation in doohttp_populate_struct_from_request
    let param_type_ids: Vec<doo_core::types::TypeId> = ctx
        .get_function_param_types(handler_name)
        .map(|v| v.iter().copied().collect())
        .unwrap_or_default();

    for type_id in param_type_ids {
        emit_struct_metadata_if_needed(ctx, type_id);
    }

    // Also register return type struct metadata (for @internal/@writeOnly response filtering)
    if let Some(ret_type_id) = ctx.get_function_return_type(handler_name) {
        emit_struct_metadata_if_needed(ctx, ret_type_id);
    }
}

/// Emit struct/enum metadata registration for auth/crud calls.
/// When app.auth() or app.crud() is called with a struct type, we need to register
/// the struct's field layout and any enum types it references so the FFI can
/// validate incoming requests at runtime.
pub(crate) fn emit_struct_metadata_registration_for_auth_crud<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    symbol: &str,
    args: &[MirOperand],
) {
    let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();

    // Find the struct name argument by scanning for a string constant
    // that resolves to a known struct type in the registry, instead of
    // hardcoding the argument index per FFI function.
    let struct_name = args.iter().find_map(|arg| {
        if let MirOperand::Const(MirConst::Str(name)) = arg {
            // Check if this string is a known struct type
            if ctx.type_registry.lookup(name).is_some() {
                return Some(name.clone());
            }
        }
        None
    });

    let struct_name = match struct_name {
        Some(name) => name,
        None => return, // No struct name found in arguments
    };

    if debug {}

    // Look up the struct in the type registry
    let struct_type_id = match ctx.type_registry.lookup(&struct_name) {
        Some(id) => id,
        None => return,
    };

    // Build struct metadata JSON
    let struct_metadata = build_struct_metadata_json(ctx, struct_type_id);

    // Get or declare doo_http_register_struct_metadata
    let void_type = ctx.context.void_type();
    let ptr_type = ctx.ptr_type();
    let fn_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);

    let register_fn = ctx
        .module
        .get_function(http_pkg::DOO_HTTP_REGISTER_STRUCT_METADATA)
        .unwrap_or_else(|| {
            ctx.module
                .add_function(http_pkg::DOO_HTTP_REGISTER_STRUCT_METADATA, fn_type, None)
        });

    // Create string constants
    let struct_name_ptr = ctx.const_string(&struct_name);
    let metadata_ptr = ctx.const_string(&struct_metadata);

    // Call doo_http_register_struct_metadata(name, metadata_json)
    let _ = ctx.builder.build_call(
        register_fn,
        &[struct_name_ptr.into(), metadata_ptr.into()],
        "register_struct_meta",
    );

    // Collect field type IDs first to avoid borrow conflict
    let field_type_ids: Vec<doo_core::types::TypeId> = ctx
        .type_registry
        .get(struct_type_id)
        .and_then(|info| {
            if let TypeKind::Struct { fields, .. } = &info.kind {
                Some(fields.iter().map(|(_, tid, _)| *tid).collect())
            } else {
                None
            }
        })
        .unwrap_or_default();

    // Register any enum types referenced by this struct
    for field_type_id in field_type_ids {
        emit_enum_metadata_if_needed(ctx, field_type_id);
    }
}

/// Emit enum metadata registration if the type is an enum.
pub(crate) fn emit_enum_metadata_if_needed<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    type_id: doo_core::types::TypeId,
) {
    let type_info = match ctx.type_registry.get(type_id) {
        Some(info) => info,
        None => return,
    };

    if let TypeKind::Enum { name, variants, .. } = &type_info.kind {
        let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();

        if debug {}

        // Build variants JSON array
        let variant_names: Vec<&str> = variants.iter().map(|(name, _)| name.as_str()).collect();
        let variants_json =
            serde_json::to_string(&variant_names).unwrap_or_else(|_| "[]".to_string());

        // Get or declare doo_http_register_enum_metadata
        let void_type = ctx.context.void_type();
        let ptr_type = ctx.ptr_type();
        let fn_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);

        let register_fn = ctx
            .module
            .get_function(http_pkg::DOO_HTTP_REGISTER_ENUM_METADATA)
            .unwrap_or_else(|| {
                ctx.module
                    .add_function(http_pkg::DOO_HTTP_REGISTER_ENUM_METADATA, fn_type, None)
            });

        // Create string constants
        let enum_name_ptr = ctx.const_string(name);
        let variants_ptr = ctx.const_string(&variants_json);

        // Call doo_http_register_enum_metadata(name, variants_json)
        let _ = ctx.builder.build_call(
            register_fn,
            &[enum_name_ptr.into(), variants_ptr.into()],
            "register_enum_meta",
        );
    }
}

/// Emit struct metadata registration if the type is a struct.
/// Also registers any enum types referenced by the struct fields.
pub(crate) fn emit_struct_metadata_if_needed<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    type_id: doo_core::types::TypeId,
) {
    use doo_core::types::TypeKind;

    let type_info = match ctx.type_registry.get(type_id) {
        Some(info) => info,
        None => return,
    };

    if let TypeKind::Struct { name, fields, .. } = &type_info.kind {
        // Build struct metadata JSON (includes decorators from MIR)
        let struct_metadata = build_struct_metadata_json(ctx, type_id);

        // Get or declare doo_http_register_struct_metadata
        let void_type = ctx.context.void_type();
        let ptr_type = ctx.ptr_type();
        let fn_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);

        let register_fn = ctx
            .module
            .get_function(http_pkg::DOO_HTTP_REGISTER_STRUCT_METADATA)
            .unwrap_or_else(|| {
                ctx.module
                    .add_function(http_pkg::DOO_HTTP_REGISTER_STRUCT_METADATA, fn_type, None)
            });

        // Create string constants
        let struct_name_ptr = ctx.const_string(name);
        let metadata_ptr = ctx.const_string(&struct_metadata);

        // Call doo_http_register_struct_metadata(name, metadata_json)
        let _ = ctx.builder.build_call(
            register_fn,
            &[struct_name_ptr.into(), metadata_ptr.into()],
            "register_struct_meta",
        );

        // Collect field type IDs before registering enum metadata
        let field_type_ids: Vec<doo_core::types::TypeId> =
            fields.iter().map(|(_, tid, _)| *tid).collect();

        // Register any enum types referenced by struct fields
        for field_type_id in field_type_ids {
            emit_enum_metadata_if_needed(ctx, field_type_id);
        }
    } else if let TypeKind::Enum { .. } = &type_info.kind {
        // If the parameter itself is an enum, register it
        emit_enum_metadata_if_needed(ctx, type_id);
    }
}

/// Build struct metadata JSON for a given type ID.
fn build_struct_metadata_json<'ctx>(
    ctx: &CodegenContext<'ctx>,
    type_id: doo_core::types::TypeId,
) -> String {
    let type_info = match ctx.type_registry.get(type_id) {
        Some(info) => info,
        None => return "{}".to_string(),
    };

    if let TypeKind::Struct { fields, name, .. } = &type_info.kind {
        // Look up field decorators from MIR data stored on context
        let field_decorators = ctx.struct_field_decorators.get(name.as_str());

        let field_list: Vec<serde_json::Value> = fields
            .iter()
            .map(|(fname, field_type_id, _is_public)| {
                let type_name = type_id_to_string_inner(&ctx.type_registry, *field_type_id);

                // Look up decorators from the MIR-populated map
                let decorators: Vec<String> = field_decorators
                    .and_then(|fds| {
                        fds.iter()
                            .find(|(n, _)| n == fname)
                            .map(|(_, decs)| decs.clone())
                    })
                    .unwrap_or_default();

                serde_json::json!({
                    "name": fname,
                    "type": type_name,
                    "decorators": decorators
                })
            })
            .collect();

        serde_json::to_string(&serde_json::json!({
            "fields": field_list
        }))
        .unwrap_or_else(|_| "{}".to_string())
    } else {
        "{}".to_string()
    }
}

/// Build metadata JSON string for a handler function.
/// Format: {"param_count":N,"param_types":["TypeName"],"struct_layouts":{...}}
fn build_handler_metadata_json<'ctx>(ctx: &CodegenContext<'ctx>, func_name: &str) -> String {
    use doo_core::types::{TypeId, TypeKind, TypeRegistry};
    use std::collections::HashMap;

    // Get function parameter types
    let param_type_ids = ctx.get_function_param_types(func_name);
    let param_count = param_type_ids.map(|v| v.len()).unwrap_or(0);

    let mut param_types: Vec<String> = Vec::new();
    let mut struct_layouts: HashMap<String, serde_json::Value> = HashMap::new();
    let mut enum_variants: HashMap<String, Vec<String>> = HashMap::new();

    // Helper to collect enums referenced by a type
    fn collect_enums_from_type(
        registry: &TypeRegistry,
        type_id: TypeId,
        enum_variants: &mut HashMap<String, Vec<String>>,
    ) {
        if let Some(type_info) = registry.get(type_id) {
            match &type_info.kind {
                TypeKind::Enum { name, variants, .. } => {
                    if !enum_variants.contains_key(name) {
                        // Extract just the variant names from (String, Option<TypeId>)
                        let variant_names: Vec<String> = variants
                            .iter()
                            .map(|(variant_name, _)| variant_name.clone())
                            .collect();
                        enum_variants.insert(name.clone(), variant_names);
                    }
                }
                TypeKind::Array { element } => {
                    collect_enums_from_type(registry, *element, enum_variants);
                }
                TypeKind::Optional { inner } => {
                    collect_enums_from_type(registry, *inner, enum_variants);
                }
                TypeKind::Map { key, value } => {
                    collect_enums_from_type(registry, *key, enum_variants);
                    collect_enums_from_type(registry, *value, enum_variants);
                }
                _ => {}
            }
        }
    }

    // Helper to collect struct layouts recursively
    fn collect_struct_layout(
        registry: &TypeRegistry,
        type_id: TypeId,
        struct_layouts: &mut HashMap<String, serde_json::Value>,
        enum_variants: &mut HashMap<String, Vec<String>>,
        field_decorators_map: &rustc_hash::FxHashMap<String, Vec<(String, Vec<String>)>>,
    ) {
        if let Some(type_info) = registry.get(type_id) {
            match &type_info.kind {
                TypeKind::Struct { name, fields, .. } => {
                    // Skip if already collected
                    if struct_layouts.contains_key(name) {
                        return;
                    }

                    let mut field_list: Vec<serde_json::Value> = Vec::new();
                    let field_count = fields.len();

                    // Look up decorators for this struct from the MIR-populated map
                    let struct_decorators = field_decorators_map.get(name.as_str());

                    // Calculate field size and alignment based on LLVM type mapping
                    // CRITICAL: Must match get_llvm_type() in context.rs
                    // - Int -> i64 (8 bytes), NOT i32
                    // - Float -> f64 (8 bytes)
                    // - Bool -> i1 (1 byte)
                    // - Str/ptr -> ptr (8 bytes on 64-bit)
                    let field_size_align: Vec<(u64, u64)> = fields
                        .iter()
                        .map(|(_, field_type_id, _)| {
                            match registry.get(*field_type_id).map(|t| &t.kind) {
                                Some(TypeKind::Int) => (8u64, 8u64),       // i64
                                Some(TypeKind::Float) => (8, 8),           // f64
                                Some(TypeKind::Bool) => (1, 1),            // i1
                                Some(TypeKind::Str) => (8, 8),             // pointer
                                Some(TypeKind::Array { .. }) => (8, 8),    // pointer to array
                                Some(TypeKind::Map { .. }) => (8, 8),      // pointer to map
                                Some(TypeKind::Struct { .. }) => (8, 8),   // pointer to struct
                                Some(TypeKind::Optional { .. }) => (8, 8), // pointer
                                Some(TypeKind::Enum { .. }) => (16, 8), // { i32, ptr } = 16 bytes
                                _ => (8, 8),                            // default to pointer size
                            }
                        })
                        .collect();

                    // Apply P06 field reordering: sort by alignment descending (stable sort)
                    // MUST match type_cache.rs create_struct_type() reordering exactly
                    let mut sorted_indices: Vec<usize> = (0..field_count).collect();
                    sorted_indices
                        .sort_by(|&a, &b| field_size_align[b].1.cmp(&field_size_align[a].1));

                    // Compute physical offsets using the sorted (physical) order
                    let mut physical_offsets = vec![0u64; field_count];
                    let mut current_offset: u64 = 0;
                    for &logical_idx in &sorted_indices {
                        let (field_size, field_align) = field_size_align[logical_idx];
                        // Align current offset to field's alignment
                        if field_align > 0 && current_offset % field_align != 0 {
                            current_offset = ((current_offset / field_align) + 1) * field_align;
                        }
                        physical_offsets[logical_idx] = current_offset;
                        current_offset += field_size;
                    }

                    // Build field list in LOGICAL order with correct PHYSICAL offsets
                    for (i, (field_name, field_type_id, _)) in fields.iter().enumerate() {
                        let field_type_name = type_id_to_string_inner(registry, *field_type_id);

                        // Look up decorators for this field (single source of truth from MIR)
                        let decorators: Vec<String> = struct_decorators
                            .and_then(|fds| {
                                fds.iter()
                                    .find(|(n, _)| n == field_name)
                                    .map(|(_, decs)| decs.clone())
                            })
                            .unwrap_or_default();

                        field_list.push(serde_json::json!({
                            "name": field_name,
                            "type": field_type_name,
                            "offset": physical_offsets[i],
                            "decorators": decorators
                        }));

                        // Recursively collect nested structs and enums
                        collect_struct_layout(
                            registry,
                            *field_type_id,
                            struct_layouts,
                            enum_variants,
                            field_decorators_map,
                        );
                        collect_enums_from_type(registry, *field_type_id, enum_variants);
                    }
                    struct_layouts.insert(
                        name.clone(),
                        serde_json::json!({
                            "fields": field_list
                        }),
                    );
                }
                TypeKind::Array { element } => {
                    collect_struct_layout(
                        registry,
                        *element,
                        struct_layouts,
                        enum_variants,
                        field_decorators_map,
                    );
                }
                TypeKind::Optional { inner } => {
                    collect_struct_layout(
                        registry,
                        *inner,
                        struct_layouts,
                        enum_variants,
                        field_decorators_map,
                    );
                }
                TypeKind::Map { key, value } => {
                    collect_struct_layout(
                        registry,
                        *key,
                        struct_layouts,
                        enum_variants,
                        field_decorators_map,
                    );
                    collect_struct_layout(
                        registry,
                        *value,
                        struct_layouts,
                        enum_variants,
                        field_decorators_map,
                    );
                }
                _ => {}
            }
        }
    }

    if let Some(type_ids) = param_type_ids {
        for type_id in type_ids {
            // Recursively collect all struct layouts and enum variants
            collect_struct_layout(
                &ctx.type_registry,
                *type_id,
                &mut struct_layouts,
                &mut enum_variants,
                &ctx.struct_field_decorators,
            );

            // Get type name from type registry
            if let Some(type_info) = ctx.type_registry.get(*type_id) {
                let type_name = match &type_info.kind {
                    TypeKind::Struct { name, .. } => name.clone(),
                    _ => type_id_to_string_inner(&ctx.type_registry, *type_id),
                };
                param_types.push(type_name);
            }
        }
    }

    // Get return type and include it in metadata
    let mut return_type = "Void".to_string();
    if let Some(ret_type_id) = ctx.get_function_return_type(func_name) {
        // Collect struct layouts for return type
        collect_struct_layout(
            &ctx.type_registry,
            ret_type_id,
            &mut struct_layouts,
            &mut enum_variants,
            &ctx.struct_field_decorators,
        );
        return_type = type_id_to_string_inner(&ctx.type_registry, ret_type_id);
    }

    // Build JSON
    let metadata = serde_json::json!({
        "param_count": param_count,
        "param_types": param_types,
        "return_type": return_type,
        "struct_layouts": struct_layouts,
        "enum_variants": enum_variants
    });

    metadata.to_string()
}

/// Convert TypeId to a string representation for metadata, resolving nested types
fn type_id_to_string_inner(
    registry: &doo_core::types::TypeRegistry,
    type_id: doo_core::types::TypeId,
) -> String {
    use doo_core::types::TypeKind;

    if let Some(type_info) = registry.get(type_id) {
        match &type_info.kind {
            TypeKind::Int => "Int".to_string(),
            TypeKind::Float => "Float".to_string(),
            TypeKind::Bool => "Bool".to_string(),
            TypeKind::Str => "Str".to_string(),
            TypeKind::Void => "Void".to_string(),
            TypeKind::Array { element } => {
                let elem_str = type_id_to_string_inner(registry, *element);
                format!("[{}]", elem_str)
            }
            TypeKind::Optional { inner } => {
                let inner_str = type_id_to_string_inner(registry, *inner);
                format!("Optional({})", inner_str)
            }
            TypeKind::Struct { name, .. } => name.clone(),
            TypeKind::Enum { name, .. } => name.clone(),
            TypeKind::Interface { name, .. } => name.clone(),
            TypeKind::Function { .. } => "Function".to_string(),
            TypeKind::Map { key, value } => {
                let key_str = type_id_to_string_inner(registry, *key);
                let value_str = type_id_to_string_inner(registry, *value);
                format!("Map<{},{}>", key_str, value_str)
            }
            TypeKind::Result { ok, err } => {
                let ok_str = type_id_to_string_inner(registry, *ok);
                let err_str = type_id_to_string_inner(registry, *err);
                format!("Result<{},{}>", ok_str, err_str)
            }
            TypeKind::Tuple { elements } => {
                let elem_strs: Vec<String> = elements
                    .iter()
                    .map(|e| type_id_to_string_inner(registry, *e))
                    .collect();
                format!("({})", elem_strs.join(","))
            }
            TypeKind::TypeRef { name } => name.clone(),
            TypeKind::Any => "Any".to_string(),
            TypeKind::Error => "Error".to_string(),
            TypeKind::TypeParam { name } => name.clone(),
        }
    } else {
        "Unknown".to_string()
    }
}

/// Emit RBAC policy metadata registration if a policy is registered for
/// the struct referenced in the current auth/crud call.
///
/// Called from `http::pre_call` after struct metadata emission.
pub(crate) fn emit_policy_metadata_if_present<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    args: &[MirOperand],
) {
    // Find the struct name from arguments (same logic as struct metadata emission)
    let struct_name = args.iter().find_map(|arg| {
        if let MirOperand::Const(MirConst::Str(name)) = arg {
            if ctx.type_registry.lookup(name).is_some() {
                return Some(name.clone());
            }
        }
        None
    });

    let struct_name = match struct_name {
        Some(name) => name,
        None => return,
    };

    // Look up the policy JSON for this struct
    if let Some(policy_json) = ctx.rbac_policies.get(&struct_name).cloned() {
        let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();
        if debug {}

        // Get or declare doo_http_register_policy(name: *const c_char, policy_json: *const c_char)
        let void_type = ctx.context.void_type();
        let ptr_type = ctx.ptr_type();
        let fn_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);

        let register_fn = ctx
            .module
            .get_function(http_pkg::DOO_HTTP_REGISTER_POLICY)
            .unwrap_or_else(|| {
                ctx.module
                    .add_function(http_pkg::DOO_HTTP_REGISTER_POLICY, fn_type, None)
            });

        let struct_name_ptr = ctx.const_string(&struct_name);
        let policy_ptr = ctx.const_string(&policy_json);

        let _ = ctx.builder.build_call(
            register_fn,
            &[struct_name_ptr.into(), policy_ptr.into()],
            "register_policy",
        );
    }

    // Always emit role hierarchy for enum types referenced by this struct
    // (e.g., User struct has Role field with @role/@inherits — needed for RBAC)
    emit_role_hierarchy_for_struct(ctx, &struct_name);
}

/// Emit role hierarchy registration for all enum fields in a struct that
/// have `@inherits` data recorded in `ctx.enum_inheritance`.
fn emit_role_hierarchy_for_struct<'ctx>(ctx: &mut CodegenContext<'ctx>, struct_name: &str) {
    // Find the struct type
    let struct_type_id = match ctx.type_registry.lookup(struct_name) {
        Some(id) => id,
        None => return,
    };

    // Collect field types
    let field_type_ids: Vec<doo_core::types::TypeId> = ctx
        .type_registry
        .get(struct_type_id)
        .and_then(|info| {
            if let doo_core::types::TypeKind::Struct { fields, .. } = &info.kind {
                Some(fields.iter().map(|(_, tid, _)| *tid).collect())
            } else {
                None
            }
        })
        .unwrap_or_default();

    for field_type_id in field_type_ids {
        let type_info = match ctx.type_registry.get(field_type_id) {
            Some(info) => info,
            None => continue,
        };

        if let doo_core::types::TypeKind::Enum { name, .. } = &type_info.kind {
            let enum_name = name.clone();
            // Check if we have inheritance data for this enum
            if let Some(inheritance) = ctx.enum_inheritance.get(&enum_name).cloned() {
                emit_role_hierarchy_for_enum(ctx, &enum_name, &inheritance);
            }
        }
    }
}

/// Emit `doo_http_register_role_hierarchy(enum_name, hierarchy_json)` for an enum.
///
/// `inheritance` is `Vec<(variant_name, Vec<inherited_variants>)>` from `ctx.enum_inheritance`.
/// The emitted JSON format is: `{"Admin":["Moderator","User"],"Moderator":["User"]}`
fn emit_role_hierarchy_for_enum<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    enum_name: &str,
    inheritance: &[(String, Vec<String>)],
) {
    let mut map = serde_json::Map::new();
    for (variant, inherited) in inheritance {
        let arr: Vec<serde_json::Value> = inherited
            .iter()
            .map(|s| serde_json::Value::String(s.clone()))
            .collect();
        map.insert(variant.clone(), serde_json::Value::Array(arr));
    }
    let hierarchy_json =
        serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or_else(|_| "{}".to_string());

    let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();
    if debug {}

    let void_type = ctx.context.void_type();
    let ptr_type = ctx.ptr_type();
    let fn_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);

    let register_fn = ctx
        .module
        .get_function("doo_http_register_role_hierarchy")
        .unwrap_or_else(|| {
            ctx.module
                .add_function("doo_http_register_role_hierarchy", fn_type, None)
        });

    let enum_name_ptr = ctx.const_string(enum_name);
    let hierarchy_ptr = ctx.const_string(&hierarchy_json);

    let _ = ctx.builder.build_call(
        register_fn,
        &[enum_name_ptr.into(), hierarchy_ptr.into()],
        "register_role_hierarchy",
    );
}
