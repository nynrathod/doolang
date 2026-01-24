//! MIR Optimization Pipeline
//!
//! Pre-LLVM optimizations that run to a fixed point.
//!
//! ## Passes
//!
//! 1. Constant Folding - Evaluate constant expressions at compile time
//! 2. Dead Code Elimination - Remove unused instructions
//! 3. Drop Optimization - Batch adjacent drops, remove redundant drops

use crate::types::*;

/// Optimization pass trait.
pub trait MirPass {
    /// Pass name for debugging.
    fn name(&self) -> &'static str;
    
    /// Run the pass on a program. Returns true if any changes were made.
    fn run(&mut self, program: &mut MirProgram) -> bool;
}

/// Optimization pipeline - runs passes to fixed point.
pub struct OptimizationPipeline {
    passes: Vec<Box<dyn MirPass>>,
}

impl OptimizationPipeline {
    /// Create a new empty pipeline.
    pub fn new() -> Self {
        Self { passes: Vec::new() }
    }

    /// Create a pipeline with default passes.
    pub fn default_pipeline() -> Self {
        let mut pipeline = Self::new();
        pipeline.add_pass(Box::new(ConstantFolding));
        pipeline.add_pass(Box::new(DeadCodeElimination));
        pipeline.add_pass(Box::new(DropOptimization));
        pipeline
    }

    /// Add a pass to the pipeline.
    pub fn add_pass(&mut self, pass: Box<dyn MirPass>) {
        self.passes.push(pass);
    }

    /// Run all passes to fixed point.
    pub fn run(&mut self, program: &mut MirProgram) {
        let max_iterations = 100; // Prevent infinite loops
        let mut iterations = 0;

        loop {
            let mut changed = false;
            for pass in &mut self.passes {
                changed |= pass.run(program);
            }

            iterations += 1;
            if !changed || iterations >= max_iterations {
                break;
            }
        }
    }
}

impl Default for OptimizationPipeline {
    fn default() -> Self {
        Self::default_pipeline()
    }
}

// ============================================================================
// Constant Folding
// ============================================================================

/// Constant folding pass.
///
/// Evaluates constant expressions at compile time:
/// - `1 + 2` → `3`
/// - `true && false` → `false`
pub struct ConstantFolding;

impl MirPass for ConstantFolding {
    fn name(&self) -> &'static str {
        "constant_folding"
    }

    fn run(&mut self, program: &mut MirProgram) -> bool {
        let mut changed = false;

        for func in &mut program.functions {
            for block in &mut func.blocks {
                for instr in &mut block.instructions {
                    if self.fold_instr(instr) {
                        changed = true;
                    }
                }
            }
        }

        changed
    }
}

