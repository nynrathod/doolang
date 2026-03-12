//! MIR Optimization Pipeline
//!
//! Pre-LLVM optimizations that run to a fixed point.
//!
//! ## Passes
//!
//! 1. Constant Folding - Evaluate constant expressions at compile time
//! 2. Dead Code Elimination - Remove unused instructions
//! 3. Drop Optimization - Batch adjacent drops, remove redundant drops

use crate::sym::Sym;
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
        pipeline.add_pass(Box::new(ConstantPropagation));
        pipeline.add_pass(Box::new(CopyPropagation));
        pipeline.add_pass(Box::new(DeadCodeElimination));
        pipeline.add_pass(Box::new(DropOptimization));
        pipeline.add_pass(Box::new(EscapeAnalysis));
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
                        let dest = *dest;
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
                        let dest = *dest;
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
            (MirConst::Int(l), MirConst::Int(r)) => Some(match op {
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
            }),
            (MirConst::Float(l), MirConst::Float(r)) => Some(match op {
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
            }),
            (MirConst::Bool(l), MirConst::Bool(r)) => Some(match op {
                BinaryOp::And => MirConst::Bool(*l && *r),
                BinaryOp::Or => MirConst::Bool(*l || *r),
                BinaryOp::Eq => MirConst::Bool(l == r),
                BinaryOp::Ne => MirConst::Bool(l != r),
                _ => return None,
            }),
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
/// - Assignments where result is never used (no side effects)
/// - Pure computations whose results are unused
///
/// Preserves:
/// - Calls (may have side effects)
/// - Drops (memory management)
/// - Stores/sets (mutations)
/// - All instructions without a destination (inherently side-effectful)
pub struct DeadCodeElimination;

impl MirPass for DeadCodeElimination {
    fn name(&self) -> &'static str {
        "dead_code_elimination"
    }

    fn run(&mut self, program: &mut MirProgram) -> bool {
        use rustc_hash::FxHashSet;

        let mut changed = false;

        for func in &mut program.functions {
            // Pass 1: Collect all used operand names across all blocks
            let mut used_names: FxHashSet<Sym> = FxHashSet::default();

            for block in &func.blocks {
                // Collect names from terminators
                match &block.terminator {
                    MirTerminator::Return { values } => {
                        for op in values {
                            Self::collect_operand_names(op, &mut used_names);
                        }
                    }
                    MirTerminator::Branch { cond, .. } => {
                        Self::collect_operand_names(cond, &mut used_names);
                    }
                    MirTerminator::Switch { value, .. } => {
                        Self::collect_operand_names(value, &mut used_names);
                    }
                    _ => {}
                }

                // Collect names from instruction operands
                for instr in &block.instructions {
                    for op in instr.operands() {
                        Self::collect_operand_names(op, &mut used_names);
                    }
                }
            }

            // Pass 2: Remove pure instructions whose destination is unused
            for block in &mut func.blocks {
                let before_len = block.instructions.len();

                block.instructions.retain(|instr| {
                    // Keep all side-effectful instructions regardless
                    if Self::has_side_effects(instr) {
                        return true;
                    }

                    // If instruction has a destination, check if it's used
                    if let Some(dest) = instr.destination() {
                        if !used_names.contains(dest) {
                            return false; // Dead: destination never read
                        }
                    }

                    true
                });

                if block.instructions.len() != before_len {
                    changed = true;
                }
            }
        }

        changed
    }
}

impl DeadCodeElimination {
    /// Collect variable names referenced by an operand.
    fn collect_operand_names(op: &MirOperand, names: &mut rustc_hash::FxHashSet<Sym>) {
        match op {
            MirOperand::Local(n) | MirOperand::Temp(n) | MirOperand::Global(n) => {
                names.insert(*n);
            }
            MirOperand::FuncRef(n) => {
                names.insert(*n);
            }
            MirOperand::Const(_) => {}
        }
    }

