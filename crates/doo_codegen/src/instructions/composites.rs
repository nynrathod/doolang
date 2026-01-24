//! Composite/Aggregate Instruction Handler
//!
//! Handles: TupleCreate, TupleGet, StructCreate, FieldGet, FieldSet

use inkwell::values::{BasicValueEnum};
use doo_mir::{MirInstr, MirInstrKind};
use crate::context::CodegenContext;
use crate::utils::{operand_to_value};
use super::InstructionHandler;

/// Composite instruction handler.
pub struct CompositeHandler;

impl<'ctx> InstructionHandler<'ctx> for CompositeHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(instr.kind,
            MirInstrKind::TupleCreate { .. } |
            MirInstrKind::TupleGet { .. } |
            MirInstrKind::StructCreate { .. } |
            MirInstrKind::FieldGet { .. } |
            MirInstrKind::FieldSet { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::TupleCreate { dest, elements } => {
                let types: Vec<_> = elements.iter()
                    .map(|_| ctx.i64_type().into())
                    .collect();
                let tuple_type = ctx.context.struct_type(&types, false);
                let alloca = ctx.builder.build_alloca(tuple_type, dest).ok()?;
                
                for (i, elem) in elements.iter().enumerate() {
                    if let Some(val) = operand_to_value(ctx, elem) {
                        if let Ok(ptr) = ctx.builder.build_struct_gep(tuple_type, alloca, i as u32, "field_ptr") {
                            ctx.builder.build_store(ptr, val).ok();
                        }
                    }
                }
                
                ctx.set_temp(dest, alloca.into());
                Some(alloca.into())
            }

            MirInstrKind::TupleGet { dest, tuple, index } => {
                if let Some(tup) = operand_to_value(ctx, tuple) {
                    if tup.is_pointer_value() {
                        let ptr = tup.into_pointer_value();
                        let tuple_ty = ctx.context.struct_type(&[ctx.i64_type().into()], false);
                        if let Ok(field_ptr) = ctx.builder.build_struct_gep(tuple_ty, ptr, *index as u32, "field") {
                            if let Ok(val) = ctx.builder.build_load(ctx.i64_type(), field_ptr, dest) {
                                ctx.set_temp(dest, val);
                                return Some(val);
                            }
                        }
                    }
                }
                None
            }

            MirInstrKind::StructCreate { dest, struct_name, fields } => {
                let field_types: Vec<_> = fields.iter()
                    .map(|_| ctx.i64_type().into())
                    .collect();
                let struct_type = ctx.get_struct_type(struct_name, &field_types);
                let alloca = ctx.builder.build_alloca(struct_type, dest).ok()?;
                
                for (i, (_, value)) in fields.iter().enumerate() {
                    if let Some(val) = operand_to_value(ctx, value) {
                        if let Ok(ptr) = ctx.builder.build_struct_gep(struct_type, alloca, i as u32, "field_ptr") {
                            ctx.builder.build_store(ptr, val).ok();
                        }
                    }
                }
                
                ctx.set_temp(dest, alloca.into());
                Some(alloca.into())
            }

            MirInstrKind::FieldGet { dest, object, field } => {
                if let Some(obj_ptr) = operand_to_value(ctx, object) {
                    if obj_ptr.is_pointer_value() {
                        let idx = field.parse::<u32>().unwrap_or(0);
                        let ptr = obj_ptr.into_pointer_value();
                        
                        if let Some(struct_type) = ctx.lookup_struct_type("_default") {
                            if let Ok(field_ptr) = ctx.builder.build_struct_gep(struct_type, ptr, idx, "field_ptr") {
                                if let Ok(val) = ctx.builder.build_load(ctx.i64_type(), field_ptr, dest) {
                                    ctx.set_temp(dest, val);
                                    return Some(val);
                                }
                            }
                        }
                    }
                }
                None
            }

            MirInstrKind::FieldSet { object, field, value } => {
                if let (Some(obj_ptr), Some(val)) = (operand_to_value(ctx, object), operand_to_value(ctx, value)) {
                    if obj_ptr.is_pointer_value() {
                        let idx = field.parse::<u32>().unwrap_or(0);
                        let ptr = obj_ptr.into_pointer_value();
                        
                        if let Some(struct_type) = ctx.lookup_struct_type("_default") {
                            if let Ok(field_ptr) = ctx.builder.build_struct_gep(struct_type, ptr, idx, "field_ptr") {
                                ctx.builder.build_store(field_ptr, val).ok();
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
