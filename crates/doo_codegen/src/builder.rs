//! Codegen Builder - Main entry point for code generation
//!
//! Orchestrates MIR -> LLVM IR translation.

use crate::context::CodegenContext;
use crate::instructions::InstructionDispatcher;
use crate::utils::operand_to_value;
use doo_core::constants::ffi_names::{self};
use doo_core::types::{builtin, TypeKind, TypeRegistry};
use doo_mir::sym::resolve;
use doo_mir::{MirConst, MirFunction, MirGlobal, MirOperand, MirProgram, MirTerminator};
use inkwell::basic_block::BasicBlock;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::BasicValueEnum;
use inkwell::IntPredicate;
use std::collections::HashMap;
use std::sync::Arc;

/// Get the variable name from a MirOperand.
fn get_operand_name(operand: &MirOperand) -> Option<String> {
    match operand {
        MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
            Some(resolve(*name))
        }
        MirOperand::Const(_) => None,
        MirOperand::FuncRef(name) => Some(resolve(*name)),
    }
}

/// Convert a return value to the expected function return type.
/// Handles type mismatches between computed value and declared return type.
fn convert_return_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
) -> BasicValueEnum<'ctx> {
    use doo_core::types::TypeKind;

    // Get expected return type from context
    let expected_type = match ctx.current_function_return_type {
        Some(t) => t,
        None => return val, // No conversion needed
    };

    // Get the expected LLVM type
    let expected_llvm_type = ctx.get_llvm_type(expected_type);

    // Get the expected type kind for semantic decisions
    let expected_kind = ctx.get_type_kind(expected_type);

    // Handle wrapping raw value into Ok Result struct.
    // MUST happen BEFORE the type-match early return below.
    // When a function has an error type (returns T ! E), its LLVM signature is { i64, ptr }.
    // `return value` where value: T (e.g. ptr for [Task]) must be wrapped into { 0, ptr }.
    // Note: expected_llvm_type here is T (not Result), so ptr==ptr would trigger early return.
    if ctx.current_function_has_error_type {
        let result_struct_type = ctx.context.struct_type(
            &[
                ctx.context.i64_type().into(),
                ctx.context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into(),
            ],
            false,
        );
        // Only wrap if the value is NOT already a Result struct
        if val.get_type() != BasicTypeEnum::StructType(result_struct_type) {
            let payload_ptr = if val.is_pointer_value() {
                val.into_pointer_value()
            } else {
                // Non-pointer value (Int, Bool, etc.): box it on the heap first
                let ptr_type = ctx.context.ptr_type(inkwell::AddressSpace::default());
                let malloc_fn = ctx.get_function(ffi_names::MALLOC).unwrap_or_else(|| {
                    let fn_type = ptr_type.fn_type(&[ctx.context.i64_type().into()], false);
                    ctx.module.add_function(
                        ffi_names::MALLOC,
                        fn_type,
                        Some(inkwell::module::Linkage::External),
                    )
                });
                let size = ctx.context.i64_type().const_int(8, false);
                if let Some(alloc) = ctx
                    .builder
                    .build_call(malloc_fn, &[size.into()], "ok_box")
                    .ok()
                    .and_then(|c| c.try_as_basic_value().basic())
                    .map(|v| v.into_pointer_value())
                {
                    ctx.builder.build_store(alloc, val).ok();
                    alloc
                } else {
                    ctx.context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                }
            };
            // Build { 0i64, payload_ptr }
            let ok_tag = ctx.context.i64_type().const_zero();
            let mut ok_struct = result_struct_type.get_undef();
            if let Ok(s) = ctx
                .builder
                .build_insert_value(ok_struct, ok_tag, 0, "ok_tag")
            {
                ok_struct = s.into_struct_value();
            }
            if let Ok(s) = ctx
                .builder
                .build_insert_value(ok_struct, payload_ptr, 1, "ok_payload")
            {
                return BasicValueEnum::StructValue(s.into_struct_value());
            }
            return BasicValueEnum::StructValue(result_struct_type.const_zero());
        }
    }

    // Check if types already match (no conversion needed)
    if val.get_type() == expected_llvm_type {
        return val;
    }

    // Handle i64/i32/i1 -> ptr conversion (when function expects pointer type)
    // SAFETY FIX: Validate the integer before inttoptr.
    if val.is_int_value() && expected_llvm_type.is_pointer_type() {
        let int_val = val.into_int_value();
        let ptr_type = expected_llvm_type.into_pointer_type();
        // Check: value > 0 (signed) — valid user-space pointer
        // This catches 0 (null) AND negative values like -1 (0xFFFF...FFFF)
        let is_positive = ctx.builder.build_int_compare(
            IntPredicate::SGT,
            int_val,
            int_val.get_type().const_zero(),
            "is_valid_addr",
        );
        if let Ok(is_valid) = is_positive {
            let null_ptr = ptr_type.const_null();
            if let Ok(as_ptr) = ctx
                .builder
                .build_int_to_ptr(int_val, ptr_type, "int_to_ptr")
            {
                if let Ok(result) =
                    ctx.builder
                        .build_select(is_valid, as_ptr, null_ptr, "safe_int_to_ptr")
                {
                    return result;
                }
            }
        }
        // Fallback: inttoptr (legacy path)
        if let Ok(ptr) = ctx.builder.build_int_to_ptr(
            int_val,
            expected_llvm_type.into_pointer_type(),
            "int_to_ptr_fallback",
        ) {
            return ptr.into();
        }
    }

    // Handle pointer -> primitive conversion (e.g., from JSON.parse)
    if val.is_pointer_value() {
        let ptr = val.into_pointer_value();

        if let Some(kind) = expected_kind {
            match kind {
                TypeKind::Int => {
                    // Load i64 from pointer
                    if let Ok(loaded) =
                        ctx.builder
                            .build_load(ctx.context.i64_type(), ptr, "ret_int")
                    {
                        return loaded;
                    }
                }
                TypeKind::Float32 | TypeKind::Float64 => {
                    // Load f64 from pointer
                    if let Ok(loaded) =
                        ctx.builder
                            .build_load(ctx.context.f64_type(), ptr, "ret_float")
                    {
                        return loaded;
                    }
                }
                TypeKind::Bool => {
                    // Load i8 from pointer (Bool uses i8 for C ABI compatibility)
                    if let Ok(loaded) =
                        ctx.builder
                            .build_load(ctx.context.i8_type(), ptr, "ret_bool")
                    {
                        return loaded;
                    }
                }
                _ => {
                    // For other types (Str, Array, Map, etc.), pointer is correct
                    return BasicValueEnum::PointerValue(ptr);
                }
            }
        }

        // If conversion failed, return original pointer
        return BasicValueEnum::PointerValue(ptr);
    }

    // Handle bool (i1) -> larger int conversion
    if val.is_int_value() && expected_llvm_type.is_int_type() {
        let int_val = val.into_int_value();
        let expected_int = expected_llvm_type.into_int_type();
        let val_bits = int_val.get_type().get_bit_width();
        let expected_bits = expected_int.get_bit_width();

        if val_bits < expected_bits {
            // Zero extend (e.g., i1 -> i64)
            if let Ok(zext) = ctx
                .builder
                .build_int_z_extend(int_val, expected_int, "zext_ret")
            {
                return zext.into();
            }
        } else if val_bits > expected_bits {
            // Truncate (e.g., i64 -> i1)
            if let Ok(trunc) = ctx
                .builder
                .build_int_truncate(int_val, expected_int, "trunc_ret")
            {
                return trunc.into();
            }
        }
    }

    // Return original value if no conversion applied
    val
}

