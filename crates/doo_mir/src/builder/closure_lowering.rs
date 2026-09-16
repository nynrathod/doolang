//! Closure lowering — transforms closures into function objects.
//!
//! A closure becomes:
//! 1. An environment struct holding captured variables
//! 2. An anonymous function taking the environment as first parameter
//! 3. A closure value: { function_pointer, environment_pointer }
//!
//! If the closure escapes its defining scope (returned, stored in heap,
//! passed to go/async), the environment is heap-allocated.
//! Otherwise, the environment is stack-allocated for zero-cost inline closures.

use crate::sym::Sym;
use crate::types::Span;
use crate::types::*;

/// How a variable is captured into a closure environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureCaptureMode {
    /// Captured by bitwise copy (Copy types only).
    Copy,
    /// Captured by move — ownership transfers to the closure.
    Move,
}

/// A single captured variable in a closure environment.
#[derive(Debug, Clone)]
pub struct ClosureCapture {
    /// Name of the captured variable in the outer scope.
    pub name: Sym,
    /// Type of the captured variable.
    pub ty: MirType,
    /// How the variable is captured (Copy or Move).
    pub mode: ClosureCaptureMode,
}

/// Whether the closure escapes its defining scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureEscape {
    /// Closure stays inline — stack-allocate environment.
    NoEscape,
    /// Closure escapes — heap-allocate environment.
    Escapes,
}

/// Result of closure lowering analysis.
#[derive(Debug, Clone)]
pub struct ClosureLoweringInfo {
    /// Captured variables in order.
    pub captures: Vec<ClosureCapture>,
    /// Whether the closure escapes (determines heap vs stack allocation).
    pub escape: ClosureEscape,
    /// Parameter types of the closure function (excluding env param).
    pub param_types: Vec<MirType>,
    /// Return type of the closure.
    pub return_type: MirType,
}

impl ClosureLoweringInfo {
    /// Generate the environment struct field list.
    pub fn env_fields(&self) -> Vec<(String, MirType)> {
        self.captures
            .iter()
            .enumerate()
            .map(|(i, c)| (format!("__cap_{}", i), c.ty.clone()))
            .collect()
    }

    /// Generate the anonymous function's full parameter list (env + params).
    pub fn function_params(&self) -> Vec<(String, MirType)> {
        let mut params = Vec::with_capacity(1 + self.param_types.len());

        // First param is always the environment pointer
        params.push(("__env".to_string(), MirType::Ptr(Box::new(MirType::Int))));

        for (i, ty) in self.param_types.iter().enumerate() {
            params.push((format!("__param_{}", i), ty.clone()));
        }

        params
    }
}

/// Generate a unique name for the anonymous closure function.
pub fn closure_function_name(closure_id: usize) -> String {
    format!("__closure_{}", closure_id)
}

/// Generate the environment struct type name.
pub fn env_type_name(closure_id: usize) -> String {
    format!("__ClosureEnv_{}", closure_id)
}

/// Emit instructions to create the closure environment.
///
/// For each capture:
/// - Copy mode: emit a copy/store of the value
/// - Move mode: emit a move (source becomes invalid)
///
/// If escaping, heap-allocate the environment struct.
/// If inline, stack-allocate.
pub fn emit_env_creation(
    info: &ClosureLoweringInfo,
    closure_id: usize,
    span: Span,
) -> (Sym, Vec<MirInstr>) {
    let env_name = Sym::from(format!("__env_{}", closure_id).as_str());
    let mut instrs = Vec::new();

    // Allocate environment: heap if escaping, stack if inline
    match info.escape {
        ClosureEscape::Escapes => {
            instrs.push(MirInstr {
                kind: MirInstrKind::Call {
                    dest: Some(env_name),
                    func: Sym::from("doo_alloc"),
                    args: vec![],
                },
                span,
            });
        }
        ClosureEscape::NoEscape => {
            instrs.push(MirInstr {
                kind: MirInstrKind::Call {
                    dest: Some(env_name),
                    func: Sym::from("doo_stack_alloc"),
                    args: vec![],
                },
                span,
            });
        }
    }

    // Store each capture into the environment struct
    for (i, capture) in info.captures.iter().enumerate() {
        match capture.mode {
            ClosureCaptureMode::Copy => {
                instrs.push(MirInstr {
                    kind: MirInstrKind::FieldSet {
                        object: MirOperand::Local(env_name),
                        field: Sym::from(&format!("__cap_{}", i)),
                        value: MirOperand::Local(capture.name),
                    },
                    span,
                });
            }
            ClosureCaptureMode::Move => {
                instrs.push(MirInstr {
                    kind: MirInstrKind::Move {
                        dest: Sym::from(format!("__cap_tmp_{}", i).as_str()),
                        src: MirOperand::Local(capture.name),
                    },
                    span,
                });
                instrs.push(MirInstr {
                    kind: MirInstrKind::FieldSet {
                        object: MirOperand::Local(env_name),
                        field: Sym::from(&format!("__cap_{}", i)),
                        value: MirOperand::Local(Sym::from(format!("__cap_tmp_{}", i).as_str())),
                    },
                    span,
                });
            }
        }
    }

    (env_name, instrs)
}

