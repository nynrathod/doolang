//! Compile-Time Function Evaluation (CTFE) for the Doo compiler.
//!
//! Provides a restricted interpreter for evaluating `const` expressions
//! at compile time. Supports arithmetic, comparisons, conditionals,
//! loops with constant bounds, and literal construction.
//!
//! Forbidden operations (@extern calls, I/O, randomness, async) are
//! rejected. A step limit prevents infinite loops from hanging.

mod interpreter;
mod limits;

pub use interpreter::{CtfeError, CtfeHeap, CtfeInterpreter, CtfeValue, StackFrame};
pub use limits::{is_forbidden_expr, is_forbidden_stmt, DEFAULT_STEP_LIMIT};

/// Evaluate a const expression with the default step limit.
///
/// Called by `doo_analysis` when a `const` declaration needs a concrete value.
/// The result is inlined at every use site (no address).
pub fn eval_const(expr: &doo_thir::ThirExpr) -> Result<CtfeValue, CtfeError> {
    eval_const_with_limit(expr, DEFAULT_STEP_LIMIT)
}

/// Evaluate a const expression with a custom step limit.
pub fn eval_const_with_limit(
    expr: &doo_thir::ThirExpr,
    step_limit: u64,
) -> Result<CtfeValue, CtfeError> {
    let mut interpreter = CtfeInterpreter::with_step_limit(step_limit);
    match interpreter.eval(expr) {
        Ok(val) => Ok(val),
        Err(CtfeError::ReturnSignal(val)) => Ok(val),
        Err(e) => Err(e),
    }
}