/// Create a default return value for the given type.
/// This is used when a return statement is missing or operand_to_value fails.
/// Returns a type-appropriate default: 0 for Int/Float, false for Bool, null for pointers.
fn create_default_return_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    type_id: doo_core::types::TypeId,
) -> BasicValueEnum<'ctx> {
    // Get LLVM type for the expected return type
    let llvm_type = ctx.get_llvm_type(type_id);

    // Create appropriate default value based on LLVM type
    match llvm_type {
        BasicTypeEnum::IntType(t) => t.const_zero().into(),
        BasicTypeEnum::FloatType(t) => t.const_zero().into(),
        BasicTypeEnum::PointerType(t) => t.const_null().into(),
        BasicTypeEnum::ArrayType(t) => t.const_zero().into(),
        BasicTypeEnum::StructType(t) => t.const_zero().into(),
        BasicTypeEnum::VectorType(t) => t.const_zero().into(),
        BasicTypeEnum::ScalableVectorType(t) => t.const_zero().into(),
    }
}

/// Convert an i64 value to the target Doo type.
/// Used for closure parameters which come in as i64 but need conversion to their actual types.
fn convert_i64_to_type<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    target_type: doo_core::types::TypeId,
) -> BasicValueEnum<'ctx> {
    use doo_core::types::TypeKind;

    let i64_val = val.into_int_value();

    let kind = match ctx.get_type_kind(target_type) {
        Some(k) => k,
        None => return val, // Unknown type, keep as-is
    };

    match kind {
        TypeKind::Int => {
            // Already i64
            i64_val.into()
        }
        TypeKind::Bool => {
            // Truncate i64 to i8 (Bool is i8 for C ABI compatibility)
            ctx.builder
                .build_int_truncate(i64_val, ctx.context.i8_type(), "i64_to_bool")
                .map(|v| v.into())
                .unwrap_or(val)
        }
        TypeKind::Float32 | TypeKind::Float64 => {
            // Reinterpret i64 bits as f64
            ctx.builder
                .build_bit_cast(i64_val, ctx.context.f64_type(), "i64_to_f64")
                .map(|v| v)
                .unwrap_or(val)
        }
        TypeKind::Str | TypeKind::Array { .. } | TypeKind::Map { .. } | TypeKind::Struct { .. } => {
            // Convert i64 to pointer — with safety check for invalid addresses.
            // Zero or negative i64 values are not valid user-space pointers.
            let ptr_type = ctx
                .context
                .i8_type()
                .ptr_type(inkwell::AddressSpace::default());
            let is_positive = ctx.builder.build_int_compare(
                IntPredicate::SGT,
                i64_val,
                ctx.context.i64_type().const_zero(),
                "is_valid_addr",
            );
            if let Ok(is_valid) = is_positive {
                let null_ptr = ptr_type.const_null();
                if let Ok(as_ptr) = ctx
                    .builder
                    .build_int_to_ptr(i64_val, ptr_type, "i64_to_ptr")
                {
                    if let Ok(result) =
                        ctx.builder
                            .build_select(is_valid, as_ptr, null_ptr, "safe_i64_to_ptr")
                    {
                        return result;
                    }
                }
            }
            // Fallback
            ctx.builder
                .build_int_to_ptr(i64_val, ptr_type, "i64_to_ptr_fallback")
                .map(|v| v.into())
                .unwrap_or(val)
        }
        _ => val, // Keep as-is for other types
    }
}

/// Convert any value to i64.
/// Used for closure return values which must be i64.
fn convert_to_i64<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
) -> BasicValueEnum<'ctx> {
    let i64_type = ctx.context.i64_type();

    if val.is_int_value() {
        let int_val = val.into_int_value();
        let bit_width = int_val.get_type().get_bit_width();
        if bit_width == 64 {
            return val;
        } else if bit_width < 64 {
            // Zero extend smaller ints (like i1 for bool)
            return ctx
                .builder
                .build_int_z_extend(int_val, i64_type, "to_i64")
                .map(|v| v.into())
                .unwrap_or(val);
        } else {
            // Truncate larger ints
            return ctx
                .builder
                .build_int_truncate(int_val, i64_type, "to_i64")
                .map(|v| v.into())
                .unwrap_or(val);
        }
    } else if val.is_float_value() {
        // Bitcast f64 to i64
        let f64_val = val.into_float_value();
        return ctx
            .builder
            .build_bit_cast(f64_val, i64_type, "f64_to_i64")
            .map(|v| v)
            .unwrap_or(BasicValueEnum::IntValue(i64_type.const_zero()));
    } else if val.is_pointer_value() {
        // Convert pointer to i64
        let ptr_val = val.into_pointer_value();
        return ctx
            .builder
            .build_ptr_to_int(ptr_val, i64_type, "ptr_to_i64")
            .map(|v| v.into())
            .unwrap_or(val);
    }

    // Fallback: return 0
    i64_type.const_zero().into()
}

/// Main code generation builder.
pub struct CodegenBuilder<'ctx> {
    context: &'ctx Context,
}

impl<'ctx> CodegenBuilder<'ctx> {
    /// Create a new CodegenBuilder with the given LLVM context.
    pub fn new(context: &'ctx Context) -> Self {
        Self { context }
    }

    /// Generate LLVM IR module from MIR.
    pub fn build(
        &self,
        mir: &MirProgram,
        module_name: &str,
        type_registry: Arc<TypeRegistry>,
    ) -> Module<'ctx> {
        let mut ctx = CodegenContext::new(self.context, module_name, type_registry);

        // Declare FFI runtime functions
        self.declare_runtime_functions(&mut ctx);

        // Check if async features are used — sets flag for main() runtime init
        ctx.has_async = mir.has_async_features();

        let dispatcher = InstructionDispatcher::new();

        // Pre-pass: declare all struct types from the type registry
        self.declare_struct_types(&mut ctx);

        // Populate struct field decorators from MIR structs
        for (struct_name, struct_def) in &mir.structs {
            let field_decorators: Vec<(String, Vec<String>)> = struct_def
                .fields
                .iter()
                .map(|f| {
                    let decs: Vec<String> = f
                        .decorators
                        .iter()
                        .map(|d| {
                            if d.args.is_empty() {
                                resolve(d.name)
                            } else {
                                format!("{}({})", resolve(d.name), d.args.join(","))
                            }
                        })
                        .collect();
                    (resolve(f.name), decs)
                })
                .collect();
            ctx.struct_field_decorators
                .insert(resolve(*struct_name), field_decorators);
        }

