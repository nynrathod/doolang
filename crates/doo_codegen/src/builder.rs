//! Codegen Builder - Main entry point for code generation
//!
//! Orchestrates MIR -> LLVM IR translation.

use crate::context::CodegenContext;
use crate::instructions::InstructionDispatcher;
use crate::utils::operand_to_value;
use doo_core::types::{builtin, TypeKind, TypeRegistry};
use doo_mir::{MirBlock, MirConst, MirFunction, MirInstr, MirOperand, MirProgram, MirTerminator};
use inkwell::basic_block::BasicBlock;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::types::BasicTypeEnum;
use inkwell::values::BasicValueEnum;
use std::collections::HashMap;
use std::sync::Arc;

/// Get the variable name from a MirOperand.
fn get_operand_name(operand: &MirOperand) -> Option<&str> {
    match operand {
        MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
            Some(name.as_str())
        }
        MirOperand::Const(_) => None,
    }
}

/// Convert a return value to the expected function return type.
/// Handles cases where JSON.parse returns a pointer but the function expects Int/Float/Bool.
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

    // Get the expected type kind
    let expected_kind = match ctx.get_type_kind(expected_type) {
        Some(k) => k,
        None => return val,
    };

    // If value is already the right type, return as-is
    // Only convert when we have a pointer but expect a primitive
    if !val.is_pointer_value() {
        return val;
    }

    let ptr = val.into_pointer_value();

    match expected_kind {
        TypeKind::Int => {
            // Load i64 from pointer
            if let Ok(loaded) = ctx
                .builder
                .build_load(ctx.context.i64_type(), ptr, "ret_int")
            {
                return loaded;
            }
        }
        TypeKind::Float => {
            // Load f64 from pointer
            if let Ok(loaded) = ctx
                .builder
                .build_load(ctx.context.f64_type(), ptr, "ret_float")
            {
                return loaded;
            }
        }
        TypeKind::Bool => {
            // Load i1 from pointer
            if let Ok(loaded) = ctx
                .builder
                .build_load(ctx.context.bool_type(), ptr, "ret_bool")
            {
                return loaded;
            }
        }
        _ => {
            // For other types (Str, Array, Map, etc.), pointer is the correct return type
        }
    }

    // If conversion failed, return original value
    BasicValueEnum::PointerValue(ptr)
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

        let dispatcher = InstructionDispatcher::new();

        // Pre-pass: declare all struct types from the type registry
        // This ensures struct types are available for FieldGet/FieldSet in methods
        // that receive structs as parameters (like 'self')
        self.declare_struct_types(&mut ctx);

        // First pass: declare all functions
        for func in &mir.functions {
            self.declare_function(&mut ctx, func);
        }

        // Second pass: generate function bodies
        for func in &mir.functions {
            self.generate_function(&mut ctx, func, &dispatcher);
        }

        ctx.module
    }

    /// Pre-declare all struct types from the type registry.
    /// This ensures struct types are cached before we try to access their fields.
    fn declare_struct_types(&self, ctx: &mut CodegenContext<'ctx>) {
        let debug = std::env::var("DOO_DEBUG").is_ok();

        // First, collect all struct information from the registry
        let structs: Vec<(String, Vec<(String, doo_core::types::TypeId)>)> = ctx
            .type_registry
            .all_type_ids()
            .filter_map(|type_id| {
                if let Some(type_info) = ctx.type_registry.get(type_id) {
                    if let doo_core::types::TypeKind::Struct { name, fields } = &type_info.kind {
                        return Some((name.clone(), fields.clone()));
                    }
                }
                None
            })
            .collect();

        // Now process each struct with mutable access to ctx
        for (name, fields) in structs {
            // Build LLVM field types
            let field_types: Vec<inkwell::types::BasicTypeEnum> = fields
                .iter()
                .map(|(_, field_type_id)| ctx.get_llvm_type(*field_type_id))
                .collect();

            // Cache the struct type
            let _struct_type = ctx.get_struct_type(&name, &field_types);

            // Also register struct metadata (field names)
            let field_names: Vec<String> = fields.iter().map(|(n, _)| n.clone()).collect();
            ctx.register_struct_metadata(&name, field_names);

            if debug {
                eprintln!(
                    "[CODEGEN] Pre-declared struct type: {} with {} fields",
                    name,
                    fields.len()
                );
            }
        }
    }

    /// Declare a function (signature only, no body).
    fn declare_function(&self, ctx: &mut CodegenContext<'ctx>, func: &MirFunction) {
        let debug = std::env::var("DOO_DEBUG").is_ok();
        if debug {
            eprintln!(
                "[CODEGEN] Declaring function {} with return_type={:?}",
                func.name, func.return_type
            );
        }

        // Build parameter types
        let param_types: Vec<BasicTypeEnum<'ctx>> = func
            .params
            .iter()
            .map(|p| ctx.get_llvm_type(p.type_id))
            .collect();

        // Collect Doo TypeIds for parameter types (for argument coercion)
        let param_type_ids: Vec<_> = func.params.iter().map(|p| p.type_id).collect();
        ctx.register_function_param_types(&func.name, param_type_ids);

        // Build return type
        // If function has an error_type, it returns a Result struct { i32, ptr }
        let return_type = if func.error_type.is_some() {
            // Result type: { i32 tag, ptr payload }
            let result_struct_type = ctx.context.struct_type(
                &[ctx.context.i32_type().into(), ctx.ptr_type().into()],
                false,
            );
            Some(result_struct_type.into())
        } else {
            func.return_type.map(|t| ctx.get_llvm_type(t))
        };

        // For FFI functions, use external linkage
        if func.ffi.is_some() {
            let param_meta: Vec<_> = param_types.iter().map(|t| (*t).into()).collect();
            let fn_type = match return_type {
                Some(ret) => {
                    use inkwell::types::BasicType;
                    ret.fn_type(&param_meta, false)
                }
                None => ctx.context.void_type().fn_type(&param_meta, false),
            };
            ctx.module
                .add_function(&func.name, fn_type, Some(Linkage::External));
        } else {
            ctx.declare_function(&func.name, &param_types, return_type);
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

        let llvm_func = match ctx.get_function(&func.name) {
            Some(f) => f,
            None => return, // Function not declared
        };

        // Clear locals for this function
        ctx.clear_locals();

        // Set current function's return type for proper return value conversion
        ctx.current_function_return_type = func.return_type;

        if std::env::var("DOO_DEBUG").is_ok() {
            eprintln!(
                "[CODEGEN] Function {} has {} MIR locals:",
                func.name,
                func.locals.len()
            );
            for local in &func.locals {
                eprintln!("[CODEGEN]   Local: {} : {:?}", local.name, local.type_id);
            }
        }

        // Collect all assigned variables from MIR to create allocas
        let mut assigned_vars: HashMap<String, doo_core::types::TypeId> = HashMap::new();
        for block in &func.blocks {
            for instr in &block.instructions {
                if let doo_mir::MirInstrKind::Assign { dest, value } = &instr.kind {
                    // Infer type from value
                    let type_id = match value {
                        MirOperand::Const(MirConst::Int(_)) => builtin::INT,
                        MirOperand::Const(MirConst::Float(_)) => builtin::FLOAT,
                        MirOperand::Const(MirConst::Bool(_)) => builtin::BOOL,
                        MirOperand::Const(MirConst::Str(_)) => builtin::STR,
                        MirOperand::Const(MirConst::Nil) => builtin::ANY,
                        MirOperand::Local(name) | MirOperand::Temp(name) => {
                            // Get type from existing var if known
                            assigned_vars.get(name).copied().unwrap_or(builtin::ANY)
                        }
                        MirOperand::Global(_) => builtin::ANY,
                    };
                    assigned_vars.insert(dest.clone(), type_id);
                }
            }
        }

        // Create LLVM basic blocks for each MIR block
        let mut block_map: HashMap<String, BasicBlock<'ctx>> = HashMap::new();
        for mir_block in &func.blocks {
            let bb = ctx.context.append_basic_block(llvm_func, &mir_block.label);
            block_map.insert(mir_block.label.clone(), bb);
        }

        // Position at entry block for allocas
        let entry_bb = block_map
            .get("entry")
            .copied()
            .unwrap_or_else(|| ctx.context.append_basic_block(llvm_func, "entry"));
        ctx.builder.position_at_end(entry_bb);

        // Create allocas for parameters
        for (i, param) in func.params.iter().enumerate() {
            let param_value = llvm_func.get_nth_param(i as u32).unwrap();
            let param_type = ctx.get_llvm_type(param.type_id);
            let alloca = ctx.create_local(&param.name, param_type);
            ctx.builder.build_store(alloca, param_value).unwrap();
            // Track parameter type for Clone/Drop
            ctx.set_variable_type(&param.name, param.type_id);

            // CRITICAL: For struct parameters, register the struct type association
            // This is needed for FieldGet/FieldSet to work correctly when the parameter
            // is passed to another function or when its fields are accessed
            if let Some(TypeKind::Struct { name, .. }) = ctx.get_type_kind(param.type_id) {
                ctx.set_temp_struct_type(&param.name, &name);
                if std::env::var("DOO_DEBUG").is_ok() {
                    eprintln!(
                        "[CODEGEN] Registered struct param {} as type {}",
                        param.name, name
                    );
                }
            }
        }

        // Create allocas for local variables from MIR
        // Skip if a variable with this name already exists (e.g., from parameters)
        for local in &func.locals {
            if ctx.get_local(&local.name).is_none() {
                let local_type = ctx.get_llvm_type(local.type_id);
                ctx.create_local(&local.name, local_type);
                // Track local type for Clone/Drop
                ctx.set_variable_type(&local.name, local.type_id);
            }
        }

        // Create allocas for all assigned variables (for proper loop handling)
        for (name, type_id) in &assigned_vars {
            if ctx.get_local(name).is_none() {
                let var_type = ctx.get_llvm_type(*type_id);
                ctx.create_local(name, var_type);
                // Track assigned variable type for Clone/Drop
                ctx.set_variable_type(name, *type_id);
                if std::env::var("DOO_DEBUG").is_ok() {
                    eprintln!("[CODEGEN] Created alloca for variable: {}", name);
                }
            }
        }

        // Generate code for each block
        for mir_block in &func.blocks {
            let bb = block_map[&mir_block.label];
            ctx.builder.position_at_end(bb);

            if std::env::var("DOO_DEBUG").is_ok() {
                eprintln!(
                    "[CODEGEN] Block {} has {} instructions",
                    mir_block.label,
                    mir_block.instructions.len()
                );
            }

            // Generate instructions
            for instr in &mir_block.instructions {
                if std::env::var("DOO_DEBUG").is_ok() {
                    eprintln!("[CODEGEN]   Instr: {:?}", instr);
                }
                // Use custom emit for Assign to store in alloca
                if let doo_mir::MirInstrKind::Assign { dest, value } = &instr.kind {
                    if let Some(val) = operand_to_value(ctx, value) {
                        // Propagate struct type association if present
                        if let Some(src_name) = get_operand_name(value) {
                            if let Some(struct_name) = ctx.get_temp_struct_type(src_name).cloned() {
                                ctx.set_temp_struct_type(dest, &struct_name);
                            }
                        }
                        if let Some(ptr) = ctx.get_local(dest) {
                            // Store to alloca
                            ctx.builder.build_store(ptr, val).ok();
                        } else {
                            // Fallback to temp
                            ctx.set_temp(dest, val);
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
                    if std::env::var("DOO_DEBUG").is_ok() {
                        eprintln!(
                            "[CODEGEN] Block {:?} has no terminator, adding default return",
                            bb.get_name().to_str()
                        );
                    }
                    ctx.builder.position_at_end(bb);

                    // Check if this is a Result-returning function (has error_type).
                    // Result functions return { i32 tag, ptr payload } struct.
                    if func.error_type.is_some() {
                        // Create a default Result struct: { 0 (Ok tag), null (no payload) }
                        let result_struct_type = ctx.context.struct_type(
                            &[ctx.context.i32_type().into(), ctx.ptr_type().into()],
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
                    } else {
                        let ret_type = ctx.get_llvm_type(func.return_type.unwrap());
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
        if std::env::var("DOO_DEBUG").is_ok() {
            eprintln!("[CODEGEN] Emitting terminator: {:?}", term);
        }

        match term {
            MirTerminator::Return { values } => {
                // Check if we're in main function - main returns i32 0
                let is_main = ctx
                    .builder
                    .get_insert_block()
                    .and_then(|bb| bb.get_parent())
                    .map(|f| f.get_name().to_str().unwrap_or("") == "main")
                    .unwrap_or(false);

                let debug = std::env::var("DOO_DEBUG").is_ok();

                if values.is_empty() {
                    if is_main {
                        // Main function must return i32 0
                        let zero = ctx.context.i32_type().const_int(0, false);
                        ctx.builder.build_return(Some(&zero)).ok();
                    } else {
                        ctx.builder.build_return(None).ok();
                    }
                } else if values.len() == 1 {
                    if let Some(val) = operand_to_value(ctx, &values[0]) {
                        if debug {
                            eprintln!(
                                "[CODEGEN] Return: got value {:?} for {:?}",
                                val.get_type(),
                                &values[0]
                            );
                        }
                        if is_main {
                            // Main function must return i32
                            let zero = ctx.context.i32_type().const_int(0, false);
                            ctx.builder.build_return(Some(&zero)).ok();
                        } else {
                            // Convert value to expected return type if needed
                            let final_val = convert_return_value(ctx, val);
                            ctx.builder.build_return(Some(&final_val)).ok();
                        }
                    } else if is_main {
                        if debug {
                            eprintln!("[CODEGEN] Return: no value for main, returning 0");
                        }
                        let zero = ctx.context.i32_type().const_int(0, false);
                        ctx.builder.build_return(Some(&zero)).ok();
                    } else {
                        if debug {
                            eprintln!("[CODEGEN] WARNING: Return: operand_to_value returned None for {:?}", &values[0]);
                        }
                        ctx.builder.build_return(None).ok();
                    }
                } else {
                    // Multiple return values - return as tuple
                    if is_main {
                        let zero = ctx.context.i32_type().const_int(0, false);
                        ctx.builder.build_return(Some(&zero)).ok();
                    } else {
                        // Convert all values to LLVM values
                        let llvm_values: Vec<_> = values
                            .iter()
                            .filter_map(|v| operand_to_value(ctx, v))
                            .collect();

                        if llvm_values.len() != values.len() {
                            // Some values couldn't be converted
                            if std::env::var("DOO_DEBUG").is_ok() {
                                eprintln!(
                                    "[CODEGEN] WARNING: Could not convert all tuple return values"
                                );
                            }
                            ctx.builder.build_return(None).ok();
                        } else {
                            // Get element types
                            let element_types: Vec<_> =
                                llvm_values.iter().map(|v| v.get_type()).collect();

                            // Create tuple struct type
                            let tuple_type = ctx.context.struct_type(&element_types, false);

                            // Allocate space for tuple on heap (so pointer is valid after return)
                            let ptr_type = ctx.context.ptr_type(inkwell::AddressSpace::default());
                            let size = tuple_type.size_of().unwrap();
                            let malloc_fn = ctx.get_function("malloc").unwrap_or_else(|| {
                                let fn_type =
                                    ptr_type.fn_type(&[ctx.context.i64_type().into()], false);
                                ctx.module.add_function(
                                    "malloc",
                                    fn_type,
                                    Some(inkwell::module::Linkage::External),
                                )
                            });
                            let tuple_ptr = ctx
                                .builder
                                .build_call(malloc_fn, &[size.into()], "tuple_alloc")
                                .ok()
                                .and_then(|call| call.try_as_basic_value().left())
                                .map(|v| v.into_pointer_value());

                            if let Some(tuple_ptr) = tuple_ptr {
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
            }
            MirTerminator::Goto { target } => {
                if let Some(target_bb) = block_map.get(target) {
                    ctx.builder.build_unconditional_branch(*target_bb).ok();
                } else if std::env::var("DOO_DEBUG").is_ok() {
                    eprintln!("[CODEGEN] ERROR: Goto target {} not found", target);
                }
            }
            MirTerminator::Branch {
                cond,
                then_block,
                else_block,
            } => {
                if std::env::var("DOO_DEBUG").is_ok() {
                    eprintln!("[CODEGEN] Branch cond: {:?}", cond);
                    if let Some(bb) = ctx.builder.get_insert_block() {
                        eprintln!(
                            "[CODEGEN] Branch emitting from block: {:?}",
                            bb.get_name().to_str()
                        );
                    }
                }
                if let Some(cond_val) = operand_to_value(ctx, cond) {
                    if std::env::var("DOO_DEBUG").is_ok() {
                        eprintln!("[CODEGEN] Branch cond_val: {:?}", cond_val);
                    }
                    let cond_bool = if cond_val.is_int_value() {
                        cond_val.into_int_value()
                    } else {
                        // Convert to bool (non-zero = true)
                        ctx.context.bool_type().const_int(1, false)
                    };

                    if let (Some(then_bb), Some(else_bb)) =
                        (block_map.get(then_block), block_map.get(else_block))
                    {
                        if std::env::var("DOO_DEBUG").is_ok() {
                            eprintln!(
                                "[CODEGEN] Building conditional branch to {} / {}",
                                then_block, else_block
                            );
                        }
                        let result = ctx
                            .builder
                            .build_conditional_branch(cond_bool, *then_bb, *else_bb);
                        if std::env::var("DOO_DEBUG").is_ok() {
                            eprintln!(
                                "[CODEGEN] build_conditional_branch result: {:?}",
                                result.is_ok()
                            );
                            if let Err(e) = &result {
                                eprintln!("[CODEGEN] build_conditional_branch error: {:?}", e);
                            }
                        }
                        result.ok();
                    } else if std::env::var("DOO_DEBUG").is_ok() {
                        eprintln!(
                            "[CODEGEN] ERROR: Branch targets not found: {} or {}",
                            then_block, else_block
                        );
                    }
                } else if std::env::var("DOO_DEBUG").is_ok() {
                    eprintln!("[CODEGEN] ERROR: Branch condition not found for {:?}", cond);
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
                        let default_bb = block_map
                            .get(default)
                            .copied()
                            .unwrap_or_else(|| ctx.builder.get_insert_block().unwrap());

                        let llvm_cases: Vec<_> = cases
                            .iter()
                            .filter_map(|(case_val, target)| {
                                block_map.get(target).map(|bb| {
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
        let i8_ty = self.context.i8_type();
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
