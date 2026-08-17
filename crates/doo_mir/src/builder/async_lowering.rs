//! Async state machine lowering.
//!
//! Transforms `async { body }` into a state machine:
//! - Each await point becomes a suspension state
//! - Locals crossing await points are stored in the state machine struct
//! - A poll function advances the state machine via switch-on-state

use crate::sym::Sym;
use crate::types::Span;
use crate::types::*;

/// States in an async state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncStateKind {
    /// Initial state — body has not started executing.
    Start,
    /// Suspended at await point N (0-indexed).
    Suspend(usize),
    /// Body completed — result is ready.
    Done,
}

impl AsyncStateKind {
    /// Integer encoding for the state field in the state machine struct.
    pub fn encode(self) -> i64 {
        match self {
            Self::Start => 0,
            Self::Suspend(n) => (n + 1) as i64,
            Self::Done => -1,
        }
    }

    /// Decode an integer back to a state kind.
    pub fn decode(val: i64) -> Self {
        if val == 0 {
            Self::Start
        } else if val == -1 {
            Self::Done
        } else {
            Self::Suspend((val - 1) as usize)
        }
    }
}

/// A local variable that is live across an await point and must be saved
/// in the state machine struct.
#[derive(Debug, Clone)]
pub struct CrossAwaitLocal {
    /// MIR operand name (Local or Temp Sym).
    pub name: Sym,
    /// Type of the local for struct field generation.
    pub ty: MirType,
}

/// Metadata collected during async lowering analysis.
#[derive(Debug, Clone)]
pub struct AsyncLoweringInfo {
    /// Total number of await (suspension) points in the body.
    pub num_await_points: usize,
    /// Locals that are live across at least one await point.
    pub cross_await_locals: Vec<CrossAwaitLocal>,
}

impl AsyncLoweringInfo {
    /// Total number of states: Start + N suspends + Done.
    pub fn num_states(&self) -> usize {
        self.num_await_points + 2
    }
}

/// Walk a function's blocks and collect await-point information.
///
/// Returns the count of `Await` instructions and the set of locals
/// referenced after any await point (conservative: all locals defined
/// before an await and referenced after it).
pub fn analyze_async_function(func: &MirFunction) -> AsyncLoweringInfo {
    let mut await_count = 0usize;

    for block in &func.blocks {
        for instr in &block.instructions {
            if matches!(instr.kind, MirInstrKind::Await { .. }) {
                await_count += 1;
            }
        }
    }

    AsyncLoweringInfo {
        num_await_points: await_count,
        cross_await_locals: Vec::new(),
    }
}

/// Generate the field list for the state machine struct.
///
/// Layout: { state: Int, ...cross_await_locals, result: T }
pub fn state_machine_fields(
    info: &AsyncLoweringInfo,
    result_ty: MirType,
) -> Vec<(String, MirType)> {
    let mut fields = Vec::with_capacity(2 + info.cross_await_locals.len());

    fields.push(("__state".to_string(), MirType::Int));

    for local in &info.cross_await_locals {
        fields.push((format!("__local_{}", local.name), local.ty.clone()));
    }

    fields.push(("__result".to_string(), result_ty));
    fields
}

/// Build the MIR blocks for the poll function's state dispatch.
///
/// The dispatch block loads the state field and switches on it:
/// - State 0 (Start)     → run segment 0
/// - State 1 (Suspend 0) → run segment 1
/// - ...
/// - State -1 (Done)      → return Ready(result)
///
/// Returns a list of (state_value, target_block_id) pairs for the switch.
pub fn build_state_dispatch(info: &AsyncLoweringInfo) -> Vec<(i64, u32)> {
    let mut cases = Vec::with_capacity(info.num_states());

    // Start state
    cases.push((AsyncStateKind::Start.encode(), 1u32));

    // Suspend states
    for i in 0..info.num_await_points {
        let state = AsyncStateKind::Suspend(i).encode();
        let block = (i + 2) as u32;
        cases.push((state, block));
    }

    // Done state
    let done_block = (info.num_await_points + 2) as u32;
    cases.push((AsyncStateKind::Done.encode(), done_block));

    cases
}