        // Populate RBAC policies from MIR
        for (struct_name, policy_json) in &mir.policies {
            ctx.rbac_policies
                .insert(resolve(*struct_name), policy_json.clone());
        }

        // Populate enum variant inheritance from MIR enums
        for (enum_name, enum_def) in &mir.enums {
            let mut inheritance: Vec<(String, Vec<String>)> = Vec::new();
            for variant in &enum_def.variants {
                let inherited: Vec<String> = variant
                    .decorators
                    .iter()
                    .filter(|d| resolve(d.name) == "inherits")
                    .flat_map(|d| d.args.iter().cloned())
                    .collect();
                if !inherited.is_empty() {
                    inheritance.push((resolve(variant.name), inherited));
                }
            }
            if !inheritance.is_empty() {
                ctx.enum_inheritance
                    .insert(resolve(*enum_name), inheritance);
            }
        }

        // Emit LLVM global constants from MirGlobal entries (primitive consts)
        for global in &mir.globals {
            self.emit_global(&mut ctx, global);
        }

        // First pass: declare all functions
        for func in &mir.functions {
            self.declare_function(&mut ctx, func);
        }

        // Second pass: generate function bodies
        for func in &mir.functions {
            self.generate_function(&mut ctx, func, &dispatcher);
        }

        // Builder must not be dropped — its Drop calls LLVMDisposeBuilder which
        // can crash on Windows due to CRT/allocator mismatch between the Rust
        // binary and LLVM-C.dll (LLVM 22+). The Module is safely returned.
        let CodegenContext {
            module, builder, ..
        } = ctx;
        std::mem::forget(builder);
        module
    }

    /// Emit a single LLVM global from a `MirGlobal`.
    ///
    /// - `GlobalKind::Const`: Emits a compile-time constant global (inlined at use sites).
    /// - `GlobalKind::Static`: Emits a OnceLock-style `{ i1, ptr }` global + set/get helpers.
    fn emit_global(&self, ctx: &mut CodegenContext<'ctx>, global: &MirGlobal) {
        use doo_mir::GlobalKind;
        use inkwell::module::Linkage;

        let name = resolve(global.name);

        match global.kind {
            GlobalKind::Const => {
                if let Some(value) = &global.value {
                    match value {
                        MirConst::Int(v) => {
                            let ty = ctx.context.i64_type();
                            let g = ctx.module.add_global(ty, None, &name);
                            g.set_initializer(&ty.const_int(*v as u64, true));
                            g.set_constant(true);
                            g.set_linkage(Linkage::Internal);
                            ctx.set_temp(&name, ty.const_int(*v as u64, true).into());
                        }
                        MirConst::Float(v) => {
                            let ty = ctx.context.f64_type();
                            let g = ctx.module.add_global(ty, None, &name);
                            g.set_initializer(&ty.const_float(*v));
                            g.set_constant(true);
                            g.set_linkage(Linkage::Internal);
                            ctx.set_temp(&name, ty.const_float(*v).into());
                        }
                        MirConst::Bool(v) => {
                            let ty = ctx.context.i8_type();
                            let g = ctx.module.add_global(ty, None, &name);
                            g.set_initializer(&ty.const_int(*v as u64, false));
                            g.set_constant(true);
                            g.set_linkage(Linkage::Internal);
                            ctx.set_temp(&name, ty.const_int(*v as u64, false).into());
                        }
                        MirConst::Str(s) => {
                            if let Ok(g) = ctx.builder.build_global_string_ptr(s.as_str(), &name) {
                                ctx.set_temp(&name, g.as_pointer_value().into());
                            }
                        }
                        MirConst::Nil => {}
                    }
                }
            }

            GlobalKind::Static => {
                // Register as a static global for set/get codegen
                ctx.static_globals.insert(name.clone(), global.type_id);

                // Create OnceLock-style global: { i1 initialized, ptr value }
                let once_lock_type = ctx.context.struct_type(
                    &[
                        ctx.context.bool_type().into(),
                        ctx.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into(),
                    ],
                    false,
                );
                let once_lock_global = ctx.module.add_global(once_lock_type, None, &name);
                once_lock_global.set_initializer(&once_lock_type.const_zero());
                once_lock_global.set_constant(false);
                once_lock_global.set_linkage(Linkage::Internal);
            }
        }
    }

    /// Pre-declare all struct types from the type registry.
    /// This ensures struct types are cached before we try to access their fields.
    fn declare_struct_types(&self, ctx: &mut CodegenContext<'ctx>) {
        let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();

        // First, collect all struct information from the registry
        // Fields are now (name, type_id, is_public) tuples
        // Also resolve TypeRef to include imported struct types
        let structs: Vec<(String, Vec<(String, doo_core::types::TypeId, bool)>)> = ctx
            .type_registry
            .all_type_ids()
            .filter_map(|type_id| {
                if let Some(type_info) = ctx.type_registry.get(type_id) {
                    match &type_info.kind {
                        doo_core::types::TypeKind::Struct { def } => {
                            let struct_name = def.name.resolve().to_string();
                            let struct_fields: Vec<(String, doo_core::types::TypeId, bool)> = def
                                .fields
                                .iter()
                                .map(|f| (f.name.resolve().to_string(), f.type_id, f.is_public))
                                .collect();
                            return Some((struct_name, struct_fields));
                        }
                        // Handle TypeRef to import struct types from other modules
                        // The TypeRef references struct types from imported modules
                        doo_core::types::TypeKind::TypeRef { name: ref_name } => {
                            if debug {}
                            // Iterate through ALL types to find the struct definition
                            // The struct might be registered under a different type_id
                            for other_tid in ctx.type_registry.all_type_ids().collect::<Vec<_>>() {
                                if let Some(other_info) = ctx.type_registry.get(other_tid) {
                                    if let doo_core::types::TypeKind::Struct { def } =
                                        &other_info.kind
                                    {
                                        if def.name == *ref_name {
                                            if debug {}
                                            let struct_name = def.name.resolve().to_string();
                                            let struct_fields: Vec<(
                                                String,
                                                doo_core::types::TypeId,
                                                bool,
                                            )> = def
                                                .fields
                                                .iter()
                                                .map(|f| {
                                                    (
                                                        f.name.resolve().to_string(),
                                                        f.type_id,
                                                        f.is_public,
                                                    )
                                                })
                                                .collect();
                                            return Some((struct_name, struct_fields));
                                        }
                                    }
                                }
                            }
                            if debug {}
                        }
                        _ => {}
                    }
                }
                None
            })
            .collect();

        // Now process each struct with mutable access to ctx
        for (name, fields) in structs {
            // Build LLVM field types (ignore visibility for LLVM type)
            let field_types: Vec<inkwell::types::BasicTypeEnum> = fields
                .iter()
                .map(|(_, field_type_id, _)| ctx.get_llvm_type(*field_type_id))
                .collect();

            // Cache the struct type
            let _struct_type = ctx.get_struct_type(&name, &field_types);

            // Also register struct metadata (field names)
            let field_names: Vec<String> = fields.iter().map(|(n, _, _)| n.clone()).collect();
            ctx.register_struct_metadata(&name, field_names);

            if debug {}
        }
    }

    /// Declare a function (signature only, no body).
    fn declare_function(&self, ctx: &mut CodegenContext<'ctx>, func: &MirFunction) {
        let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();
        let func_name = resolve(func.name);
        if debug {}

        // Closure functions have special calling convention: (env: ptr, params...) -> return_type
        // Use actual types for parameters and return type (no i64 boxing)
        if func.is_closure {
            let ptr_type = ctx
                .context
                .i8_type()
                .ptr_type(inkwell::AddressSpace::default());

            // Build param types: env ptr + user params (with actual types)
            let mut param_types: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_type.into()];
            for param in &func.params {
                let llvm_type = ctx.get_llvm_type(param.type_id);
                param_types.push(llvm_type.into());
            }

            // Use actual return type (default to i64 if none specified)
            let return_llvm_type = func
                .return_type
                .map(|t| ctx.get_llvm_type(t))
                .unwrap_or_else(|| ctx.context.i64_type().into());

            let fn_type = return_llvm_type.fn_type(&param_types, false);
            let closure_fn = ctx.module.add_function(&func_name, fn_type, None);

            // Disable tail calls to prevent sret + tail call stack corruption on Windows x64
            let no_tail = ctx
                .context
                .create_string_attribute("disable-tail-calls", "true");
            closure_fn.add_attribute(inkwell::attributes::AttributeLoc::Function, no_tail);

            return;
        }

        // Build parameter types
        let param_types: Vec<BasicTypeEnum<'ctx>> = func
            .params
            .iter()
            .map(|p| ctx.get_llvm_type(p.type_id))
            .collect();

        // Collect Doo TypeIds for parameter types (for argument coercion)
        let param_type_ids: Vec<_> = func.params.iter().map(|p| p.type_id).collect();
        ctx.register_function_param_types(&func_name, param_type_ids);

        // Register return type for struct serialization
        if let Some(ret_type_id) = func.return_type {
            ctx.register_function_return_type(&func_name, ret_type_id);
        }

        // Register error type for middleware error handling
        if let Some(err_type_id) = func.error_type {
            ctx.register_function_error_type(&func_name, err_type_id);
        }

        // Build return type
        // If function has an error_type, it returns a Result struct { i64, ptr }
        // Using ptr for the payload field preserves pointer provenance through LLVM O3.
        let return_type = if func.error_type.is_some() {
            // Result type: { i64 tag, ptr payload }
            let result_struct_type = ctx.context.struct_type(
                &[
                    ctx.context.i64_type().into(),
                    ctx.context
                        .ptr_type(inkwell::AddressSpace::default())
                        .into(),
                ],
                false,
            );
            Some(result_struct_type.into())
        } else {
            func.return_type.map(|t| ctx.get_llvm_type(t))
        };

        // For FFI functions, use external linkage and register symbol
        if let Some(ffi) = &func.ffi {
            // Use C ABI compatible types for FFI declarations:
            // Bool → i32 (C ABI) instead of i8 (internal Doo representation)
            let ffi_param_meta: Vec<BasicMetadataTypeEnum<'ctx>> = func
                .params
                .iter()
                .map(|p| {
                    let llvm_ty: BasicTypeEnum<'ctx> = if p.type_id == builtin::BOOL {
                        ctx.i32_type().into()
                    } else {
                        ctx.get_llvm_type(p.type_id)
                    };
                    llvm_ty.into()
                })
                .collect();

            // For FFI functions with error types, they return *mut SimpleResult (pointer to heap-allocated result)
            // to avoid Windows x64 sret ABI issues. The caller loads the struct from the pointer.
            let ffi_return_type = if func.error_type.is_some() {
                // FFI returns pointer to result, not struct by value
                Some(
                    ctx.context
                        .ptr_type(inkwell::AddressSpace::default())
                        .into(),
                )
            } else {
                // Use C ABI compatible return type: Bool → i32
                func.return_type.map(|t| -> BasicTypeEnum<'ctx> {
                    if t == builtin::BOOL {
                        ctx.i32_type().into()
                    } else {
                        ctx.get_llvm_type(t)
                    }
                })
            };

            let fn_type = match ffi_return_type {
                Some(ret) => {
                    use inkwell::types::BasicType;
                    ret.fn_type(&ffi_param_meta, false)
                }
                None => ctx.context.void_type().fn_type(&ffi_param_meta, false),
            };

            // Use explicit symbol if provided, otherwise derive from library + function name
            let ffi_lib = resolve(ffi.library);
            let symbol = ffi
                .symbol
                .map(|s| resolve(s))
                .unwrap_or_else(|| { let p: Vec<&str> = func_name.split(".").collect(); if p.len() == 2 { format!("{}_{}_{}", ffi_lib, p[0].to_lowercase(), p[1].to_lowercase()) } else { format!("{}_{}", ffi_lib, func_name.to_lowercase()) } });

            // Declare FFI function with its EXTERNAL SYMBOL NAME (not the Doo function name)
            // This is critical: linker will look for this exact symbol name
            ctx.module
                .add_function(&symbol, fn_type, Some(Linkage::External));

            // Also register the Doo function name as an alias to the symbol
            // This allows ctx.get_function("jwt") to find "doo_http_jwt"
            if let Some(f) = ctx.module.get_function(&symbol) {
                ctx.register_function_alias(&func_name, f);
            }

            // Register FFI symbol mapping for wrapper generation
            ctx.register_ffi_symbol(&func_name, &ffi_lib, &symbol);

            // Register FFI library mapping for package dispatch.
            // Maps external symbol → library name so the codegen dispatch system
            // can route FFI calls to the correct package hooks.
            ctx.register_ffi_library(&symbol, &ffi_lib);

            // Register FFI type signature from MIR FfiLinkage (package-ready).
            // This allows codegen to generate correct LLVM types for any @extern
            // function, including third-party packages, without needing a
            // hardcoded match table entry.
            ctx.register_ffi_type_signature(
                &symbol,
                ffi.param_types.clone(),
                ffi.return_type,
                ffi.is_result,
            );
        } else {
            ctx.declare_function(&func_name, &param_types, return_type);
        }
    }

    /// Generate a function body.
    fn generate_function(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        func: &MirFunction,
        dispatcher: &InstructionDispatcher<'ctx>,
    ) {
        // Skip FFI functions - they have no body
        if func.ffi.is_some() {
            return;
        }

        let func_name_str = resolve(func.name);
        let llvm_func = match ctx.get_function(&func_name_str) {
            Some(f) => f,
            None => return, // Function not declared
        };

        // Clear locals for this function
        ctx.clear_locals();

        // Set current function's return type for proper return value conversion
        ctx.current_function_return_type = func.return_type;

        // Track whether current function has error type (returns Result { i64, ptr })
        ctx.current_function_has_error_type = func.error_type.is_some();

        // Set closure flag for special return value handling
        ctx.is_closure_function = func.is_closure;

        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
            for local in &func.locals {}
        }

        // Collect all assigned variables from MIR to create allocas
        // FIRST: Pre-populate with all known local types from func.locals
        let mut assigned_vars: HashMap<String, doo_core::types::TypeId> = HashMap::new();
        for local in &func.locals {
            assigned_vars.insert(resolve(local.name), local.type_id);
        }

        // SECOND: Scan all blocks to infer types for temps from assignments
        // This requires multiple passes since types can flow through multiple temps
        let max_passes = 10; // Prevent infinite loops
        for _pass in 0..max_passes {
            let mut changed = false;
            for block in &func.blocks {
                for instr in &block.instructions {
                    if let doo_mir::MirInstrKind::Assign { dest, value } = &instr.kind {
                        let dest_str = resolve(*dest);
                        // Infer type from value
                        let type_id = match value {
                            MirOperand::Const(MirConst::Int(_)) => builtin::INT,
                            MirOperand::Const(MirConst::Float(_)) => builtin::FLOAT,
                            MirOperand::Const(MirConst::Bool(_)) => builtin::BOOL,
                            MirOperand::Const(MirConst::Str(_)) => builtin::STR,
                            MirOperand::Const(MirConst::Nil) => builtin::ANY,
                            MirOperand::Local(name) | MirOperand::Temp(name) => {
                                // Get type from existing var if known (now includes func.locals)
                                let n = resolve(*name);
                                assigned_vars.get(&n).copied().unwrap_or(builtin::ANY)
                            }
                            MirOperand::Global(_) => builtin::ANY,
                            MirOperand::FuncRef(_) => builtin::ANY, // Function references are opaque
                        };
                        // Only update if we have a concrete type (not ANY) or dest is unknown
                        if type_id != builtin::ANY || !assigned_vars.contains_key(&dest_str) {
                            if assigned_vars.get(&dest_str) != Some(&type_id) {
                                assigned_vars.insert(dest_str, type_id);
                                changed = true;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        // Create LLVM basic blocks for each MIR block
        let mut block_map: HashMap<String, BasicBlock<'ctx>> = HashMap::new();
        for mir_block in &func.blocks {
            let label = resolve(mir_block.label);
            let bb = ctx.context.append_basic_block(llvm_func, &label);
            block_map.insert(label, bb);
        }

        // Position at entry block for allocas
        let entry_bb = block_map
            .get("entry")
            .copied()
            .unwrap_or_else(|| ctx.context.append_basic_block(llvm_func, "entry"));
        ctx.builder.position_at_end(entry_bb);

        // If this is main() and async features are used, emit doo_runtime_init()
        let func_name = llvm_func.get_name().to_str().unwrap_or("");
        if func_name == "main" && ctx.has_async {
            let init_fn = crate::instructions::async_ops::get_or_declare_doo_runtime_init(ctx);
            let _ = ctx.builder.build_call(init_fn, &[], "runtime_init");
        }

        // For closures, first LLVM param is env_ptr (skip it when mapping to MIR params)
        // Closure params now use actual types (no i64 conversion needed)
        let param_offset = if func.is_closure { 1 } else { 0 };

        // Create allocas for parameters
        for (i, param) in func.params.iter().enumerate() {
            let param_name = resolve(param.name);
            let llvm_param_idx = (i + param_offset) as u32;
            let param_value = match llvm_func.get_nth_param(llvm_param_idx) {
                Some(p) => p,
                None => {
                    continue;
                }
            };
            let param_type = ctx.get_llvm_type(param.type_id);
            let alloca = ctx.create_local(&param_name, param_type);

            // Store param directly - closures now use actual types
            if let Err(_) = ctx.builder.build_store(alloca, param_value) {
                continue;
            }

            // Track parameter type for Clone/Drop
            ctx.set_variable_type(&param_name, param.type_id);

            // CRITICAL: For struct parameters (or TypeRef to struct), register the struct type association
            // This is needed for FieldGet/FieldSet to work correctly when the parameter
            // is passed to another function or when its fields are accessed
            if let Some(kind) = ctx.get_type_kind(param.type_id) {
                match kind {
                    TypeKind::Struct { def } => {
                        ctx.set_temp_struct_type(&param_name, def.name.resolve());
                    }
                    // CRITICAL: Handle TypeRef for imported/cross-module types
                    // The TypeRef name IS the struct name, use it directly
                    TypeKind::TypeRef { name: ref_name } => {
                        // First try to resolve through lookup
                        let struct_name: Option<String> = if let Some(resolved_tid) =
                            ctx.type_registry.lookup(ref_name.resolve())
                        {
                            if let Some(TypeKind::Struct { def }) = ctx.get_type_kind(resolved_tid)
                            {
                                Some(def.name.resolve().to_string())
                            } else {
                                // If resolution doesn't give a struct, use the TypeRef name directly
                                Some(ref_name.resolve().to_string())
                            }
                        } else {
                            // If lookup fails (common for imported types), use the TypeRef name directly
                            // This is the struct name used for LLVM type lookup
                            Some(ref_name.resolve().to_string())
                        };

                        if let Some(sname) = struct_name {
                            ctx.set_temp_struct_type(&param_name, &sname);
                        }
                    }
                    other => if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {},
                }
            } else {
            }
        }

        // Create allocas for local variables from MIR
        // Skip if a variable with this name already exists (e.g., from parameters)
        for local in &func.locals {
            let local_name = resolve(local.name);
            if ctx.get_local(&local_name).is_none() {
                let local_type = ctx.get_llvm_type(local.type_id);
                ctx.create_local(&local_name, local_type);
                // Track local type for Clone/Drop
                ctx.set_variable_type(&local_name, local.type_id);

                // CRITICAL: For struct locals (or TypeRef to struct), register the struct type association
                // This is needed for FieldGet/FieldSet to work correctly
                if let Some(kind) = ctx.get_type_kind(local.type_id) {
                    match kind {
                        TypeKind::Struct { def } => {
                            ctx.set_temp_struct_type(&local_name, def.name.resolve());
                        }
                        // CRITICAL: Handle TypeRef for imported/cross-module types
                        // The TypeRef name IS the struct name, use it directly
                        TypeKind::TypeRef { name: ref_name } => {
                            // First try to resolve through lookup
                            let struct_name: Option<String> = if let Some(resolved_tid) =
                                ctx.type_registry.lookup(ref_name.resolve())
                            {
                                if let Some(TypeKind::Struct { def }) =
                                    ctx.get_type_kind(resolved_tid)
                                {
                                    Some(def.name.resolve().to_string())
                                } else {
                                    Some(ref_name.resolve().to_string())
                                }
                            } else {
                                Some(ref_name.resolve().to_string())
                            };

                            if let Some(sname) = struct_name {
                                ctx.set_temp_struct_type(&local_name, &sname);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Create allocas for all assigned variables (for proper loop handling)
        for (name, type_id) in &assigned_vars {
            // Skip static globals — they are OnceLock globals, not stack allocas
            if ctx.static_globals.contains_key(name) {
                continue;
            }
            if ctx.get_local(name).is_none() {
                let var_type = ctx.get_llvm_type(*type_id);
                ctx.create_local(name, var_type);
                // Track assigned variable type for Clone/Drop
                ctx.set_variable_type(name, *type_id);
                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}
            }
        }

        // Unpack captured variables from env pointer for spawn functions.
        // Must happen AFTER allocas are created and BEFORE block code generation.
        if func.is_closure && !func.captures.is_empty() {
            let capture_names: Vec<String> = func.captures.iter().map(|s| resolve(*s)).collect();
            crate::instructions::async_ops::emit_env_unpack(
                ctx,
                &capture_names,
                llvm_func,
                func.captures_by_value,
                false, // closures are multi-use; env freed when closure dropped
            );
        }

        // Generate code for each block
        for mir_block in &func.blocks {
            let block_label = resolve(mir_block.label);
            let bb = block_map[&block_label];
            ctx.builder.position_at_end(bb);

            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}

            // Generate instructions
            let mut instr_count = 0;
            for instr in &mir_block.instructions {
                instr_count += 1;
                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}
                // Use custom emit for Assign to store in alloca.
                // Also handles static globals (OnceLock set-once semantics).
                if let doo_mir::MirInstrKind::Assign { dest, value } = &instr.kind {
                    let dest_str = resolve(*dest);
                    if let Some(val) = operand_to_value(ctx, value) {
                        // Check if assigning to a static global — direct OnceLock write
                        if ctx.static_globals.contains_key(&dest_str) {
                            if let Some(once_lock) = ctx.module.get_global(&dest_str) {
                                let once_lock_ptr = once_lock.as_pointer_value();
                                let once_lock_type = ctx.context.struct_type(
                                    &[
                                        ctx.context.bool_type().into(),
                                        ctx.context
                                            .ptr_type(inkwell::AddressSpace::default())
                                            .into(),
                                    ],
                                    false,
                                );
                                if let Ok(ptr_field) = ctx.builder.build_struct_gep(
                                    once_lock_type,
                                    once_lock_ptr,
                                    1,
                                    "static_ptr_field",
                                ) {
                                    let ptr_val = if val.is_pointer_value() {
                                        val.into_pointer_value()
                                    } else if val.is_int_value() {
                                        ctx.builder
                                            .build_int_to_ptr(
                                                val.into_int_value(),
                                                ctx.context
                                                    .ptr_type(inkwell::AddressSpace::default()),
                                                "int_to_ptr",
                                            )
                                            .ok()
                                            .unwrap_or_else(|| {
                                                ctx.context
                                                    .ptr_type(inkwell::AddressSpace::default())
                                                    .const_null()
                                            })
                                    } else {
                                        ctx.context
                                            .ptr_type(inkwell::AddressSpace::default())
                                            .const_null()
                                    };
                                    ctx.builder.build_store(ptr_field, ptr_val).ok();
                                }
                                if let Ok(flag_field) = ctx.builder.build_struct_gep(
                                    once_lock_type,
                                    once_lock_ptr,
                                    0,
                                    "static_flag_field",
                                ) {
                                    ctx.builder
                                        .build_store(
                                            flag_field,
                                            ctx.context.bool_type().const_int(1, false),
                                        )
                                        .ok();
                                }
                            }
                        } else {
                            // Propagate struct type association if present
                            if let Some(src_name) = get_operand_name(value) {
                                if let Some(struct_name) =
                                    ctx.get_temp_struct_type(&src_name).cloned()
                                {
                                    ctx.set_temp_struct_type(&dest_str, &struct_name);
                                }
                                if let Some(src_type) = ctx.get_variable_type(&src_name) {
                                    ctx.set_variable_type(&dest_str, src_type);
                                }
                                if let Some(&elem_type) = ctx.array_element_types.get(&src_name) {
                                    ctx.array_element_types.insert(dest_str.clone(), elem_type);
                                }
                            }
                            ctx.set_local(dest_str, val);
                        }
                    }
                } else {
                    dispatcher.emit(ctx, instr);
                }
            }

            // Generate terminator
            self.emit_terminator(ctx, &mir_block.terminator, &block_map);
        }

        // Ensure ALL LLVM basic blocks have terminators.
        // This is critical because some codegen operations (like clone_string) create
        // additional basic blocks (e.g., clone_merge) that are not in the MIR block_map.
        // Without this, LLVM verification will fail with "does not have terminator" errors.
        let llvm_func = block_map.values().next().and_then(|bb| bb.get_parent());
        if let Some(llvm_func) = llvm_func {
            // Check if this is the main function - main must always return i32
            let is_main = llvm_func.get_name().to_str().unwrap_or("") == "main";

            let mut maybe_bb = llvm_func.get_first_basic_block();
            while let Some(bb) = maybe_bb {
                if bb.get_terminator().is_none() {
                    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}
                    ctx.builder.position_at_end(bb);

                    // Check if this is a Result-returning function (has error_type).
                    // Result functions return { i64 tag, ptr payload } struct.
                    if func.error_type.is_some() {
                        // Create a default Result struct: { 0 (Ok tag), null (payload) }
                        let result_struct_type = ctx.context.struct_type(
                            &[
                                ctx.context.i64_type().into(),
                                ctx.context
                                    .ptr_type(inkwell::AddressSpace::default())
                                    .into(),
                            ],
                            false,
                        );
                        let default_result = result_struct_type.const_zero();
                        ctx.builder.build_return(Some(&default_result)).ok();
                    } else if is_main {
                        // Main function must return i32, even if MIR has no return type
                        let zero = ctx.context.i32_type().const_int(0, false);
                        ctx.builder.build_return(Some(&zero)).ok();
                    } else if func.return_type.is_none() {
                        ctx.builder.build_return(None).ok();
                    } else if let Some(ret_type_id) = func.return_type {
                        let ret_type = ctx.get_llvm_type(ret_type_id);
                        let default: BasicValueEnum = match ret_type {
                            BasicTypeEnum::IntType(t) => t.const_zero().into(),
                            BasicTypeEnum::FloatType(t) => t.const_zero().into(),
                            BasicTypeEnum::PointerType(t) => t.const_null().into(),
                            BasicTypeEnum::ArrayType(t) => t.const_zero().into(),
                            BasicTypeEnum::StructType(t) => t.const_zero().into(),
                            BasicTypeEnum::VectorType(t) => t.const_zero().into(),
                            BasicTypeEnum::ScalableVectorType(t) => t.const_zero().into(),
                        };
                        ctx.builder.build_return(Some(&default)).ok();
                    }
                }
                maybe_bb = bb.get_next_basic_block();
            }
        }
    }

    /// Emit a terminator instruction.
    fn emit_terminator(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        term: &MirTerminator,
        block_map: &HashMap<String, BasicBlock<'ctx>>,
    ) {
        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}

        match term {
            MirTerminator::Return { values } => {
                // Check if we're in main function - main returns i32 0
                let is_main = ctx
                    .builder
                    .get_insert_block()
                    .and_then(|bb| bb.get_parent())
                    .map(|f| f.get_name().to_str().unwrap_or("") == "main")
                    .unwrap_or(false);

                let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();

                if is_main {
                    // For main function, call exit(0) to bypass Tokio runtime shutdown issues.
                    // The tokio runtime used by FFI (doo_db, doo_http) can cause crashes at exit
                    // if we use normal return. Calling exit(0) cleanly terminates the process.

                    // First flush stdout to ensure all output is written
                    let fflush_fn =
                        ctx.module
                            .get_function(ffi_names::FFLUSH)
                            .unwrap_or_else(|| {
                                let fn_type = ctx.context.i32_type().fn_type(
                                    &[ctx
                                        .context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into()],
                                    false,
                                );
                                ctx.module.add_function(ffi_names::FFLUSH, fn_type, None)
                            });
                    let null_ptr = ctx
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null();
                    ctx.builder
                        .build_call(fflush_fn, &[null_ptr.into()], "flush_stdout")
                        .ok();

                    // Now call exit(0)
                    let exit_fn = ctx.module.get_function(ffi_names::EXIT).unwrap_or_else(|| {
                        let fn_type = ctx
                            .context
                            .void_type()
                            .fn_type(&[ctx.context.i32_type().into()], false);
                        ctx.module.add_function(ffi_names::EXIT, fn_type, None)
                    });
                    let zero = ctx.context.i32_type().const_int(0, false);
                    ctx.builder
                        .build_call(exit_fn, &[zero.into()], "main_exit")
                        .ok();
                    ctx.builder.build_unreachable().ok();
                } else if values.is_empty() {
                    // Non-main function with no return values
                    // CRITICAL: If function has error_type, return a default Result struct
                    if ctx.current_function_has_error_type {
                        let result_struct_type = ctx.context.struct_type(
                            &[
                                ctx.context.i64_type().into(),
                                ctx.context
                                    .ptr_type(inkwell::AddressSpace::default())
                                    .into(),
                            ],
                            false,
                        );
                        let default_result = result_struct_type.const_zero();
                        ctx.builder.build_return(Some(&default_result)).ok();
                    } else if let Some(ret_type) = ctx.current_function_return_type {
                        let default_val = create_default_return_value(ctx, ret_type);
                        ctx.builder.build_return(Some(&default_val)).ok();
                    } else {
                        ctx.builder.build_return(None).ok();
                    }
                } else if values.len() == 1 {
                    // CRITICAL: For return values that are temps, prefer loading from
                    // an alloca over using the temp HashMap value directly.
                    // This is necessary for match/if expressions where different arms
                    // assign to the same temp in different blocks - the temp value
                    // may not dominate the merge block where the return is emitted.
                    let return_val = match &values[0] {
                        MirOperand::Temp(name) | MirOperand::Local(name) => {
                            // If there's an alloca for this variable, always load from it
                            // to ensure SSA domination (allocas are in the entry block)
                            let name_str = resolve(*name);
                            if let Some(loaded) = ctx.load_from_local(&name_str) {
                                Some(loaded)
                            } else {
                                operand_to_value(ctx, &values[0])
                            }
                        }
                        _ => operand_to_value(ctx, &values[0]),
                    };
                    if let Some(val) = return_val {
                        if debug {}
                        // Convert value to expected return type if needed
                        // Closures now use actual types, no i64 conversion needed
                        let final_val = convert_return_value(ctx, val);
                        ctx.builder.build_return(Some(&final_val)).ok();
                    } else {
                        if debug {}
                        // CRITICAL: If function has error_type, return a default Result struct
                        if ctx.current_function_has_error_type {
                            let result_struct_type = ctx.context.struct_type(
                                &[
                                    ctx.context.i64_type().into(),
                                    ctx.context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                ],
                                false,
                            );
                            let default_result = result_struct_type.const_zero();
                            ctx.builder.build_return(Some(&default_result)).ok();
                        } else if let Some(ret_type) = ctx.current_function_return_type {
                            let default_val = create_default_return_value(ctx, ret_type);
                            ctx.builder.build_return(Some(&default_val)).ok();
                        } else {
                            ctx.builder.build_return(None).ok();
                        }
                    }
                } else {
                    // Multiple return values - return as tuple
                    // Convert all values to LLVM values
                    let llvm_values: Vec<_> = values
                        .iter()
                        .filter_map(|v| operand_to_value(ctx, v))
                        .collect();

                    if llvm_values.len() != values.len() {
                        // Some values couldn't be converted
                        ctx.builder.build_return(None).ok();
                    } else {
                        // Get element types
                        let element_types: Vec<_> =
                            llvm_values.iter().map(|v| v.get_type()).collect();

                        // Create tuple struct type
                        let tuple_type = ctx.context.struct_type(&element_types, false);

                        // Allocate space for tuple on heap (so pointer is valid after return)
                        let ptr_type = ctx.context.ptr_type(inkwell::AddressSpace::default());
                        let size = match tuple_type.size_of() {
                            Some(s) => s,
                            None => {
                                // Fallback: use a reasonable default size (8 bytes per element)
                                ctx.context
                                    .i64_type()
                                    .const_int((element_types.len() as u64) * 8, false)
                            }
                        };
                        let malloc_fn = ctx.get_function(ffi_names::MALLOC).unwrap_or_else(|| {
                            let fn_type = ptr_type.fn_type(&[ctx.context.i64_type().into()], false);
                            ctx.module.add_function(
                                ffi_names::MALLOC,
                                fn_type,
                                Some(inkwell::module::Linkage::External),
                            )
                        });
                        let tuple_ptr = ctx
                            .builder
                            .build_call(malloc_fn, &[size.into()], "tuple_alloc")
                            .ok()
                            .and_then(|call| call.try_as_basic_value().basic())
                            .map(|v| v.into_pointer_value());

                        if let Some(tuple_ptr) = tuple_ptr {
                            // Zero-initialize to prevent garbage in unset fields
                            let memset_fn = ctx
                                .module
                                .get_function(ffi_names::MEMSET)
                                .unwrap_or_else(|| {
                                    let ptr_t =
                                        ctx.context.ptr_type(inkwell::AddressSpace::default());
                                    let fn_ty = ptr_t.fn_type(
                                        &[
                                            ptr_t.into(),
                                            ctx.context.i32_type().into(),
                                            ctx.context.i64_type().into(),
                                        ],
                                        false,
                                    );
                                    ctx.module.add_function(ffi_names::MEMSET, fn_ty, None)
                                });
                            let _ = ctx.builder.build_call(
                                memset_fn,
                                &[
                                    tuple_ptr.into(),
                                    ctx.context.i32_type().const_zero().into(),
                                    size.into(),
                                ],
                                "",
                            );

                            // Store each value in the tuple
                            for (i, val) in llvm_values.iter().enumerate() {
                                if let Ok(field_ptr) = ctx.builder.build_struct_gep(
                                    tuple_type,
                                    tuple_ptr,
                                    i as u32,
                                    &format!("field_{}", i),
                                ) {
                                    ctx.builder.build_store(field_ptr, *val).ok();
                                }
                            }

                            // Return the tuple pointer
                            ctx.builder.build_return(Some(&tuple_ptr)).ok();
                        } else {
                            ctx.builder.build_return(None).ok();
                        }
                    }
                }
            }
            MirTerminator::Goto { target } => {
                let target_str = resolve(*target);
                if let Some(target_bb) = block_map.get(&target_str) {
                    ctx.builder.build_unconditional_branch(*target_bb).ok();
                } else if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                }
            }
            MirTerminator::Branch {
                cond,
                then_block,
                else_block,
            } => {
                let then_str = resolve(*then_block);
                let else_str = resolve(*else_block);
                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                    if let Some(bb) = ctx.builder.get_insert_block() {}
                }
                if let Some(cond_val) = operand_to_value(ctx, cond) {
                    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}
                    // LLVM conditional branch requires i1 condition.
                    // Bool type is i8 (for C ABI), comparison results are i1.
                    // Convert any non-i1 integer to i1 via icmp ne 0.
                    let cond_bool = if cond_val.is_int_value() {
                        let int_val = cond_val.into_int_value();
                        if int_val.get_type().get_bit_width() == 1 {
                            int_val
                        } else {
                            // Convert i8/i32/i64 to i1: val != 0
                            ctx.builder
                                .build_int_compare(
                                    IntPredicate::NE,
                                    int_val,
                                    int_val.get_type().const_zero(),
                                    "cond_i1",
                                )
                                .unwrap_or(ctx.context.bool_type().const_int(1, false))
                        }
                    } else {
                        // Non-int: default to true
                        ctx.context.bool_type().const_int(1, false)
                    };

                    if let (Some(then_bb), Some(else_bb)) =
                        (block_map.get(&then_str), block_map.get(&else_str))
                    {
                        let result = ctx
                            .builder
                            .build_conditional_branch(cond_bool, *then_bb, *else_bb);
                        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                            if let Err(e) = &result {}
                        }
                        result.ok();
                    } else if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                    }
                } else if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                }
            }
            MirTerminator::Switch {
                value,
                cases,
                default,
            } => {
                // Multi-way branch (switch)
                if let Some(val) = operand_to_value(ctx, value) {
                    if val.is_int_value() {
                        let int_val = val.into_int_value();
                        let default_str = resolve(*default);
                        let default_bb = block_map
                            .get(&default_str)
                            .copied()
                            .or_else(|| ctx.builder.get_insert_block())
                            .unwrap_or_else(|| {
                                // Last resort: create a new block in the current function
                                let func = ctx.builder.get_insert_block()
                                    .and_then(|bb| bb.get_parent())
                                    .expect("ICE: no insert block or parent function during switch codegen");
                                ctx.context.append_basic_block(func, "switch_default_fallback")
                            });

                        let llvm_cases: Vec<_> = cases
                            .iter()
                            .filter_map(|(case_val, target)| {
                                let target_str = resolve(*target);
                                block_map.get(&target_str).map(|bb| {
                                    (
                                        ctx.context.i64_type().const_int(*case_val as u64, true),
                                        *bb,
                                    )
                                })
                            })
                            .collect();

                        ctx.builder
                            .build_switch(int_val, default_bb, &llvm_cases)
                            .ok();
                    }
                }
            }
            MirTerminator::Unreachable => {
                ctx.builder.build_unreachable().ok();
            }
        }
    }

    /// Declare FFI runtime functions (malloc, free, strlen, etc.)
    fn declare_runtime_functions(&self, ctx: &mut CodegenContext<'ctx>) {
        use doo_core::constants::ffi_names;
        use inkwell::AddressSpace;

        let i64_ty = self.context.i64_type();
        let i32_ty = self.context.i32_type();
        let _i8_ty = self.context.i8_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        // malloc: ptr malloc(i64)
        if ctx.module.get_function(ffi_names::MALLOC).is_none() {
            let fn_ty = ptr_ty.fn_type(&[i64_ty.into()], false);
            ctx.module.add_function(ffi_names::MALLOC, fn_ty, None);
        }

        // free: void free(ptr)
        if ctx.module.get_function(ffi_names::FREE).is_none() {
            let fn_ty = self.context.void_type().fn_type(&[ptr_ty.into()], false);
            ctx.module.add_function(ffi_names::FREE, fn_ty, None);
        }

        // realloc: ptr realloc(ptr, i64)
        if ctx.module.get_function(ffi_names::REALLOC).is_none() {
            let fn_ty = ptr_ty.fn_type(&[ptr_ty.into(), i64_ty.into()], false);
            ctx.module.add_function(ffi_names::REALLOC, fn_ty, None);
        }

        // strlen: i64 strlen(ptr)
        if ctx.module.get_function(ffi_names::STRLEN).is_none() {
            let fn_ty = i64_ty.fn_type(&[ptr_ty.into()], false);
            ctx.module.add_function(ffi_names::STRLEN, fn_ty, None);
        }

        // memcpy: ptr memcpy(ptr, ptr, i64)
        if ctx.module.get_function(ffi_names::MEMCPY).is_none() {
            let fn_ty = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false);
            ctx.module.add_function(ffi_names::MEMCPY, fn_ty, None);
        }

        // memset: ptr memset(ptr, i32, i64)
        if ctx.module.get_function(ffi_names::MEMSET).is_none() {
            let fn_ty = ptr_ty.fn_type(&[ptr_ty.into(), i32_ty.into(), i64_ty.into()], false);
            ctx.module.add_function(ffi_names::MEMSET, fn_ty, None);
        }

        // strcmp: i32 strcmp(ptr, ptr)
        if ctx.module.get_function(ffi_names::STRCMP).is_none() {
            let fn_ty = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            ctx.module.add_function(ffi_names::STRCMP, fn_ty, None);
        }

        // printf: i32 printf(ptr, ...)
        if ctx.module.get_function(ffi_names::PRINTF).is_none() {
            let fn_ty = i32_ty.fn_type(&[ptr_ty.into()], true); // variadic
            ctx.module.add_function(ffi_names::PRINTF, fn_ty, None);
        }

        // snprintf: i32 snprintf(ptr, i64, ptr, ...)
        if ctx.module.get_function(ffi_names::SNPRINTF).is_none() {
            let fn_ty = i32_ty.fn_type(&[ptr_ty.into(), i64_ty.into(), ptr_ty.into()], true);
            ctx.module.add_function(ffi_names::SNPRINTF, fn_ty, None);
        }

        // puts: i32 puts(ptr)
        if ctx.module.get_function(ffi_names::PUTS).is_none() {
            let fn_ty = i32_ty.fn_type(&[ptr_ty.into()], false);
            ctx.module.add_function(ffi_names::PUTS, fn_ty, None);
        }

        // putchar: i32 putchar(i32)
        if ctx.module.get_function(ffi_names::PUTCHAR).is_none() {
            let fn_ty = i32_ty.fn_type(&[i32_ty.into()], false);
            ctx.module.add_function(ffi_names::PUTCHAR, fn_ty, None);
        }
    }
}
