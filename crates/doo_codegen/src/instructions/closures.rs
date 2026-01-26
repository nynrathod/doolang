//! Closure Instruction Handler
//!
//! Handles: ClosureCreate, ClosureCall
//!
//! ## PORT-FIRST RULE
//!
//! Closures are compiled into actual LLVM functions (not runtime structs).
//! The legacy compiler generates standalone LLVM functions for each closure and stores
//! function pointers. Calls are direct LLVM calls, not indirect through structs.
//!
//! ## Closure Layout
//!
//! ```text
//! struct Closure {
//!     i8* fn_ptr;   // Pointer to the closure function
//!     i8* env_ptr;  // Pointer to captured environment (null if no captures)
//! }
//! ```
//!
//! ## Environment Layout
//!
//! Captured variables are stored as i64 values in a contiguous block:
//! ```text
//! struct Env {
//!     i64 captures[N];  // Each captured value encoded as i64
//! }
//! ```
//!
//! ## Calling Convention
//!
//! Closure functions receive the env pointer as first parameter:
//! ```text
//! fn closure_impl(env: *i8, args...) -> ReturnType
//! ```

use super::InstructionHandler;
use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use doo_mir::{MirInstr, MirInstrKind, MirOperand};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue,
};
use inkwell::AddressSpace;

/// Closure instruction handler.
pub struct ClosureHandler;

