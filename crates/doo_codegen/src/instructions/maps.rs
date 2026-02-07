//! Map Instruction Handler
//!
//! Handles: MapCreate, MapGet, MapSet, MapHas

use super::InstructionHandler;
use crate::context::CodegenContext;
use crate::layout::{
    alloc_with_header, data_ptr_from_header, get_map_data_ptr, get_map_length,
    header_ptr_from_data, load_len_i32, set_map_length, store_len_at_header,
};
use crate::utils::{default_for_type, emit_eq, operand_to_value};
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_mir::{MirInstr, MirInstrKind, MirOperand};
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

/// Map instruction handler.
pub struct MapHandler;

impl<'ctx> InstructionHandler<'ctx> for MapHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(
            instr.kind,
            MirInstrKind::MapCreate { .. }
                | MirInstrKind::MapGet { .. }
                | MirInstrKind::MapSet { .. }
                | MirInstrKind::MapHas { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::MapCreate {
                dest,
                entries,
                key_type,
                val_type,
            } => {
                let key_llvm = ctx.get_llvm_type(*key_type);
                let val_llvm = ctx.get_llvm_type(*val_type);
                let pair_ty = ctx
                    .context
                    .struct_type(&[key_llvm.into(), val_llvm.into()], false);

                let len_i32 = ctx.i32_type().const_int(entries.len() as u64, false);
                let data_ptr = alloc_with_header(ctx, len_i32, pair_ty, "map")?;

                let pair_ptr_ty = pair_ty.ptr_type(AddressSpace::default());
                let base = ctx
                    .builder
                    .build_pointer_cast(data_ptr, pair_ptr_ty, "map_data_cast")
                    .ok()?;

                for (i, (k, v)) in entries.iter().enumerate() {
                    let Some(kv) = operand_to_value(ctx, k) else {
                        continue;
                    };
                    let Some(vv) = operand_to_value(ctx, v) else {
                        continue;
                    };
                    let idx = ctx.i64_type().const_int(i as u64, false);
                    let pair_ptr =
                        unsafe { ctx.builder.build_gep(pair_ty, base, &[idx], "pair_ptr") }.ok()?;
                    let key_ptr = ctx
                        .builder
                        .build_struct_gep(pair_ty, pair_ptr, 0, "key_ptr")
                        .ok()?;
                    let val_ptr = ctx
                        .builder
                        .build_struct_gep(pair_ty, pair_ptr, 1, "val_ptr")
                        .ok()?;
                    ctx.builder.build_store(key_ptr, kv).ok();
                    ctx.builder.build_store(val_ptr, vv).ok();
                }

                ctx.set_temp(dest, data_ptr.into());
                Some(data_ptr.into())
            }

            MirInstrKind::MapGet {
                dest,
                map,
                key,
                key_type,
                val_type,
            } => {
                let mapv = operand_to_value(ctx, map)?;
                let keyv = operand_to_value(ctx, key)?;
                if !mapv.is_pointer_value() {
                    return None;
                }
                let map_ptr = mapv.into_pointer_value();

                let key_llvm = ctx.get_llvm_type(*key_type);
                let val_llvm = ctx.get_llvm_type(*val_type);
                let pair_ty = ctx
                    .context
                    .struct_type(&[key_llvm.into(), val_llvm.into()], false);
                let pair_ptr_ty = pair_ty.ptr_type(AddressSpace::default());
                let base = ctx
                    .builder
                    .build_pointer_cast(map_ptr, pair_ptr_ty, "map_data_cast")
                    .ok()?;

                let len_i32 = load_len_i32(ctx, map_ptr)?;
                let len_i64 = ctx
                    .builder
                    .build_int_z_extend(len_i32, ctx.i64_type(), "len_i64")
                    .ok()?;

                let default_val = default_for_type(ctx, val_llvm);
                let res_alloca = ctx.builder.build_alloca(val_llvm, "map_get_res").ok()?;
                ctx.builder.build_store(res_alloca, default_val).ok();

                let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
                let loop_bb = ctx.context.append_basic_block(current_fn, "map_get_loop");
                let check_bb = ctx.context.append_basic_block(current_fn, "map_get_check");
                let inc_bb = ctx.context.append_basic_block(current_fn, "map_get_inc");
                let found_bb = ctx.context.append_basic_block(current_fn, "map_get_found");
                let end_bb = ctx.context.append_basic_block(current_fn, "map_get_end");

                let idx_alloca = ctx.builder.build_alloca(ctx.i64_type(), "idx").ok()?;
                ctx.builder
                    .build_store(idx_alloca, ctx.i64_type().const_zero())
                    .ok();

                ctx.builder.build_unconditional_branch(loop_bb).ok()?;

                ctx.builder.position_at_end(loop_bb);
                let idx = ctx
                    .builder
                    .build_load(ctx.i64_type(), idx_alloca, "idx")
                    .ok()?
                    .into_int_value();
                let cond = ctx
                    .builder
                    .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
                    .ok()?;
                ctx.builder
                    .build_conditional_branch(cond, check_bb, end_bb)
                    .ok()?;

                ctx.builder.position_at_end(check_bb);
                let pair_ptr =
                    unsafe { ctx.builder.build_gep(pair_ty, base, &[idx], "pair_ptr") }.ok()?;
                let key_ptr = ctx
                    .builder
                    .build_struct_gep(pair_ty, pair_ptr, 0, "key_ptr")
                    .ok()?;
                let stored_key = ctx
                    .builder
                    .build_load(key_llvm, key_ptr, "stored_key")
                    .ok()?;
                let is_eq = emit_eq(ctx, *key_type, stored_key, keyv)?;
                ctx.builder
                    .build_conditional_branch(is_eq, found_bb, inc_bb)
                    .ok()?;

                ctx.builder.position_at_end(found_bb);
                let val_ptr = ctx
                    .builder
                    .build_struct_gep(pair_ty, pair_ptr, 1, "val_ptr")
                    .ok()?;
                let stored_val = ctx
                    .builder
                    .build_load(val_llvm, val_ptr, "stored_val")
                    .ok()?;
                ctx.builder.build_store(res_alloca, stored_val).ok();
                ctx.builder.build_unconditional_branch(end_bb).ok()?;

                ctx.builder.position_at_end(inc_bb);
                let next = ctx
                    .builder
                    .build_int_add(idx, ctx.i64_type().const_int(1, false), "next")
                    .ok()?;
                ctx.builder.build_store(idx_alloca, next).ok();
                ctx.builder.build_unconditional_branch(loop_bb).ok()?;

                ctx.builder.position_at_end(end_bb);
                let res = ctx.builder.build_load(val_llvm, res_alloca, dest).ok()?;
                ctx.set_temp(dest, res);
                // Set the type for the temp so Clone knows the correct value type
                ctx.set_variable_type(dest, *val_type);
                Some(res)
            }

            MirInstrKind::MapHas {
                dest,
                map,
                key,
                key_type,
                val_type,
            } => {
                let mapv = operand_to_value(ctx, map)?;
                let keyv = operand_to_value(ctx, key)?;
                if !mapv.is_pointer_value() {
                    return None;
                }
                let map_ptr = mapv.into_pointer_value();

                let key_llvm = ctx.get_llvm_type(*key_type);
                let val_llvm = ctx.get_llvm_type(*val_type);
                let pair_ty = ctx
                    .context
                    .struct_type(&[key_llvm.into(), val_llvm.into()], false);
                let pair_ptr_ty = pair_ty.ptr_type(AddressSpace::default());
                let base = ctx
                    .builder
                    .build_pointer_cast(map_ptr, pair_ptr_ty, "map_data_cast")
                    .ok()?;

                let len_i32 = load_len_i32(ctx, map_ptr)?;
                let len_i64 = ctx
                    .builder
                    .build_int_z_extend(len_i32, ctx.i64_type(), "len_i64")
                    .ok()?;

                let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
                let loop_bb = ctx.context.append_basic_block(current_fn, "map_has_loop");
                let check_bb = ctx.context.append_basic_block(current_fn, "map_has_check");
                let inc_bb = ctx.context.append_basic_block(current_fn, "map_has_inc");
                let found_bb = ctx.context.append_basic_block(current_fn, "map_has_found");
                let end_bb = ctx.context.append_basic_block(current_fn, "map_has_end");

                let idx_alloca = ctx.builder.build_alloca(ctx.i64_type(), "idx").ok()?;
                ctx.builder
                    .build_store(idx_alloca, ctx.i64_type().const_zero())
                    .ok();
                let res_alloca = ctx.builder.build_alloca(ctx.bool_type(), "res").ok()?;
                ctx.builder
                    .build_store(res_alloca, ctx.bool_type().const_zero())
                    .ok();

                ctx.builder.build_unconditional_branch(loop_bb).ok()?;

                ctx.builder.position_at_end(loop_bb);
                let idx = ctx
                    .builder
                    .build_load(ctx.i64_type(), idx_alloca, "idx")
                    .ok()?
                    .into_int_value();
                let cond = ctx
                    .builder
                    .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
                    .ok()?;
                ctx.builder
                    .build_conditional_branch(cond, check_bb, end_bb)
                    .ok()?;

                ctx.builder.position_at_end(check_bb);
                let pair_ptr =
                    unsafe { ctx.builder.build_gep(pair_ty, base, &[idx], "pair_ptr") }.ok()?;
                let key_ptr = ctx
                    .builder
                    .build_struct_gep(pair_ty, pair_ptr, 0, "key_ptr")
                    .ok()?;
                let stored_key = ctx
                    .builder
                    .build_load(key_llvm, key_ptr, "stored_key")
                    .ok()?;
                let is_eq = emit_eq(ctx, *key_type, stored_key, keyv)?;
                ctx.builder
                    .build_conditional_branch(is_eq, found_bb, inc_bb)
                    .ok()?;

                ctx.builder.position_at_end(found_bb);
                ctx.builder
                    .build_store(res_alloca, ctx.bool_type().const_int(1, false))
                    .ok();
                ctx.builder.build_unconditional_branch(end_bb).ok()?;

                ctx.builder.position_at_end(inc_bb);
                let next = ctx
                    .builder
                    .build_int_add(idx, ctx.i64_type().const_int(1, false), "next")
                    .ok()?;
                ctx.builder.build_store(idx_alloca, next).ok();
                ctx.builder.build_unconditional_branch(loop_bb).ok()?;

                ctx.builder.position_at_end(end_bb);
                let res = ctx
                    .builder
                    .build_load(ctx.bool_type(), res_alloca, dest)
                    .ok()?;
                ctx.set_temp(dest, res);
                Some(res)
            }

            MirInstrKind::MapSet {
                map,
                key,
                value,
                key_type,
                val_type,
            } => {
                let debug = std::env::var("DOO_DEBUG").is_ok();
                if debug {
                    doo_debug!("CODEGEN", "MapSet: map={:?}, key={:?}, value={:?}", map, key, value);
                }
                
                let mapv = operand_to_value(ctx, map)?;
                if debug {
                    doo_debug!("CODEGEN", "MapSet: mapv ok");
                }
                let keyv = operand_to_value(ctx, key)?;
                if debug {
                    doo_debug!("CODEGEN", "MapSet: keyv ok");
                }
                let valv = operand_to_value(ctx, value)?;
                if debug {
                    doo_debug!("CODEGEN", "MapSet: valv ok");
                }
                if !mapv.is_pointer_value() {
                    if debug {
                        doo_debug!("CODEGEN", "MapSet: ERROR - mapv is not pointer!");
                    }
                    return None;
                }

                let old_data = mapv.into_pointer_value();
                let key_llvm = ctx.get_llvm_type(*key_type);
                let val_llvm = ctx.get_llvm_type(*val_type);
                let pair_ty = ctx
                    .context
                    .struct_type(&[key_llvm.into(), val_llvm.into()], false);
                let pair_ptr_ty = pair_ty.ptr_type(AddressSpace::default());
                let old_base = ctx
                    .builder
                    .build_pointer_cast(old_data, pair_ptr_ty, "map_data_cast")
                    .ok()?;
                if debug {
                    doo_debug!("CODEGEN", "MapSet: old_base ok");
                }

                let len_i32 = load_len_i32(ctx, old_data)?;
                if debug {
                    doo_debug!("CODEGEN", "MapSet: len_i32 ok");
                }
                let len_i64 = ctx
                    .builder
                    .build_int_z_extend(len_i32, ctx.i64_type(), "len_i64")
                    .ok()?;

                let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
                let loop_bb = ctx.context.append_basic_block(current_fn, "map_set_loop");
                let check_bb = ctx.context.append_basic_block(current_fn, "map_set_check");
                let inc_bb = ctx.context.append_basic_block(current_fn, "map_set_inc");
                let found_bb = ctx.context.append_basic_block(current_fn, "map_set_found");
                let not_found_bb = ctx
                    .context
                    .append_basic_block(current_fn, "map_set_not_found");
                let end_bb = ctx.context.append_basic_block(current_fn, "map_set_end");

                let idx_alloca = ctx.builder.build_alloca(ctx.i64_type(), "idx").ok()?;
                ctx.builder
                    .build_store(idx_alloca, ctx.i64_type().const_zero())
                    .ok();

                // We'll store updated data pointer here (defaults to old)
                let updated_alloca = ctx.builder.build_alloca(ctx.ptr_type(), "updated").ok()?;
                ctx.builder.build_store(updated_alloca, old_data).ok();

                ctx.builder.build_unconditional_branch(loop_bb).ok()?;

                ctx.builder.position_at_end(loop_bb);
                let idx = ctx
                    .builder
                    .build_load(ctx.i64_type(), idx_alloca, "idx")
                    .ok()?
                    .into_int_value();
                let cond = ctx
                    .builder
                    .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
                    .ok()?;
                ctx.builder
                    .build_conditional_branch(cond, check_bb, not_found_bb)
                    .ok()?;

                ctx.builder.position_at_end(check_bb);
                let pair_ptr =
                    unsafe { ctx.builder.build_gep(pair_ty, old_base, &[idx], "pair_ptr") }.ok()?;
                let key_ptr = ctx
                    .builder
                    .build_struct_gep(pair_ty, pair_ptr, 0, "key_ptr")
                    .ok()?;
                let stored_key = ctx
                    .builder
                    .build_load(key_llvm, key_ptr, "stored_key")
                    .ok()?;
                let is_eq = emit_eq(ctx, *key_type, stored_key, keyv)?;
                ctx.builder
                    .build_conditional_branch(is_eq, found_bb, inc_bb)
                    .ok()?;

                ctx.builder.position_at_end(found_bb);
                let val_ptr = ctx
                    .builder
                    .build_struct_gep(pair_ty, pair_ptr, 1, "val_ptr")
                    .ok()?;
                ctx.builder.build_store(val_ptr, valv).ok();
                let _ = ctx.builder.build_unconditional_branch(end_bb);

                ctx.builder.position_at_end(inc_bb);
                let next = ctx
                    .builder
                    .build_int_add(idx, ctx.i64_type().const_int(1, false), "next")
                    .ok()?;
                ctx.builder.build_store(idx_alloca, next).ok();
                ctx.builder.build_unconditional_branch(loop_bb).ok()?;

                ctx.builder.position_at_end(not_found_bb);
                // Grow via realloc(header, new_total)
                let doo_realloc = ctx.get_function(ffi_names::REALLOC)?;
                let header_ptr = header_ptr_from_data(ctx, old_data)?;
                let pair_size = pair_ty.size_of()?;
                let new_len_i32 = ctx
                    .builder
                    .build_int_add(len_i32, ctx.i32_type().const_int(1, false), "new_len")
                    .ok()?;
                let new_len_i64 = ctx
                    .builder
                    .build_int_z_extend(new_len_i32, ctx.i64_type(), "new_len_i64")
                    .ok()?;
                let data_bytes = ctx
                    .builder
                    .build_int_mul(new_len_i64, pair_size, "data_bytes")
                    .ok()?;
                // Header is 16 bytes: 2 x i64 (length + capacity)
                let total = ctx
                    .builder
                    .build_int_add(ctx.i64_type().const_int(16, false), data_bytes, "total")
                    .ok()?;
                let new_header = ctx
                    .builder
                    .build_call(doo_realloc, &[header_ptr.into(), total.into()], "realloc")
                    .ok()?
                    .try_as_basic_value()
                    .basic()?
                    .into_pointer_value();
                // Store length as i64 (header expects i64)
                let new_len_i64_for_store = ctx
                    .builder
                    .build_int_z_extend(new_len_i32, ctx.i64_type(), "new_len_i64_store")
                    .ok()?;
                store_len_at_header(ctx, new_header, new_len_i64_for_store)?;
                let new_data = data_ptr_from_header(ctx, new_header)?;
                ctx.builder.build_store(updated_alloca, new_data).ok();

                let new_base = ctx
                    .builder
                    .build_pointer_cast(new_data, pair_ptr_ty, "map_new_cast")
                    .ok()?;
                let append_idx = ctx
                    .builder
                    .build_int_z_extend(len_i32, ctx.i64_type(), "append_idx")
                    .ok()?;
                let new_pair_ptr = unsafe {
                    ctx.builder
                        .build_gep(pair_ty, new_base, &[append_idx], "new_pair")
                }
                .ok()?;
                let nk = ctx
                    .builder
                    .build_struct_gep(pair_ty, new_pair_ptr, 0, "new_key_ptr")
                    .ok()?;
                let nv = ctx
                    .builder
                    .build_struct_gep(pair_ty, new_pair_ptr, 1, "new_val_ptr")
                    .ok()?;
                ctx.builder.build_store(nk, keyv).ok();
                ctx.builder.build_store(nv, valv).ok();
                let _ = ctx.builder.build_unconditional_branch(end_bb);

                ctx.builder.position_at_end(end_bb);
                // update temp binding if we reallocated
                if let MirOperand::Local(name) | MirOperand::Temp(name) = map {
                    if let Ok(updated) = ctx
                        .builder
                        .build_load(ctx.ptr_type(), updated_alloca, "updated")
                    {
                        if let Some(local_ptr) = ctx.get_local(name) {
                            ctx.builder.build_store(local_ptr, updated).ok();
                        } else {
                            ctx.set_temp(name, updated);
                        }
                    }
                }
                None
            }

            _ => None,
        }
    }
}
// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use doo_mir::MirInstr;

    #[test]
    fn test_map_handler_handles_map_create() {
        use doo_core::types::builtin;
        let handler = MapHandler;
        let instr = MirInstr::new(MirInstrKind::MapCreate {
            dest: "m".to_string(),
            entries: vec![],
            key_type: builtin::STR,
            val_type: builtin::INT,
        });
        assert!(handler.handles(&instr));
    }

    #[test]
    fn test_map_handler_handles_map_get() {
        use doo_core::types::builtin;
        let handler = MapHandler;
        let instr = MirInstr::new(MirInstrKind::MapGet {
            dest: "v".to_string(),
            map: MirOperand::Local("m".to_string()),
            key: MirOperand::Local("k".to_string()),
            key_type: builtin::STR,
            val_type: builtin::INT,
        });
        assert!(handler.handles(&instr));
    }

    #[test]
    fn test_map_handler_handles_map_set() {
        use doo_core::types::builtin;
        let handler = MapHandler;
        let instr = MirInstr::new(MirInstrKind::MapSet {
            map: MirOperand::Local("m".to_string()),
            key: MirOperand::Local("k".to_string()),
            value: MirOperand::Local("v".to_string()),
            key_type: builtin::STR,
            val_type: builtin::INT,
        });
        assert!(handler.handles(&instr));
    }

    #[test]
    fn test_map_handler_handles_map_has() {
        use doo_core::types::builtin;
        let handler = MapHandler;
        let instr = MirInstr::new(MirInstrKind::MapHas {
            dest: "has".to_string(),
            map: MirOperand::Local("m".to_string()),
            key: MirOperand::Local("k".to_string()),
            key_type: builtin::STR,
            val_type: builtin::INT,
        });
        assert!(handler.handles(&instr));
    }

    #[test]
    fn test_map_handler_does_not_handle_other() {
        let handler = MapHandler;
        let instr = MirInstr::new(MirInstrKind::Assign {
            dest: "x".to_string(),
            value: MirOperand::Local("y".to_string()),
        });
        assert!(!handler.handles(&instr));
    }

    #[test]
    fn test_map_handler_does_not_handle_array() {
        use doo_core::types::builtin;
        let handler = MapHandler;
        let instr = MirInstr::new(MirInstrKind::ArrayCreate {
            dest: "arr".to_string(),
            elements: vec![],
            elem_type: builtin::INT,
        });
        assert!(!handler.handles(&instr));
    }
}
