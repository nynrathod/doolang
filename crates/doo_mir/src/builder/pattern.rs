use doo_hir::{HirMatchPattern};
use crate::{MirInstrKind, MirOperand, MirConst, BinaryOp};
use super::MirBuilder;
use crate::types::Span;
use crate::sym::sym;

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
                    dest,
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
                    dest: tag_dest,
                    value: scrutinees[0].clone(),
                    enum_name: sym(enum_name),
                },
                span,
            );
            
            // Compare tag with the expected variant name
            let cmp_dest = builder.new_temp();
            builder.emit(
                MirInstrKind::EnumTagEquals {
                    dest: cmp_dest,
                    tag: MirOperand::Temp(tag_dest),
                    variant_name: sym(variant),
                    enum_name: sym(enum_name),
                },
                span,
            );
            
            MirOperand::Temp(cmp_dest)
        }
        
        HirMatchPattern::EnumVariantPayload { enum_name, variant, bindings: _ } => {
            // Only check the discriminant - payload extraction happens in the arm body
            // This avoids SSA domination issues where payload values defined in check blocks
            // are invalid in arm blocks or merge blocks
            if scrutinees.is_empty() {
                return MirOperand::Const(MirConst::Bool(false));
            }
            
            let tag_dest = builder.new_temp();
            builder.emit(
                MirInstrKind::EnumGetTag {
                    dest: tag_dest,
                    value: scrutinees[0].clone(),
                    enum_name: sym(enum_name),
                },
                span,
            );
            
            // Compare tag
            let cmp_dest = builder.new_temp();
            builder.emit(
                MirInstrKind::EnumTagEquals {
                    dest: cmp_dest,
                    tag: MirOperand::Temp(tag_dest),
                    variant_name: sym(variant),
                    enum_name: sym(enum_name),
                },
                span,
            );
            
            // NOTE: Payload extraction moved to arm body (see expr.rs build Match)
            // to ensure SSA values are defined in the correct basic block
            
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
                                dest,
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
                            dest,
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
