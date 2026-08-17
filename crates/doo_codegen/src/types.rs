//! Type Translation — MIR types to LLVM types.
//!
//! Single source of truth for mapping Doo's type system to LLVM IR types.
//! All codegen passes use `mir_to_llvm` to get the LLVM representation of a type.

use crate::context::CodegenContext;
use doo_mir::types::MirType;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::AddressSpace;

/// Translate a MIR type to its LLVM IR type representation.
///
/// Returns `None` for `Void` and `Never` since they don't have a storable
/// `BasicTypeEnum` representation (void is only valid as a function return type).
pub fn mir_to_llvm<'ctx>(ty: &MirType, ctx: &CodegenContext<'ctx>) -> Option<BasicTypeEnum<'ctx>> {
    match ty {
        MirType::Int => Some(ctx.context.i64_type().into()),
        MirType::Float => Some(ctx.context.f64_type().into()),
        MirType::Bool => Some(ctx.context.bool_type().into()),
        MirType::Char => Some(ctx.context.i32_type().into()),
        MirType::Str => {
            // Fat string: { i8*, i64 }
            let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
            Some(
                ctx.context
                    .struct_type(&[ptr_ty.into(), ctx.context.i64_type().into()], false)
                    .into(),
            )
        }
        MirType::Void | MirType::Never => None,

        MirType::Ptr(inner) => {
            // Raw pointer — always i8* at LLVM level
            let _ = mir_to_llvm(inner, ctx);
            Some(ctx.context.ptr_type(AddressSpace::default()).into())
        }

        MirType::Array(elem_ty) => {
            // Array: { i64 len, i64 cap, ptr data }
            let elem_llvm = mir_to_llvm(elem_ty, ctx).unwrap_or(ctx.context.i64_type().into());
            let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
            Some(
                ctx.context
                    .struct_type(
                        &[
                            ctx.context.i64_type().into(),
                            ctx.context.i64_type().into(),
                            ptr_ty.into(),
                        ],
                        false,
                    )
                    .into(),
            )
        }

        MirType::Map { .. } => {
            // Opaque struct — all operations via FFI
            let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
            Some(
                ctx.context
                    .struct_type(
                        &[
                            ctx.context.i64_type().into(),
                            ctx.context.i64_type().into(),
                            ptr_ty.into(),
                        ],
                        false,
                    )
                    .into(),
            )
        }

        MirType::Optional(inner) => {
            // Optional: { i1 tag, [T payload] }
            let inner_llvm = mir_to_llvm(inner, ctx).unwrap_or(ctx.context.i64_type().into());
            let payload_ty = ctx.context.struct_type(&[inner_llvm.into()], false);
            Some(
                ctx.context
                    .struct_type(&[ctx.context.bool_type().into(), payload_ty.into()], false)
                    .into(),
            )
        }

        MirType::Result { ok, err } => {
            // Result: { i1 tag, union payload }
            let ok_llvm = mir_to_llvm(ok, ctx).unwrap_or(ctx.context.i64_type().into());
            let err_llvm = mir_to_llvm(err, ctx).unwrap_or(ctx.context.i64_type().into());
            let ok_size = ok_llvm
                .size_of()
                .unwrap_or(ctx.context.i64_type().const_int(8, false));
            let err_size = err_llvm
                .size_of()
                .unwrap_or(ctx.context.i64_type().const_int(8, false));

            // Use the larger type as the union payload
            let payload_ty = if ok_llvm == err_llvm {
                ok_llvm
            } else {
                ctx.context.i64_type().into()
            };

            Some(
                ctx.context
                    .struct_type(&[ctx.context.bool_type().into(), payload_ty.into()], false)
                    .into(),
            )
        }

        MirType::Tuple(elements) => {
            let field_types: Vec<BasicTypeEnum> = elements
                .iter()
                .filter_map(|t| mir_to_llvm(t, ctx))
                .collect();
            if field_types.is_empty() {
                Some(ctx.context.struct_type(&[], false).into())
            } else {
                Some(ctx.context.struct_type(&field_types, false).into())
            }
        }

        MirType::Struct { name, fields } => {
            let field_types: Vec<BasicTypeEnum> = fields
                .iter()
                .filter_map(|(_, t)| mir_to_llvm(t, ctx))
                .collect();
            Some(ctx.context.struct_type(&field_types, false).into())
        }

        MirType::Enum { variants, .. } => {
            // Enum: { i32 tag, union payload }
            let max_payload = variants
                .iter()
                .filter_map(|(_, payload_types)| {
                    if payload_types.is_empty() {
                        None
                    } else {
                        Some(
                            payload_types
                                .iter()
                                .filter_map(|t| mir_to_llvm(t, ctx))
                                .collect::<Vec<_>>(),
                        )
                    }
                })
                .map(|types| if types.is_empty() { 0 } else { types.len() })
                .max()
                .unwrap_or(0);

            let payload_ty = if max_payload > 0 {
                ctx.context.i64_type().into()
            } else {
                ctx.context.i64_type().into()
            };

            Some(
                ctx.context
                    .struct_type(&[ctx.context.i32_type().into(), payload_ty], false)
                    .into(),
            )
        }

        MirType::Function { params, ret } => {
            // Function pointer: ptr to function type
            let _ = mir_to_llvm(ret, ctx);
            for p in params {
                let _ = mir_to_llvm(p, ctx);
            }
            Some(ctx.context.ptr_type(AddressSpace::default()).into())
        }

        MirType::Closure { .. } => {
            // Closure: { fn_ptr, env_ptr }
            let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
            Some(
                ctx.context
                    .struct_type(&[ptr_ty.into(), ptr_ty.into()], false)
                    .into(),
            )
        }
    }
}