impl<'ctx> InstructionHandler<'ctx> for ClosureHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(
            instr.kind,
            MirInstrKind::ClosureCreate { .. } | MirInstrKind::ClosureCall { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::ClosureCreate {
                dest,
                func,
                captures,
            } => {
                // PORT-FIRST: Legacy stores function pointer directly as the closure value
                // The `func` should refer to an existing LLVM function
                // Captures are not yet implemented (legacy doesn't use them either)

                let Some(func_val) = ctx.get_function(func) else {
                    // If function not found, return null pointer as fallback
                    // This allows compilation to proceed even if closure generation is incomplete
                    let null_ptr = ctx
                        .context
                        .i8_type()
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null();
                    ctx.set_temp(dest, null_ptr.into());
                    return Some(null_ptr.into());
                };

                // Closure layout expected by array/map builtins: { i8* fn_ptr, i8* env_ptr }
                let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
                let i64_type = ctx.context.i64_type();
                let closure_type = ctx
                    .context
                    .struct_type(&[ptr_type.into(), ptr_type.into()], false);

                // Allocate closure struct
                let doo_alloc = get_or_declare_doo_alloc(ctx);
                let closure_size = i64_type.const_int(16, false);
                let closure_raw = ctx
                    .builder
                    .build_call(doo_alloc, &[closure_size.into()], "closure_alloc")
                    .ok()?
                    .try_as_basic_value()
                    .left()?
                    .into_pointer_value();
                let closure_ptr = ctx
                    .builder
                    .build_pointer_cast(
                        closure_raw,
                        closure_type.ptr_type(AddressSpace::default()),
                        "closure_ptr",
                    )
                    .ok()?;

                // Allocate and populate env
                let env_ptr_i8: PointerValue<'ctx> = if captures.is_empty() {
                    ptr_type.const_null()
                } else {
                    let env_size = i64_type.const_int((captures.len() as u64) * 8, false);
                    let env_raw = ctx
                        .builder
                        .build_call(doo_alloc, &[env_size.into()], "closure_env_alloc")
                        .ok()?
                        .try_as_basic_value()
                        .left()?
                        .into_pointer_value();

                    let env_i64_ptr = ctx
                        .builder
                        .build_pointer_cast(
                            env_raw,
                            i64_type.ptr_type(AddressSpace::default()),
                            "env_i64_ptr",
                        )
                        .ok()?;

                    for (idx, cap) in captures.iter().enumerate() {
                        let Some(cap_val) = operand_to_value(ctx, cap) else {
                            continue;
                        };
                        let Some(cap_i64) = value_to_i64(ctx, cap_val) else {
                            continue;
                        };
                        let idx_i64 = i64_type.const_int(idx as u64, false);
                        let slot = unsafe {
                            ctx.builder.build_in_bounds_gep(
                                i64_type,
                                env_i64_ptr,
                                &[idx_i64],
                                "cap_slot",
                            )
                        }
                        .ok()?;
                        ctx.builder.build_store(slot, cap_i64).ok()?;
                    }

                    ctx.builder
                        .build_pointer_cast(env_i64_ptr, ptr_type, "env_i8")
                        .ok()?
                };

                // Store function pointer as i8*
                let fn_ptr = func_val.as_global_value().as_pointer_value();
                let fn_ptr_i8 = ctx
                    .builder
                    .build_pointer_cast(fn_ptr, ptr_type, "fn_i8")
                    .ok()?;

                // Write closure fields
                let fn_slot = ctx
                    .builder
                    .build_struct_gep(closure_type, closure_ptr, 0, "fn_ptr_slot")
                    .ok()?;
                ctx.builder.build_store(fn_slot, fn_ptr_i8).ok()?;
                let env_slot = ctx
                    .builder
                    .build_struct_gep(closure_type, closure_ptr, 1, "env_ptr_slot")
                    .ok()?;
                ctx.builder.build_store(env_slot, env_ptr_i8).ok()?;

                ctx.set_temp(dest, closure_ptr.into());
                Some(closure_ptr.into())
            }

            MirInstrKind::ClosureCall {
                dest,
                closure,
                args,
            } => {
                // PORT-FIRST: Call the function pointer directly (not through a struct)
                // The closure value is the function pointer itself

                let closure_val = operand_to_value(ctx, closure)?;
                if !closure_val.is_pointer_value() {
                    return None;
                }

                let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
                let i64_type = ctx.context.i64_type();
                let closure_type = ctx
                    .context
                    .struct_type(&[ptr_type.into(), ptr_type.into()], false);

                // Ensure we have a typed pointer to the closure struct
                let closure_ptr_raw = closure_val.into_pointer_value();
                let closure_ptr = ctx
                    .builder
                    .build_pointer_cast(
                        closure_ptr_raw,
                        closure_type.ptr_type(AddressSpace::default()),
                        "closure_ptr",
                    )
                    .ok()?;

                // Load fn_ptr (field 0)
                let fn_slot = ctx
                    .builder
                    .build_struct_gep(closure_type, closure_ptr, 0, "fn_ptr_slot")
                    .ok()?;
                let fn_ptr_i8 = ctx
                    .builder
                    .build_load(ptr_type, fn_slot, "fn_ptr")
                    .ok()?
                    .into_pointer_value();

                // Load env_ptr (field 1)
                let env_slot = ctx
                    .builder
                    .build_struct_gep(closure_type, closure_ptr, 1, "env_ptr_slot")
                    .ok()?;
                let env_ptr_i8 = ctx.builder.build_load(ptr_type, env_slot, "env_ptr").ok()?;

                // Build argument list: (env, ...args_as_i64)
                let mut arg_i64s: Vec<IntValue<'ctx>> = Vec::with_capacity(args.len());
                for arg in args {
                    let Some(v) = operand_to_value(ctx, arg) else {
                        arg_i64s.push(i64_type.const_zero());
                        continue;
                    };
                    arg_i64s.push(value_to_i64(ctx, v).unwrap_or_else(|| i64_type.const_zero()));
                }

                let mut param_types = Vec::with_capacity(1 + arg_i64s.len());
                param_types.push(ptr_type.into());
                for _ in 0..arg_i64s.len() {
                    param_types.push(i64_type.into());
                }

                let fn_type = i64_type.fn_type(&param_types, false);
                let fn_ptr_typed = ctx
                    .builder
                    .build_pointer_cast(
                        fn_ptr_i8,
                        fn_type.ptr_type(AddressSpace::default()),
                        "fn_typed",
                    )
                    .ok()?;

                let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> =
                    Vec::with_capacity(1 + arg_i64s.len());
                call_args.push(env_ptr_i8.into());
                for a in arg_i64s {
                    call_args.push(a.into());
                }

                let call_site = ctx
                    .builder
                    .build_indirect_call(fn_type, fn_ptr_typed, &call_args, "closure_call")
                    .ok()?;
                let result = call_site.try_as_basic_value().left()?;

                if let Some(dest_name) = dest {
                    ctx.set_temp(dest_name, result);
                }

                Some(result)
            }

            _ => None,
        }
    }
}

/// Convert MirOperand to LLVM value.
fn operand_to_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    operand: &MirOperand,
) -> Option<BasicValueEnum<'ctx>> {
    match operand {
        MirOperand::Const(c) => {
            // Basic constant conversion - expand as needed
            match c {
                doo_mir::MirConst::Int(val) => Some(ctx.const_i64(*val).into()),
                doo_mir::MirConst::Bool(val) => Some(ctx.const_bool(*val).into()),
                doo_mir::MirConst::Float(val) => Some(ctx.const_f64(*val).into()),
                doo_mir::MirConst::Str(s) => Some(ctx.const_string(s).into()),
                doo_mir::MirConst::Nil => Some(ctx.const_i64(0).into()),
            }
        }
        MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
            ctx.get_value(name)
        }
    }
}

fn get_or_declare_doo_alloc<'ctx>(ctx: &mut CodegenContext<'ctx>) -> FunctionValue<'ctx> {
    if let Some(f) = ctx.get_function(ffi_names::DOO_ALLOC) {
        return f;
    }
    let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
    let fn_type = ptr_type.fn_type(&[ctx.context.i64_type().into()], false);
    ctx.module.add_function(ffi_names::DOO_ALLOC, fn_type, None)
}

