use doo_hir::{HirMatchPattern};
use doo_mir::{MirInstrKind, MirOperand, MirConst, BinaryOp};
use super::MirBuilder;
use doo_core::Span;
use doo_core::types::builtin;
use crate::types::LocalDef;

pub fn build_match_condition(
    builder: &mut MirBuilder,
    scrutinees: &[MirOperand],
    pattern: &HirMatchPattern,
    span: Span,
) -> MirOperand {
    if scrutinees.is_empty() {
        match pattern {
            HirMatchPattern::Wildcard => MirOperand::Const(MirConst::Bool(true)),
            HirMatchPattern::Condition(e) | HirMatchPattern::Literal(e) => builder.build_expr(e),
            _ => MirOperand::Const(MirConst::Bool(false)),
        }
    } else {
        build_value_pattern_condition(builder, scrutinees, pattern, span)
    }
}

pub fn build_value_pattern_condition(
    builder: &mut MirBuilder,
    scrutinees: &[MirOperand],
    pattern: &HirMatchPattern,
    span: Span,
) -> MirOperand {
    match pattern {
        HirMatchPattern::Wildcard => MirOperand::Const(MirConst::Bool(true)),
        
        HirMatchPattern::Literal(e) => {
            let lit = builder.build_expr(e);
            let dest = builder.new_temp();
            builder.emit(
                MirInstrKind::BinaryOp {
                    dest: dest.clone(),
                    op: BinaryOp::Eq,
                    lhs: scrutinees[0].clone(),
                    rhs: lit,
                },
                span,
            );
            MirOperand::Temp(dest)
        }
        
        HirMatchPattern::EnumVariant { enum_name, variant } => {
            // Get the discriminant (tag) of the scrutinee and compare with variant index
            if scrutinees.is_empty() {
                return MirOperand::Const(MirConst::Bool(false));
            }
            
            let tag_dest = builder.new_temp();
            builder.emit(
                MirInstrKind::EnumGetTag {
                    dest: tag_dest.clone(),
                    value: scrutinees[0].clone(),
                    enum_name: enum_name.clone(),
                },
                span,
            );
            
            // Compare tag with the expected variant name
            let cmp_dest = builder.new_temp();
            builder.emit(
                MirInstrKind::EnumTagEquals {
                    dest: cmp_dest.clone(),
                    tag: MirOperand::Temp(tag_dest),
                    variant_name: variant.clone(),
                    enum_name: enum_name.clone(),
                },
                span,
            );
            
            MirOperand::Temp(cmp_dest)
        }
        
        HirMatchPattern::EnumVariantPayload { enum_name, variant, bindings } => {
            // First check the discriminant
            if scrutinees.is_empty() {
                return MirOperand::Const(MirConst::Bool(false));
            }
            
            let tag_dest = builder.new_temp();
            builder.emit(
                MirInstrKind::EnumGetTag {
                    dest: tag_dest.clone(),
                    value: scrutinees[0].clone(),
                    enum_name: enum_name.clone(),
                },
                span,
            );
            
            // Compare tag
            let cmp_dest = builder.new_temp();
            builder.emit(
                MirInstrKind::EnumTagEquals {
                    dest: cmp_dest.clone(),
                    tag: MirOperand::Temp(tag_dest),
                    variant_name: variant.clone(),
                    enum_name: enum_name.clone(),
                },
                span,
            );
            
            // Extract and bind payload values
            for (i, binding) in bindings.iter().enumerate() {
                if binding != "_" {
                    let payload_dest = binding.clone();
                    builder.emit(
                        MirInstrKind::EnumGetPayload {
                            dest: payload_dest.clone(),
                            value: scrutinees[0].clone(),
                            variant_name: variant.clone(),
                            enum_name: enum_name.clone(),
                            index: i as u32,
                        },
                        span,
                    );
                    
                    // Register as local
                    if let Some(f) = &mut builder.current_func {
                        f.locals.push(LocalDef {
                            name: payload_dest,
                            type_id: builtin::ANY,
                            mutable: false,
                        });
                    }
                }
            }
            
            MirOperand::Temp(cmp_dest)
        }
        
        HirMatchPattern::Tuple(parts) if parts.len() == scrutinees.len() => {
            let mut current: Option<MirOperand> = None;
            for (s, p) in scrutinees.iter().cloned().zip(parts.iter()) {
                let eq = match p {
                    HirMatchPattern::Wildcard => MirOperand::Const(MirConst::Bool(true)),
                    HirMatchPattern::Literal(e) => {
                        let lit = builder.build_expr(e);
                        let dest = builder.new_temp();
                        builder.emit(
                            MirInstrKind::BinaryOp {
                                dest: dest.clone(),
                                op: BinaryOp::Eq,
                                lhs: s,
                                rhs: lit,
                            },
                            span,
                        );
                        MirOperand::Temp(dest)
                    }
                    _ => MirOperand::Const(MirConst::Bool(false)),
                };

                current = Some(if let Some(prev) = current {
                    let dest = builder.new_temp();
                    builder.emit(
                        MirInstrKind::BinaryOp {
                            dest: dest.clone(),
                            op: BinaryOp::And,
                            lhs: prev,
                            rhs: eq,
                        },
                        span,
                    );
                    MirOperand::Temp(dest)
                } else {
                    eq
                });
            }

            current.unwrap_or_else(|| MirOperand::Const(MirConst::Bool(true)))
        }
        
        HirMatchPattern::Condition(e) => builder.build_expr(e),
        
        _ => MirOperand::Const(MirConst::Bool(false)),
    }
}
