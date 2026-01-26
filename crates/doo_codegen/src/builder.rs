//! Codegen Builder - Main entry point for code generation
//!
//! Orchestrates MIR -> LLVM IR translation.

use crate::context::CodegenContext;
use crate::instructions::InstructionDispatcher;
use crate::utils::operand_to_value;
use doo_core::types::{builtin, TypeRegistry};
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

    /// Declare a function (signature only, no body).
    fn declare_function(&self, ctx: &mut CodegenContext<'ctx>, func: &MirFunction) {
        // Build parameter types
        let param_types: Vec<BasicTypeEnum<'ctx>> = func
            .params
            .iter()
            .map(|p| ctx.get_llvm_type(p.type_id))
            .collect();

        // Build return type
        let return_type = func.return_type.map(|t| ctx.get_llvm_type(t));

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
        }

        // Create allocas for local variables from MIR
        for local in &func.locals {
            let local_type = ctx.get_llvm_type(local.type_id);
            ctx.create_local(&local.name, local_type);
            // Track local type for Clone/Drop
            ctx.set_variable_type(&local.name, local.type_id);
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

        // Ensure all blocks have terminators
        for mir_block in &func.blocks {
            let bb = block_map[&mir_block.label];
            if bb.get_terminator().is_none() {
                ctx.builder.position_at_end(bb);
                if func.return_type.is_none() {
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
                let is_main = ctx.builder.get_insert_block()
                    .and_then(|bb| bb.get_parent())
                    .map(|f| f.get_name().to_str().unwrap_or("") == "main")
                    .unwrap_or(false);

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
                        if is_main {
                            // Main function must return i32
                            let zero = ctx.context.i32_type().const_int(0, false);
                            ctx.builder.build_return(Some(&zero)).ok();
                        } else {
                            ctx.builder.build_return(Some(&val)).ok();
                        }
                    } else if is_main {
                        let zero = ctx.context.i32_type().const_int(0, false);
                        ctx.builder.build_return(Some(&zero)).ok();
                    } else {
                        ctx.builder.build_return(None).ok();
                    }
                } else {
                    // Multiple return values - TODO: return as tuple
                    if is_main {
                        let zero = ctx.context.i32_type().const_int(0, false);
                        ctx.builder.build_return(Some(&zero)).ok();
                    } else {
                        ctx.builder.build_return(None).ok();
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
                        ctx.builder
                            .build_conditional_branch(cond_bool, *then_bb, *else_bb)
                            .ok();
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