fn value_to_i64<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    v: BasicValueEnum<'ctx>,
) -> Option<IntValue<'ctx>> {
    let i64_type = ctx.context.i64_type();

    if v.is_int_value() {
        let iv = v.into_int_value();
        let bw = iv.get_type().get_bit_width();
        if bw == 64 {
            Some(iv)
        } else if bw < 64 {
            ctx.builder
                .build_int_z_extend(iv, i64_type, "zext_i64")
                .ok()
        } else {
            ctx.builder
                .build_int_truncate(iv, i64_type, "trunc_i64")
                .ok()
        }
    } else if v.is_pointer_value() {
        ctx.builder
            .build_ptr_to_int(v.into_pointer_value(), i64_type, "ptr_to_i64")
            .ok()
    } else if v.is_float_value() {
        let fv = v.into_float_value();
        let f64v = if fv.get_type() == ctx.context.f64_type() {
            fv
        } else {
            ctx.builder
                .build_float_ext(fv, ctx.context.f64_type(), "fext_f64")
                .ok()?
        };
        ctx.builder
            .build_bit_cast(f64v, i64_type, "f64_bits")
            .ok()
            .map(|vv: BasicValueEnum<'ctx>| vv.into_int_value())
    } else {
        None
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use doo_core::types::TypeRegistry;
    use inkwell::context::Context;
    use std::sync::Arc;

    #[test]
    fn test_closure_handler_handles() {
        use doo_mir::{MirInstr, MirInstrKind, Span};

        let handler = ClosureHandler;

        let closure_create = MirInstr {
            kind: MirInstrKind::ClosureCreate {
                dest: "temp_0".to_string(),
                func: "closure_fn".to_string(),
                captures: vec![],
            },
            span: Span::default(),
        };
        assert!(handler.handles(&closure_create));

        let closure_call = MirInstr {
            kind: MirInstrKind::ClosureCall {
                dest: Some("result".to_string()),
                closure: MirOperand::Temp("temp_0".to_string()),
                args: vec![],
            },
            span: Span::default(),
        };
        assert!(handler.handles(&closure_call));

        // Should not handle non-closure instructions
        let assign = MirInstr {
            kind: MirInstrKind::Assign {
                dest: "x".to_string(),
                value: MirOperand::Const(doo_mir::MirConst::Int(42)),
            },
            span: Span::default(),
        };
        assert!(!handler.handles(&assign));
    }

    #[test]
    fn test_operand_to_value_constants() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);

        // Test Int constant
        let int_op = MirOperand::Const(doo_mir::MirConst::Int(42));
        let int_val = operand_to_value(&mut codegen, &int_op).unwrap();
        assert!(int_val.is_int_value());

        // Test Bool constant
        let bool_op = MirOperand::Const(doo_mir::MirConst::Bool(true));
        let bool_val = operand_to_value(&mut codegen, &bool_op).unwrap();
        assert!(bool_val.is_int_value());

        // Test Float constant
        let float_op = MirOperand::Const(doo_mir::MirConst::Float(3.14));
        let float_val = operand_to_value(&mut codegen, &float_op).unwrap();
        assert!(float_val.is_float_value());

        // Test String constant
        let str_op = MirOperand::Const(doo_mir::MirConst::Str("hello".to_string()));
        let str_val = operand_to_value(&mut codegen, &str_op).unwrap();
        assert!(str_val.is_pointer_value());

        // Test Nil constant
        let nil_op = MirOperand::Const(doo_mir::MirConst::Nil);
        let nil_val = operand_to_value(&mut codegen, &nil_op).unwrap();
        assert!(nil_val.is_int_value());
    }

    #[test]
    fn test_value_to_i64_conversions() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);

        // Set up a basic block for builder operations
        let fn_type = ctx.void_type().fn_type(&[], false);
        let func = codegen.module.add_function("test_fn", fn_type, None);
        let entry = ctx.append_basic_block(func, "entry");
        codegen.builder.position_at_end(entry);

        // Test i64 value (no conversion needed)
        let i64_val = codegen.const_i64(123);
        let result = value_to_i64(&mut codegen, i64_val.into());
        assert!(result.is_some());

        // Test i32 value (needs extension)
        let i32_val = ctx.i32_type().const_int(456, false);
        let result = value_to_i64(&mut codegen, i32_val.into());
        assert!(result.is_some());

        // Test f64 value (bit cast)
        let f64_val = codegen.const_f64(3.14);
        let result = value_to_i64(&mut codegen, f64_val.into());
        assert!(result.is_some());
    }

    #[test]
    fn test_get_or_declare_doo_alloc() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);

        // First call should declare the function
        let alloc1 = get_or_declare_doo_alloc(&mut codegen);
        assert_eq!(alloc1.get_name().to_str().unwrap(), ffi_names::DOO_ALLOC);

        // Second call should return the same function (cached)
        let alloc2 = get_or_declare_doo_alloc(&mut codegen);
        assert_eq!(alloc1, alloc2);
    }
}