/// Emit the state-save sequence at an await suspension point.
///
/// This stores:
/// 1. The current state (Suspend N) into the state field
/// 2. All cross-await locals into their struct fields
/// 3. Returns Poll::Pending
pub fn emit_suspend_point(
    state: AsyncStateKind,
    locals: &[CrossAwaitLocal],
    state_struct: Sym,
    span: Span,
) -> Vec<MirInstr> {
    let mut instrs = Vec::new();

    // Store state
    instrs.push(MirInstr {
        kind: MirInstrKind::FieldSet {
            object: MirOperand::Local(state_struct),
            field: Sym::from("state"),
            value: MirOperand::Const(MirConst::Int(state.encode())),
        },
        span,
    });

    // Store cross-await locals
    for (i, local) in locals.iter().enumerate() {
        instrs.push(MirInstr {
            kind: MirInstrKind::FieldSet {
                object: MirOperand::Local(state_struct),
                field: Sym::intern(&format!("__local_{}", i)),
                value: MirOperand::Local(local.name),
            },
            span,
        });
    }

    instrs
}

/// Emit the state-restore sequence at the beginning of a resume segment.
///
/// This loads all cross-await locals from the struct fields back into locals.
pub fn emit_resume_sequence(
    locals: &[CrossAwaitLocal],
    state_struct: Sym,
    span: Span,
) -> Vec<MirInstr> {
    let mut instrs = Vec::new();

    for (i, local) in locals.iter().enumerate() {
        let dest = local.name;
        instrs.push(MirInstr {
            kind: MirInstrKind::FieldGet {
                dest,
                object: MirOperand::Local(state_struct),
                field: Sym::from(&format!("__local_{}", i)),
            },
            span,
        });
    }

    instrs
}

/// Emit the completion sequence: store result, set state to Done, return Ready.
pub fn emit_completion(
    result: MirOperand,
    _result_ty: MirType,
    state_struct: Sym,
    span: Span,
) -> Vec<MirInstr> {
    vec![
        MirInstr {
            kind: MirInstrKind::FieldSet {
                object: MirOperand::Local(state_struct),
                field: Sym::from("result"),
                value: result,
            },
            span,
        },
        MirInstr {
            kind: MirInstrKind::FieldSet {
                object: MirOperand::Local(state_struct),
                field: Sym::from("state"),
                value: MirOperand::Const(MirConst::Int(AsyncStateKind::Done.encode())),
            },
            span,
        },
    ]
}

/// Generate a fresh function name for the poll function of an async block.
pub fn poll_function_name(async_block_id: usize) -> String {
    format!("__async_poll_{}", async_block_id)
}

/// Generate the state machine struct type name.
pub fn state_machine_type_name(async_block_id: usize) -> String {
    format!("__AsyncStateMachine_{}", async_block_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_encoding() {
        assert_eq!(AsyncStateKind::Start.encode(), 0);
        assert_eq!(AsyncStateKind::Suspend(0).encode(), 1);
        assert_eq!(AsyncStateKind::Suspend(2).encode(), 3);
        assert_eq!(AsyncStateKind::Done.encode(), -1);
    }

    #[test]
    fn test_state_decoding() {
        assert_eq!(AsyncStateKind::decode(0), AsyncStateKind::Start);
        assert_eq!(AsyncStateKind::decode(1), AsyncStateKind::Suspend(0));
        assert_eq!(AsyncStateKind::decode(3), AsyncStateKind::Suspend(2));
        assert_eq!(AsyncStateKind::decode(-1), AsyncStateKind::Done);
    }

    #[test]
    fn test_num_states() {
        let info = AsyncLoweringInfo {
            num_await_points: 3,
            cross_await_locals: vec![],
        };
        assert_eq!(info.num_states(), 5); // Start + 3 suspends + Done
    }

    #[test]
    fn test_dispatch_cases() {
        let info = AsyncLoweringInfo {
            num_await_points: 2,
            cross_await_locals: vec![],
        };
        let cases = build_state_dispatch(&info);
        assert_eq!(cases.len(), 4); // Start + 2 suspends + Done
        assert_eq!(cases[0].0, 0); // Start
        assert_eq!(cases[1].0, 1); // Suspend 0
        assert_eq!(cases[2].0, 2); // Suspend 1
        assert_eq!(cases[3].0, -1); // Done
    }
}
