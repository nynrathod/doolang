//! Composite/Aggregate Instruction Handler
//!
//! Handles: TupleCreate, TupleGet, StructCreate, FieldGet, FieldSet

use super::InstructionHandler;
use crate::context::CodegenContext;
use crate::utils::operand_to_value;
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
                let alloca = ctx.builder.build_alloca(tuple_type, dest).ok()?;

                for (i, elem) in elements.iter().enumerate() {
                    if let Some(val) = operand_to_value(ctx, elem) {
                        if let Ok(ptr) =
                            ctx.builder
                                .build_struct_gep(tuple_type, alloca, i as u32, "field_ptr")
                        {
                            ctx.builder.build_store(ptr, val).ok();
                        }
                    }
                }

                ctx.set_temp(dest, alloca.into());
                Some(alloca.into())
            }

            MirInstrKind::TupleGet { dest, tuple, index, tuple_type } => {
                if let Some(tup) = operand_to_value(ctx, tuple) {
                    let tup: inkwell::values::BasicValueEnum = tup;
                    if tup.is_pointer_value() {
                        let ptr = tup.into_pointer_value();
                        
                        // Build the tuple type from registry if we have the type info
                        // First, extract element TypeIds to avoid borrowing issues
                        let elem_type_ids: Option<Vec<doo_core::types::TypeId>> = if let Some(type_id) = tuple_type {
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
                            let element_types: Vec<inkwell::types::BasicTypeEnum> = elem_ids
                                .iter()
                                .map(|tid| ctx.get_llvm_type(*tid))
                                .collect();
                            let tuple_struct = ctx.context.struct_type(&element_types, false);
                            let elem_llvm = ctx.get_llvm_type(elem_ids[*index]);
                            (tuple_struct, elem_llvm)
                        } else {
                            // Fallback: single i64
                            (ctx.context.struct_type(&[ctx.i64_type().into()], false), ctx.i64_type().into())
                        };

                        if let Ok(field_ptr) =
                            ctx.builder
                                .build_struct_gep(tuple_ty, ptr, *index as u32, "field")
                        {
                            if let Ok(val) = ctx.builder.build_load(elem_type, field_ptr, dest)
                            {
                                ctx.set_temp(dest, val);
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
                // Collect field names in order for metadata
                let field_names: Vec<String> =
                    fields.iter().map(|(name, _)| name.clone()).collect();

                // Register struct metadata for field lookups
                ctx.register_struct_metadata(struct_name, field_names);

                // Build LLVM struct type with correct field types from type registry
                // Use get_llvm_type for consistent type mapping across all code paths
                let field_types: Vec<inkwell::types::BasicTypeEnum> = 
                    if let Some(field_type_ids) = ctx.get_struct_field_types(struct_name) {
                        field_type_ids.iter().map(|type_id| {
                            ctx.get_llvm_type(*type_id)
                        }).collect()
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
                        ctx.module.add_function(doo_core::constants::ffi_names::MALLOC, fn_ty, None)
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
                    .left()?
                    .into_pointer_value();

                // Get field type IDs for proper boxing
                let field_type_ids = ctx.get_struct_field_types(struct_name);

                // Store field values
                for (i, (_, value)) in fields.iter().enumerate() {
                    if let Some(val) = operand_to_value(ctx, value) {
                        if let Ok(ptr) =
                            ctx.builder
                                .build_struct_gep(struct_type, struct_ptr, i as u32, "field_ptr")
                        {
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
                if let Some(obj_ptr) = operand_to_value(ctx, object) {
                    if obj_ptr.is_pointer_value() {
                        let ptr = obj_ptr.into_pointer_value();

                        // Get struct name from the object operand
                        let var_name = Self::get_operand_name(object);
                        let struct_name = var_name
                            .and_then(|name| ctx.get_temp_struct_type(name).cloned());

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
                                    // Determine the correct LLVM type based on field's Doo type
                                    let field_type_id = ctx.get_struct_field_type(&struct_name, field);
                                    let load_type: inkwell::types::BasicTypeEnum = match field_type_id {
                                        Some(t) if t == builtin::STR => ctx.ptr_type().into(),
                                        Some(t) if t == builtin::FLOAT => ctx.f64_type().into(),
                                        Some(t) if t == builtin::BOOL => ctx.bool_type().into(),
                                        _ => ctx.i64_type().into(), // Int and default
                                    };
                                    
                                    if let Ok(val) =
                                        ctx.builder.build_load(load_type, field_ptr, dest)
                                    {
                                        ctx.set_temp(dest, val);
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