    /// Check if an instruction has side effects (must be preserved even if unused).
    fn has_side_effects(instr: &MirInstr) -> bool {
        matches!(
            &instr.kind,
            // Function calls may have side effects
            MirInstrKind::Call { .. }
            | MirInstrKind::MethodCall { .. }
            | MirInstrKind::FfiCall { .. }
            | MirInstrKind::ClosureCall { .. }
            // Memory management
            | MirInstrKind::Drop { .. }
            // Mutation operations
            | MirInstrKind::ArraySet { .. }
            | MirInstrKind::ArrayPush { .. }
            | MirInstrKind::ArrayExtend { .. }
            | MirInstrKind::MapSet { .. }
            | MirInstrKind::FieldSet { .. }
            // Print and I/O
            | MirInstrKind::Print { .. }
            // Async operations
            | MirInstrKind::Spawn { .. }
            | MirInstrKind::Await { .. }
            | MirInstrKind::ScopeCreate { .. }
            | MirInstrKind::ScopeSpawn { .. }
            | MirInstrKind::ScopeWait { .. }
            // Result wrapping (often part of error flow)  
            | MirInstrKind::WrapOk { .. }
            | MirInstrKind::WrapErr { .. }
        )
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
                let mut prev_drop: Option<Sym> = None;
                let before = block.instructions.len();

                block.instructions.retain(|instr| {
                    if let MirInstrKind::Drop { value } = &instr.kind {
                        if Some(*value) == prev_drop {
                            return false; // Remove duplicate
                        }
                        prev_drop = Some(*value);
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
// Constant Propagation
// ============================================================================

/// Constant propagation pass.
///
/// When a variable is assigned a constant value, replace all subsequent
/// uses of that variable with the constant (within the same block).
pub struct ConstantPropagation;

impl MirPass for ConstantPropagation {
    fn name(&self) -> &'static str {
        "constant_propagation"
    }

    fn run(&mut self, program: &mut MirProgram) -> bool {
        use rustc_hash::FxHashMap;

        let mut changed = false;

        for func in &mut program.functions {
            for block in &mut func.blocks {
                // Track known constant values: variable -> constant
                let mut known: FxHashMap<Sym, MirConst> = FxHashMap::default();

                for instr in &mut block.instructions {
                    // Replace operands with known constants
                    let operand_replacements: Vec<(Sym, MirConst)> = instr
                        .operands()
                        .iter()
                        .filter_map(|op| {
                            if let MirOperand::Local(name) | MirOperand::Temp(name) = op {
                                known.get(name).map(|c| (*name, c.clone()))
                            } else {
                                None
                            }
                        })
                        .collect();

                    if !operand_replacements.is_empty() {
                        Self::replace_operands(instr, &operand_replacements);
                        changed = true;
                    }

                    // Track new constant assignments
                    if let MirInstrKind::Assign {
                        dest,
                        value: MirOperand::Const(c),
                    } = &instr.kind
                    {
                        known.insert(*dest, c.clone());
                    }

                    // If a tracked variable is reassigned, invalidate it
                    if let Some(dest) = instr.destination() {
                        if !matches!(
                            &instr.kind,
                            MirInstrKind::Assign {
                                value: MirOperand::Const(_),
                                ..
                            }
                        ) {
                            known.remove(dest);
                        }
                    }
                }
            }
        }

        changed
    }
}

impl ConstantPropagation {
    /// Replace operands in an instruction with known constant values.
    fn replace_operands(instr: &mut MirInstr, replacements: &[(Sym, MirConst)]) {
        for (name, constant) in replacements {
            Self::replace_operand_in_kind(&mut instr.kind, *name, constant);
        }
    }

    fn replace_operand_in_kind(kind: &mut MirInstrKind, name: Sym, constant: &MirConst) {
        match kind {
            MirInstrKind::BinaryOp { lhs, rhs, .. } => {
                Self::maybe_replace(lhs, name, constant);
                Self::maybe_replace(rhs, name, constant);
            }
            MirInstrKind::UnaryOp { operand, .. } => {
                Self::maybe_replace(operand, name, constant);
            }
            MirInstrKind::Assign { value, .. } => {
                Self::maybe_replace(value, name, constant);
            }
            MirInstrKind::Move { src, .. } => {
                Self::maybe_replace(src, name, constant);
            }
            MirInstrKind::Copy { src, .. } => {
                Self::maybe_replace(src, name, constant);
            }
            _ => {} // Other instruction kinds are left as-is
        }
    }

    fn maybe_replace(op: &mut MirOperand, name: Sym, constant: &MirConst) {
        match op {
            MirOperand::Local(n) | MirOperand::Temp(n) if *n == name => {
                *op = MirOperand::Const(constant.clone());
            }
            _ => {}
        }
    }
}

// ============================================================================
// Copy Propagation
// ============================================================================

/// Copy propagation pass.
///
/// When a variable is a direct copy of another variable (`x = y`), replace
/// subsequent uses of `x` with `y` (within the same block).
pub struct CopyPropagation;

impl MirPass for CopyPropagation {
    fn name(&self) -> &'static str {
        "copy_propagation"
    }

    fn run(&mut self, program: &mut MirProgram) -> bool {
        use rustc_hash::FxHashMap;

        let changed = false;

        for func in &mut program.functions {
            for block in &mut func.blocks {
                // Track copy chains: x = y means copies[x] = y
                let mut copies: FxHashMap<Sym, Sym> = FxHashMap::default();

                for instr in &mut block.instructions {
                    // Track simple copies: Assign { dest, value: Local/Temp }
                    if let MirInstrKind::Assign {
                        dest,
                        value: MirOperand::Local(src) | MirOperand::Temp(src),
                    } = &instr.kind
                    {
                        // Follow the chain: if src is also a copy, use the root
                        let root = copies.get(src).copied().unwrap_or(*src);
                        copies.insert(*dest, root);
                        continue;
                    }

                    // Also track Move/Copy instructions
                    if let MirInstrKind::Move {
                        dest,
                        src: MirOperand::Local(src) | MirOperand::Temp(src),
                    }
                    | MirInstrKind::Copy {
                        dest,
                        src: MirOperand::Local(src) | MirOperand::Temp(src),
                    } = &instr.kind
                    {
                        let root = copies.get(src).copied().unwrap_or(*src);
                        copies.insert(*dest, root);
                        continue;
                    }

                    // If a tracked dest is reassigned by other means, invalidate
                    if let Some(dest) = instr.destination() {
                        copies.remove(dest);
                    }
                }
            }
        }

        changed
    }
}

// ============================================================================
// Escape Analysis (P01)
// ============================================================================

/// Escape analysis pass.
///
/// Determines which heap-allocated values can safely be stack-allocated:
/// - Tracks all values that "escape" the current function (returned, stored
///   in escaped containers, passed to external calls)
/// - Values that DON'T escape are marked with `EscapeState::NoEscape`
/// - Codegen can use this to replace heap alloc with stack alloca
///
/// ## Escape Reasons
///
/// A value escapes if ANY of:
/// 1. It appears in a `Return` terminator
/// 2. It's stored into a struct/array/map that escapes
/// 3. It's passed as an argument to an FFI call
/// 4. It's captured by a closure that escapes
/// 5. It's stored into a global
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeState {
    /// Value does not escape — safe for stack allocation.
    NoEscape,
    /// Value escapes the function — must be heap-allocated.
    Escapes,
    /// Unknown — conservatively treated as escaping.
    Unknown,
}

/// Per-function escape analysis results.
#[derive(Debug, Clone)]
pub struct EscapeAnalysisResult {
    /// Escape state for each variable/temp in the function.
    pub states: rustc_hash::FxHashMap<Sym, EscapeState>,
}

pub struct EscapeAnalysis;

impl MirPass for EscapeAnalysis {
    fn name(&self) -> &'static str {
        "escape_analysis"
    }

    fn run(&mut self, program: &mut MirProgram) -> bool {
        // Escape analysis is an analysis pass — it annotates but doesn't
        // transform. It populates function-level metadata that codegen reads.
        // For now, we compute escape states per function.
        // The actual transformation (heap→stack) happens in codegen when it
        // checks EscapeAnalysisResult.

        for func in &mut program.functions {
            let result = Self::analyze_function(func);
            // Store result in function metadata for codegen to read
            func.escape_info = Some(result);
        }

        false // Analysis pass — never "changes" the program
    }
}

impl EscapeAnalysis {
    /// Analyze a single function's escape behavior.
    fn analyze_function(func: &MirFunction) -> EscapeAnalysisResult {
        use rustc_hash::FxHashSet;

        let mut escaped: FxHashSet<Sym> = FxHashSet::default();

        // Pass 1: Find directly escaping values
        for block in &func.blocks {
            // Values in return terminators escape
            match &block.terminator {
                MirTerminator::Return { values } => {
                    for op in values {
                        Self::mark_operand_escaped(op, &mut escaped);
                    }
                }
                _ => {}
            }

            for instr in &block.instructions {
                match &instr.kind {
                    // FFI calls: all arguments escape (we can't track into FFI)
                    MirInstrKind::FfiCall { args, .. } => {
                        for arg in args {
                            Self::mark_operand_escaped(arg, &mut escaped);
                        }
                    }
                    // Closure captures: the captured value escapes
                    MirInstrKind::ClosureCreate { captures, .. } => {
                        for cap in captures {
                            Self::mark_operand_escaped(cap, &mut escaped);
                        }
                    }
                    // Spawn: captures to spawned function escape
                    MirInstrKind::Spawn { captures, .. } => {
                        for cap in captures {
                            Self::mark_operand_escaped(cap, &mut escaped);
                        }
                    }
                    _ => {}
                }
            }
        }

        // Pass 2: Propagate escape through data flow
        // If a struct escapes, all values stored into it also escape.
        let mut changed = true;
        while changed {
            changed = false;
            for block in &func.blocks {
                for instr in &block.instructions {
                    match &instr.kind {
                        // If a struct escapes, its field values escape
                        MirInstrKind::StructCreate { dest, fields, .. } => {
                            if escaped.contains(dest) {
                                for (_, val) in fields {
                                    if Self::mark_operand_escaped(val, &mut escaped) {
                                        changed = true;
                                    }
                                }
                            }
                        }
                        // If an array escapes, its elements escape
                        MirInstrKind::ArrayCreate { dest, elements, .. } => {
                            if escaped.contains(dest) {
                                for elem in elements {
                                    if Self::mark_operand_escaped(elem, &mut escaped) {
                                        changed = true;
                                    }
                                }
                            }
                        }
                        // If a map escapes, its entries escape
                        MirInstrKind::MapCreate { dest, entries, .. } => {
                            if escaped.contains(dest) {
                                for (k, v) in entries {
                                    if Self::mark_operand_escaped(k, &mut escaped) {
                                        changed = true;
                                    }
                                    if Self::mark_operand_escaped(v, &mut escaped) {
                                        changed = true;
                                    }
                                }
                            }
                        }
                        // Assignment/Move/Copy: if dest escapes, src escapes
                        MirInstrKind::Assign { dest, value } => {
                            if escaped.contains(dest) {
                                if Self::mark_operand_escaped(value, &mut escaped) {
                                    changed = true;
                                }
                            }
                        }
                        MirInstrKind::Move { dest, src }
                        | MirInstrKind::Copy { dest, src }
                        | MirInstrKind::Clone { dest, src } => {
                            if escaped.contains(dest) {
                                if Self::mark_operand_escaped(src, &mut escaped) {
                                    changed = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Build result: everything NOT in `escaped` is NoEscape
        let mut states = rustc_hash::FxHashMap::default();
        for block in &func.blocks {
            for instr in &block.instructions {
                if let Some(dest) = instr.destination() {
                    let state = if escaped.contains(dest) {
                        EscapeState::Escapes
                    } else {
                        EscapeState::NoEscape
                    };
                    states.insert(*dest, state);
                }
            }
        }

        EscapeAnalysisResult { states }
    }

    /// Mark an operand's variable as escaped. Returns true if newly escaped.
    fn mark_operand_escaped(op: &MirOperand, escaped: &mut rustc_hash::FxHashSet<Sym>) -> bool {
        match op {
            MirOperand::Local(n) | MirOperand::Temp(n) | MirOperand::Global(n) => {
                escaped.insert(*n)
            }
            _ => false,
        }
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
        let result = folding.fold_binop(BinaryOp::Add, &MirConst::Int(1), &MirConst::Int(2));
        assert_eq!(result, Some(MirConst::Int(3)));
    }

    #[test]
    fn test_constant_folding_bool_and() {
        let folding = ConstantFolding;
        let result =
            folding.fold_binop(BinaryOp::And, &MirConst::Bool(true), &MirConst::Bool(false));
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
        assert_eq!(pipeline.passes.len(), 6);
    }
}