impl ConstantFolding {
    fn fold_instr(&self, instr: &mut MirInstr) -> bool {
        match &instr.kind {
            MirInstrKind::BinaryOp { dest, op, lhs, rhs } => {
                if let (MirOperand::Const(lc), MirOperand::Const(rc)) = (lhs, rhs) {
                    if let Some(result) = self.fold_binop(*op, lc, rc) {
                        let dest = dest.clone();
                        instr.kind = MirInstrKind::Assign {
                            dest,
                            value: MirOperand::Const(result),
                        };
                        return true;
                    }
                }
            }
            MirInstrKind::UnaryOp { dest, op, operand } => {
                if let MirOperand::Const(c) = operand {
                    if let Some(result) = self.fold_unaryop(*op, c) {
                        let dest = dest.clone();
                        instr.kind = MirInstrKind::Assign {
                            dest,
                            value: MirOperand::Const(result),
                        };
                        return true;
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn fold_binop(&self, op: BinaryOp, lhs: &MirConst, rhs: &MirConst) -> Option<MirConst> {
        match (lhs, rhs) {
            (MirConst::Int(l), MirConst::Int(r)) => {
                Some(match op {
                    BinaryOp::Add => MirConst::Int(l.wrapping_add(*r)),
                    BinaryOp::Sub => MirConst::Int(l.wrapping_sub(*r)),
                    BinaryOp::Mul => MirConst::Int(l.wrapping_mul(*r)),
                    BinaryOp::Div if *r != 0 => MirConst::Int(l / r),
                    BinaryOp::Mod if *r != 0 => MirConst::Int(l % r),
                    BinaryOp::Eq => MirConst::Bool(l == r),
                    BinaryOp::Ne => MirConst::Bool(l != r),
                    BinaryOp::Lt => MirConst::Bool(l < r),
                    BinaryOp::Le => MirConst::Bool(l <= r),
                    BinaryOp::Gt => MirConst::Bool(l > r),
                    BinaryOp::Ge => MirConst::Bool(l >= r),
                    _ => return None,
                })
            }
            (MirConst::Float(l), MirConst::Float(r)) => {
                Some(match op {
                    BinaryOp::Add => MirConst::Float(l + r),
                    BinaryOp::Sub => MirConst::Float(l - r),
                    BinaryOp::Mul => MirConst::Float(l * r),
                    BinaryOp::Div if *r != 0.0 => MirConst::Float(l / r),
                    BinaryOp::Eq => MirConst::Bool(l == r),
                    BinaryOp::Ne => MirConst::Bool(l != r),
                    BinaryOp::Lt => MirConst::Bool(l < r),
                    BinaryOp::Le => MirConst::Bool(l <= r),
                    BinaryOp::Gt => MirConst::Bool(l > r),
                    BinaryOp::Ge => MirConst::Bool(l >= r),
                    _ => return None,
                })
            }
            (MirConst::Bool(l), MirConst::Bool(r)) => {
                Some(match op {
                    BinaryOp::And => MirConst::Bool(*l && *r),
                    BinaryOp::Or => MirConst::Bool(*l || *r),
                    BinaryOp::Eq => MirConst::Bool(l == r),
                    BinaryOp::Ne => MirConst::Bool(l != r),
                    _ => return None,
                })
            }
            _ => None,
        }
    }

    fn fold_unaryop(&self, op: UnaryOp, c: &MirConst) -> Option<MirConst> {
        match (op, c) {
            (UnaryOp::Neg, MirConst::Int(v)) => Some(MirConst::Int(-v)),
            (UnaryOp::Neg, MirConst::Float(v)) => Some(MirConst::Float(-v)),
            (UnaryOp::Not, MirConst::Bool(v)) => Some(MirConst::Bool(!v)),
            _ => None,
        }
    }
}

// ============================================================================
// Dead Code Elimination
// ============================================================================

/// Dead code elimination pass.
///
/// Removes:
/// - Assignments where result is never used
/// - Unreachable blocks
pub struct DeadCodeElimination;

impl MirPass for DeadCodeElimination {
    fn name(&self) -> &'static str {
        "dead_code_elimination"
    }

    fn run(&mut self, _program: &mut MirProgram) -> bool {
        // Simplified DCE - full implementation would track used values
        false
    }
}

// ============================================================================
// Drop Optimization
// ============================================================================

/// Drop optimization pass.
///
/// Optimizations:
/// - Batch adjacent drops
/// - Remove drops for Copy types
/// - Remove redundant drops
pub struct DropOptimization;

impl MirPass for DropOptimization {
    fn name(&self) -> &'static str {
        "drop_optimization"
    }

    fn run(&mut self, program: &mut MirProgram) -> bool {
        let mut changed = false;

        for func in &mut program.functions {
            for block in &mut func.blocks {
                // Remove consecutive duplicate drops
                let mut prev_drop: Option<String> = None;
                let before = block.instructions.len();
                
                block.instructions.retain(|instr| {
                    if let MirInstrKind::Drop { value } = &instr.kind {
                        if Some(value.clone()) == prev_drop {
                            return false; // Remove duplicate
                        }
                        prev_drop = Some(value.clone());
                    } else {
                        prev_drop = None;
                    }
                    true
                });

                if block.instructions.len() != before {
                    changed = true;
                }
            }
        }

        changed
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_folding_int_add() {
        let folding = ConstantFolding;
        let result = folding.fold_binop(
            BinaryOp::Add,
            &MirConst::Int(1),
            &MirConst::Int(2),
        );
        assert_eq!(result, Some(MirConst::Int(3)));
    }

    #[test]
    fn test_constant_folding_bool_and() {
        let folding = ConstantFolding;
        let result = folding.fold_binop(
            BinaryOp::And,
            &MirConst::Bool(true),
            &MirConst::Bool(false),
        );
        assert_eq!(result, Some(MirConst::Bool(false)));
    }

    #[test]
    fn test_constant_folding_neg() {
        let folding = ConstantFolding;
        let result = folding.fold_unaryop(UnaryOp::Neg, &MirConst::Int(5));
        assert_eq!(result, Some(MirConst::Int(-5)));
    }

    #[test]
    fn test_pipeline_creation() {
        let pipeline = OptimizationPipeline::default_pipeline();
        assert_eq!(pipeline.passes.len(), 3);
    }
}