/// Emit the closure creation instruction.
///
/// Produces a closure value: { function_pointer, environment_pointer }
pub fn emit_closure_create(
    info: &ClosureLoweringInfo,
    closure_id: usize,
    env_name: Sym,
    dest: Sym,
    span: Span,
) -> MirInstr {
    let func_name = Sym::from(closure_function_name(closure_id).as_str());

    let captures: Vec<MirOperand> = info
        .captures
        .iter()
        .map(|c| MirOperand::Local(c.name))
        .collect();

    MirInstr {
        kind: MirInstrKind::ClosureCreate {
            dest,
            func: func_name,
            captures,
        },
        span,
    }
}

/// Emit instructions to call a closure.
///
/// Extracts the function pointer and environment from the closure value,
/// then calls the function with env as the first argument.
pub fn emit_closure_call(
    closure: MirOperand,
    args: Vec<MirOperand>,
    dest: Sym,
    _ret_ty: MirType,
    span: Span,
) -> Vec<MirInstr> {
    let mut instrs = Vec::new();

    // Extract function pointer: closure.func
    let func_dest = Sym::from("__closure_func");
    instrs.push(MirInstr {
        kind: MirInstrKind::FieldGet {
            dest: func_dest,
            object: closure.clone(),
            field: Sym::from("func"),
        },
        span,
    });

    // Extract environment pointer: closure.env
    let env_dest = Sym::from("__closure_env");
    instrs.push(MirInstr {
        kind: MirInstrKind::FieldGet {
            dest: env_dest,
            object: closure,
            field: Sym::from("env"),
        },
        span,
    });

    // Build call with env as first argument
    let mut call_args = vec![MirOperand::Local(env_dest)];
    call_args.extend(args);

    instrs.push(MirInstr {
        kind: MirInstrKind::Call {
            dest: Some(dest),
            func: func_dest,
            args: call_args,
        },
        span,
    });

    instrs
}

/// Emit instructions to drop a closure's environment.
///
/// Drops each captured field in reverse order,
/// then frees the environment if it was heap-allocated.
pub fn emit_closure_drop(
    closure: MirOperand,
    escapes: bool,
    num_captures: usize,
    span: Span,
) -> Vec<MirInstr> {
    let mut instrs = Vec::new();

    // Extract environment pointer
    let env_dest = Sym::from("__closure_env_drop");
    instrs.push(MirInstr {
        kind: MirInstrKind::FieldGet {
            dest: env_dest,
            object: closure,
            field: Sym::from("env"),
        },
        span,
    });

    // Drop each field in reverse order
    for i in (0..num_captures).rev() {
        let field_dest = Sym::from(format!("__cap_drop_{}", i).as_str());
        instrs.push(MirInstr {
            kind: MirInstrKind::FieldGet {
                dest: field_dest,
                object: MirOperand::Local(env_dest),
                field: Sym::from(&format!("__cap_{}", i)),
            },
            span,
        });
        instrs.push(MirInstr {
            kind: MirInstrKind::Drop { value: field_dest },
            span,
        });
    }

    // Free environment if heap-allocated
    if escapes {
        instrs.push(MirInstr {
            kind: MirInstrKind::Call {
                dest: None,
                func: Sym::from("doo_free"),
                args: vec![MirOperand::Local(env_dest)],
            },
            span,
        });
    }

    instrs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_fields() {
        let info = ClosureLoweringInfo {
            captures: vec![
                ClosureCapture {
                    name: Sym::from("x"),
                    ty: MirType::Int,
                    mode: ClosureCaptureMode::Copy,
                },
                ClosureCapture {
                    name: Sym::from("y"),
                    ty: MirType::Str,
                    mode: ClosureCaptureMode::Move,
                },
            ],
            escape: ClosureEscape::NoEscape,
            param_types: vec![MirType::Int],
            return_type: MirType::Int,
        };

        let fields = info.env_fields();
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn test_function_params() {
        let info = ClosureLoweringInfo {
            captures: vec![],
            escape: ClosureEscape::NoEscape,
            param_types: vec![MirType::Int, MirType::Bool],
            return_type: MirType::Float,
        };

        let params = info.function_params();
        assert_eq!(params.len(), 3); // env + 2 params
    }

    #[test]
    fn test_closure_function_name() {
        assert_eq!(closure_function_name(0), "__closure_0");
        assert_eq!(closure_function_name(42), "__closure_42");
    }
}
