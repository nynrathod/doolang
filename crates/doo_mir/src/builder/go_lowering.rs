//! Go block lowering — transforms `go { expr }` into spawned tasks.
//!
//! - All captures must be Copy or Move — NO borrow across go boundary
//! - The go body becomes a separate function
//! - Emits a Spawn instruction that creates a new task
//! - Returns a task handle for joining

use crate::sym::Sym;
use crate::types::Span;
use crate::types::*;

/// Capture mode for go block — must be Copy or Move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoCaptureMode {
    /// Captured by bitwise copy (Copy types only).
    Copy,
    /// Captured by move — ownership transfers to the spawned task.
    Move,
}

/// A captured variable for a go block.
#[derive(Debug, Clone)]
pub struct GoCapture {
    /// Name of the captured variable.
    pub name: Sym,
    /// Type of the captured variable.
    pub ty: MirType,
    /// Capture mode (must be Copy or Move, never Borrow).
    pub mode: GoCaptureMode,
}

/// Result of go block lowering analysis.
#[derive(Debug, Clone)]
pub struct GoLoweringInfo {
    /// Captured variables (all must be Copy or Move).
    pub captures: Vec<GoCapture>,
}

/// Error when a go block capture violates the no-borrow rule.
#[derive(Debug, Clone)]
pub struct GoCaptureError {
    /// Name of the variable that was borrowed.
    pub var_name: String,
    /// Span of the go block.
    pub span: Span,
    /// Human-readable error message.
    pub message: String,
}

impl std::fmt::Display for GoCaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Verify that all captures are Copy or Move.
///
/// Returns an error if any capture would require a borrow across the go
pub fn verify_go_captures(captures: &[GoCapture]) -> Result<(), GoCaptureError> {
    for capture in captures {
        match capture.mode {
            GoCaptureMode::Copy | GoCaptureMode::Move => {}
        }
    }
    Ok(())
}

/// Generate a unique name for the spawned function.
pub fn go_function_name(go_block_id: usize) -> String {
    format!("__go_body_{}", go_block_id)
}

/// Emit instructions to create the go task.
///
/// For each capture:
/// - Copy mode: store a copy of the value
/// - Move mode: move ownership to the task (source becomes invalid)
///
/// Then emit a Spawn instruction that creates the task.
pub fn emit_go_spawn(
    info: &GoLoweringInfo,
    go_block_id: usize,
    dest: Sym,
    span: Span,
) -> Vec<MirInstr> {
    let mut instrs = Vec::new();

    let func_name = Sym::from(go_function_name(go_block_id).as_str());

    // Build capture list as operands
    let captures: Vec<MirOperand> = info
        .captures
        .iter()
        .map(|c| match c.mode {
            GoCaptureMode::Copy => MirOperand::Local(c.name),
            GoCaptureMode::Move => MirOperand::Local(c.name),
        })
        .collect();

    // Emit Spawn instruction
    instrs.push(MirInstr {
        kind: MirInstrKind::Spawn {
            dest,
            func: func_name,
            captures,
        },
        span,
    });

    instrs
}

/// Emit blocks for the spawned function body.
///
/// The spawned function:
/// 1. Receives captures as parameters
/// 2. Executes the go body
/// 3. Returns void (go blocks don't return values to the caller)
pub fn emit_go_function_body(
    _info: &GoLoweringInfo,
    body_blocks: Vec<MirBlock>,
    _span: Span,
) -> Vec<MirBlock> {
    body_blocks
}

/// Determine capture mode for a go block variable.
///
/// 1. Is it Copy? → Copy
/// 2. Is it consumed (moved/stored/returned)? → Move
/// 3. Otherwise → error (borrow not allowed across go boundary)
pub fn determine_capture_mode(
    is_copy_type: bool,
    is_consumed: bool,
) -> Result<GoCaptureMode, &'static str> {
    if is_copy_type {
        Ok(GoCaptureMode::Copy)
    } else if is_consumed {
        Ok(GoCaptureMode::Move)
    } else {
        Err("cannot borrow across go boundary — variable must be Copy or Move")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_go_function_name() {
        assert_eq!(go_function_name(0), "__go_body_0");
        assert_eq!(go_function_name(7), "__go_body_7");
    }

    #[test]
    fn test_verify_copies_and_moves() {
        let captures = vec![
            GoCapture {
                name: Sym::from("x"),
                ty: MirType::Int,
                mode: GoCaptureMode::Copy,
            },
            GoCapture {
                name: Sym::from("y"),
                ty: MirType::Str,
                mode: GoCaptureMode::Move,
            },
        ];
        assert!(verify_go_captures(&captures).is_ok());
    }

    #[test]
    fn test_determine_mode_copy() {
        let mode = determine_capture_mode(true, false).unwrap();
        assert_eq!(mode, GoCaptureMode::Copy);
    }

    #[test]
    fn test_determine_mode_move() {
        let mode = determine_capture_mode(false, true).unwrap();
        assert_eq!(mode, GoCaptureMode::Move);
    }

    #[test]
    fn test_determine_mode_borrow_rejected() {
        let result = determine_capture_mode(false, false);
        assert!(result.is_err());
    }
}
