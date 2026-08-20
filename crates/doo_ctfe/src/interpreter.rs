//! CTFE Interpreter — evaluates THIR expressions at compile time.

use doo_core::types::builtin;
use doo_core::types::TypeId;
use doo_thir::{
    ThirArm, ThirBinOp, ThirExpr, ThirExprKind, ThirLiteral, ThirPattern, ThirPatternKind,
    ThirStmt, ThirStmtKind, ThirUnOp,
};
use rustc_hash::FxHashMap;

use crate::limits::{is_forbidden_expr, is_forbidden_stmt, DEFAULT_STEP_LIMIT};

// ============================================================================
// CtfeValue
// ============================================================================

/// A compile-time value produced by the CTFE interpreter.
#[derive(Debug, Clone)]
pub enum CtfeValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Null,
    Array(Vec<CtfeValue>),
    Map(Vec<(CtfeValue, CtfeValue)>),
    Struct {
        name: String,
        fields: FxHashMap<String, CtfeValue>,
    },
    Enum {
        name: String,
        variant: String,
        payload: Option<Vec<CtfeValue>>,
    },
    Tuple(Vec<CtfeValue>),
}

impl CtfeValue {
    /// Check if two values are equal (for pattern matching).
    fn equals(&self, other: &CtfeValue) -> bool {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Null, Self::Null) => true,
            _ => false,
        }
    }

    /// Convert to boolean for conditionals.
    fn as_bool(&self) -> Result<bool, CtfeError> {
        match self {
            Self::Bool(b) => Ok(*b),
            Self::Int(i) => Ok(*i != 0),
            _ => Err(CtfeError::TypeMismatch(format!(
                "expected Bool, got {:?}",
                self
            ))),
        }
    }

    /// Convert to integer for indexing.
    fn as_int(&self) -> Result<i64, CtfeError> {
        match self {
            Self::Int(i) => Ok(*i),
            _ => Err(CtfeError::TypeMismatch(format!(
                "expected Int, got {:?}",
                self
            ))),
        }
    }
}

impl std::fmt::Display for CtfeValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int(i) => write!(f, "{}", i),
            Self::Float(fl) => write!(f, "{}", fl),
            Self::Bool(b) => write!(f, "{}", b),
            Self::String(s) => write!(f, "\"{}\"", s),
            Self::Null => write!(f, "null"),
            Self::Array(_) => write!(f, "[...]"),
            Self::Map(_) => write!(f, "{{...}}"),
            Self::Struct { name, .. } => write!(f, "{}{{...}}", name),
            Self::Enum { name, variant, .. } => write!(f, "{}::{}", name, variant),
            Self::Tuple(_) => write!(f, "(...)"),
        }
    }
}

// ============================================================================
// CtfeError
// ============================================================================

/// Errors from CTFE evaluation.
#[derive(Debug)]
pub enum CtfeError {
    /// Step limit exceeded — possible infinite loop.
    StepLimitExceeded,
    /// Operation not allowed in const context.
    ForbiddenOperation(String),
    /// Division or modulo by zero.
    DivisionByZero,
    /// Arithmetic overflow.
    Overflow,
    /// Type mismatch during evaluation.
    TypeMismatch(String),
    /// Variable not found in scope.
    UnboundVariable(String),
    /// Operation not yet implemented in CTFE.
    NotImplemented(String),
    /// Match expression had no matching arm.
    NonExhaustiveMatch,
    /// Internal control flow signal — not a real error.
    ReturnSignal(CtfeValue),
    /// Internal control flow signal — not a real error.
    BreakSignal(Option<CtfeValue>),
    /// Internal control flow signal — not a real error.
    ContinueSignal,
}

impl std::fmt::Display for CtfeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StepLimitExceeded => {
                write!(
                    f,
                    "compile-time evaluation exceeded step limit (possible infinite loop)"
                )
            }
            Self::ForbiddenOperation(msg) => {
                write!(f, "forbidden in const context: {}", msg)
            }
            Self::DivisionByZero => {
                write!(f, "division by zero in const expression")
            }
            Self::Overflow => {
                write!(f, "arithmetic overflow in const expression")
            }
            Self::TypeMismatch(msg) => {
                write!(f, "type mismatch in const expression: {}", msg)
            }
            Self::UnboundVariable(name) => {
                write!(f, "undefined variable '{}' in const expression", name)
            }
            Self::NotImplemented(msg) => {
                write!(f, "not yet supported in const context: {}", msg)
            }
            Self::NonExhaustiveMatch => {
                write!(f, "non-exhaustive match in const expression")
            }
            Self::ReturnSignal(_) | Self::BreakSignal(_) | Self::ContinueSignal => {
                write!(f, "internal control flow signal")
            }
        }
    }
}

impl std::error::Error for CtfeError {}

// ============================================================================
// CtfeHeap
// ============================================================================

/// Heap for CTFE allocations.
///
/// Currently unused — CTFE uses by-value semantics with cloning.
/// Reserved for future mutable reference support in const context.
#[derive(Debug, Default)]
pub struct CtfeHeap {
    allocations: Vec<CtfeValue>,
}

impl CtfeHeap {
    pub fn new() -> Self {
        Self::default()
    }
}

// ============================================================================
// StackFrame
// ============================================================================

/// A stack frame holding local variable bindings.
#[derive(Debug, Clone)]
pub struct StackFrame {
    pub locals: FxHashMap<String, CtfeValue>,
}

impl StackFrame {
    pub fn new() -> Self {
        Self {
            locals: FxHashMap::default(),
        }
    }
}

impl Default for StackFrame {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// CtfeInterpreter
// ============================================================================

/// Restricted interpreter for compile-time function evaluation.
///
/// Evaluates THIR expressions in a pure, side-effect-free environment.
/// Tracks evaluation steps and aborts after exceeding the step limit.
pub struct CtfeInterpreter {
    stack: Vec<StackFrame>,
    heap: CtfeHeap,
    step_counter: u64,
    step_limit: u64,
}

impl CtfeInterpreter {
    /// Create a new interpreter with the default step limit.
    pub fn new() -> Self {
        Self::with_step_limit(DEFAULT_STEP_LIMIT)
    }

    /// Create a new interpreter with a custom step limit.
    pub fn with_step_limit(step_limit: u64) -> Self {
        Self {
            stack: vec![StackFrame::new()],
            heap: CtfeHeap::new(),
            step_counter: 0,
            step_limit,
        }
    }

    /// Increment the step counter and check the limit.
    fn tick(&mut self) -> Result<(), CtfeError> {
        self.step_counter += 1;
        if self.step_counter > self.step_limit {
            return Err(CtfeError::StepLimitExceeded);
        }
        Ok(())
    }

    /// Get the current (top) stack frame.
    fn frame(&self) -> &StackFrame {
        self.stack
            .last()
            .expect("stack must have at least one frame")
    }

    /// Get the current stack frame mutably.
    fn frame_mut(&mut self) -> &mut StackFrame {
        self.stack
            .last_mut()
            .expect("stack must have at least one frame")
    }

    /// Look up a variable in the current stack frame.
    fn lookup(&self, name: &str) -> Result<CtfeValue, CtfeError> {
        self.frame()
            .locals
            .get(name)
            .cloned()
            .ok_or_else(|| CtfeError::UnboundVariable(name.to_string()))
    }

    // ========================================================================
    // Expression Evaluation
    // ========================================================================

    /// Evaluate a THIR expression to a compile-time value.
    pub fn eval(&mut self, expr: &ThirExpr) -> Result<CtfeValue, CtfeError> {
        self.tick()?;

        // Check for forbidden operations
        if let Some(reason) = is_forbidden_expr(expr) {
            return Err(CtfeError::ForbiddenOperation(reason));
        }

        match &expr.kind {
            // === Literals ===
            ThirExprKind::Literal(lit) => Ok(literal_to_value(lit)),

            // === Variables ===
            ThirExprKind::Var(name) => self.lookup(name),

            // === Binary Operations ===
            ThirExprKind::Binary { op, lhs, rhs } => {
                let left = self.eval(lhs)?;
                let right = self.eval(rhs)?;
                self.eval_binary(*op, left, right)
            }

            // === Unary Operations ===
            ThirExprKind::Unary { op, expr } => {
                let val = self.eval(expr)?;
                self.eval_unary(*op, val)
            }

            // === Field Access ===
            ThirExprKind::FieldAccess {
                object,
                field,
                field_idx,
            } => {
                let obj = self.eval(object)?;
                match obj {
                    CtfeValue::Struct { fields, .. } => fields
                        .get(field)
                        .cloned()
                        .or_else(|| {
                            let entries: Vec<_> = fields.into_iter().collect();
                            entries.get(*field_idx).map(|(_, v)| v.clone())
                        })
                        .ok_or_else(|| {
                            CtfeError::TypeMismatch(format!("no field '{}' in struct", field))
                        }),
                    CtfeValue::Tuple(elements) => {
                        elements.get(*field_idx).cloned().ok_or_else(|| {
                            CtfeError::TypeMismatch(format!(
                                "tuple index {} out of bounds",
                                field_idx
                            ))
                        })
                    }
                    _ => Err(CtfeError::TypeMismatch(format!(
                        "cannot access field on {:?}",
                        obj
                    ))),
                }
            }

            // === Index Access ===
            ThirExprKind::Index { object, index } => {
                let obj = self.eval(object)?;
                let idx = self.eval(index)?.as_int()?;
                match obj {
                    CtfeValue::Array(elements) => {
                        let len = elements.len() as i64;
                        if idx < 0 || idx >= len {
                            return Err(CtfeError::TypeMismatch(format!(
                                "array index {} out of bounds (len {})",
                                idx, len
                            )));
                        }
                        Ok(elements[idx as usize].clone())
                    }
                    CtfeValue::Map(entries) => {
                        let key = self.eval(index)?;
                        for (k, v) in &entries {
                            if k.equals(&key) {
                                return Ok(v.clone());
                            }
                        }
                        Ok(CtfeValue::Null)
                    }
                    CtfeValue::Tuple(elements) => {
                        if idx < 0 || idx >= elements.len() as i64 {
                            return Err(CtfeError::TypeMismatch(format!(
                                "tuple index {} out of bounds",
                                idx
                            )));
                        }
                        Ok(elements[idx as usize].clone())
                    }
                    _ => Err(CtfeError::TypeMismatch(format!("cannot index {:?}", obj))),
                }
            }

            // === Array Literal ===
            ThirExprKind::ArrayLiteral(elements) => {
                let mut values = Vec::with_capacity(elements.len());
                for elem in elements {
                    values.push(self.eval(elem)?);
                }
                Ok(CtfeValue::Array(values))
            }

            // === Map Literal ===
            ThirExprKind::MapLiteral(entries) => {
                let mut pairs = Vec::with_capacity(entries.len());
                for (k_expr, v_expr) in entries {
                    let k = self.eval(k_expr)?;
                    let v = self.eval(v_expr)?;
                    pairs.push((k, v));
                }
                Ok(CtfeValue::Map(pairs))
            }

            // === Struct Literal ===
            ThirExprKind::StructLiteral { name, fields } => {
                let mut field_map = FxHashMap::default();
                for (field_name, field_expr) in fields {
                    let val = self.eval(field_expr)?;
                    field_map.insert(field_name.clone(), val);
                }
                Ok(CtfeValue::Struct {
                    name: name.clone(),
                    fields: field_map,
                })
            }

            // === Enum Variant ===
            ThirExprKind::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                let mut payload_vals = None;
                if !payload.is_empty() {
                    let mut vals = Vec::with_capacity(payload.len());
                    for p_expr in payload {
                        vals.push(self.eval(p_expr)?);
                    }
                    payload_vals = Some(vals);
                }
                Ok(CtfeValue::Enum {
                    name: enum_name.clone(),
                    variant: variant.clone(),
                    payload: payload_vals,
                })
            }

            // === Tuple ===
            ThirExprKind::Tuple(elements) => {
                let mut values = Vec::with_capacity(elements.len());
                for elem in elements {
                    values.push(self.eval(elem)?);
                }
                Ok(CtfeValue::Tuple(values))
            }

            // === Conditional ===
            ThirExprKind::If { cond, then, else_ } => {
                let cond_val = self.eval(cond)?;
                if cond_val.as_bool()? {
                    self.eval(then)
                } else if let Some(else_expr) = else_ {
                    self.eval(else_expr)
                } else {
                    Ok(CtfeValue::Null)
                }
            }

            // === Match ===
            ThirExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                let value = self.eval(scrutinee)?;
                self.eval_match(&value, arms)
            }

            // === Block ===
            ThirExprKind::Block(stmts, tail) => {
                self.push_frame();
                let result = self.eval_block_stmts(stmts);
                match result {
                    Ok(()) => {
                        if let Some(tail_expr) = tail {
                            let val = self.eval(tail_expr);
                            self.pop_frame();
                            val
                        } else {
                            self.pop_frame();
                            Ok(CtfeValue::Null)
                        }
                    }
                    Err(e) => {
                        self.pop_frame();
                        Err(e)
                    }
                }
            }

            // === Scope Block ===
            ThirExprKind::ScopeBlock { stmts } => {
                self.push_frame();
                let result = self.eval_block_stmts(stmts);
                self.pop_frame();
                match result {
                    Ok(()) => Ok(CtfeValue::Null),
                    Err(CtfeError::ReturnSignal(v)) => Ok(v),
                    Err(e) => Err(e),
                }
            }

            // === Range ===
            ThirExprKind::Range { .. } => Err(CtfeError::NotImplemented(
                "range values in const context".to_string(),
            )),

            // === Ok/Err Wrappers ===
            ThirExprKind::Ok(inner) => {
                let val = self.eval(inner)?;
                Ok(CtfeValue::Enum {
                    name: "Result".to_string(),
                    variant: "Ok".to_string(),
                    payload: Some(vec![val]),
                })
            }
            ThirExprKind::Err(inner) => {
                let val = self.eval(inner)?;
                Ok(CtfeValue::Enum {
                    name: "Result".to_string(),
                    variant: "Err".to_string(),
                    payload: Some(vec![val]),
                })
            }

            // === Ownership (no-ops in CTFE) ===
            ThirExprKind::Move(inner) | ThirExprKind::Clone(inner) => self.eval(inner),

            // === Cast ===
            ThirExprKind::Cast { value, to_type } => {
                let val = self.eval(value)?;
                self.eval_cast(val, *to_type)
            }

            // === Spread (should be desugared) ===
            ThirExprKind::Spread(_) => Err(CtfeError::NotImplemented(
                "spread in const context".to_string(),
            )),

            // === Forbidden (caught by is_forbidden_expr, but handle for safety) ===
            _ => Err(CtfeError::ForbiddenOperation(
                "this expression is not allowed in const context".to_string(),
            )),
        }
    }

    // ========================================================================
    // Binary Operation Evaluation
    // ========================================================================

    fn eval_binary(
        &self,
        op: ThirBinOp,
        left: CtfeValue,
        right: CtfeValue,
    ) -> Result<CtfeValue, CtfeError> {
        match (&left, &right) {
            // Integer arithmetic
            (CtfeValue::Int(a), CtfeValue::Int(b)) => match op {
                ThirBinOp::Add => left
                    .checked_add(*b)
                    .map(CtfeValue::Int)
                    .ok_or(CtfeError::Overflow),
                ThirBinOp::Sub => a
                    .checked_sub(*b)
                    .map(CtfeValue::Int)
                    .ok_or(CtfeError::Overflow),
                ThirBinOp::Mul => a
                    .checked_mul(*b)
                    .map(CtfeValue::Int)
                    .ok_or(CtfeError::Overflow),
                ThirBinOp::Div => {
                    if *b == 0 {
                        Err(CtfeError::DivisionByZero)
                    } else {
                        Ok(CtfeValue::Int(a / b))
                    }
                }
                ThirBinOp::Mod => {
                    if *b == 0 {
                        Err(CtfeError::DivisionByZero)
                    } else {
                        Ok(CtfeValue::Int(a % b))
                    }
                }
                ThirBinOp::Eq => Ok(CtfeValue::Bool(a == b)),
                ThirBinOp::NotEq => Ok(CtfeValue::Bool(a != b)),
                ThirBinOp::Lt => Ok(CtfeValue::Bool(a < b)),
                ThirBinOp::Gt => Ok(CtfeValue::Bool(a > b)),
                ThirBinOp::LtEq => Ok(CtfeValue::Bool(a <= b)),
                ThirBinOp::GtEq => Ok(CtfeValue::Bool(a >= b)),
                ThirBinOp::BitAnd => Ok(CtfeValue::Int(a & b)),
                ThirBinOp::BitOr => Ok(CtfeValue::Int(a | b)),
                ThirBinOp::BitXor => Ok(CtfeValue::Int(a ^ b)),
                _ => Err(CtfeError::NotImplemented(format!(
                    "binary op {:?} on Int",
                    op
                ))),
            },

            // Float arithmetic
            (CtfeValue::Float(a), CtfeValue::Float(b)) => match op {
                ThirBinOp::Add => Ok(CtfeValue::Float(a + b)),
                ThirBinOp::Sub => Ok(CtfeValue::Float(a - b)),
                ThirBinOp::Mul => Ok(CtfeValue::Float(a * b)),
                ThirBinOp::Div => {
                    if *b == 0.0 {
                        Err(CtfeError::DivisionByZero)
                    } else {
                        Ok(CtfeValue::Float(a / b))
                    }
                }
                ThirBinOp::Mod => Ok(CtfeValue::Float(a % b)),
                ThirBinOp::Eq => Ok(CtfeValue::Bool(a == b)),
                ThirBinOp::NotEq => Ok(CtfeValue::Bool(a != b)),
                ThirBinOp::Lt => Ok(CtfeValue::Bool(a < b)),
                ThirBinOp::Gt => Ok(CtfeValue::Bool(a > b)),
                ThirBinOp::LtEq => Ok(CtfeValue::Bool(a <= b)),
                ThirBinOp::GtEq => Ok(CtfeValue::Bool(a >= b)),
                _ => Err(CtfeError::NotImplemented(format!(
                    "binary op {:?} on Float",
                    op
                ))),
            },

            // Boolean logic
            (CtfeValue::Bool(a), CtfeValue::Bool(b)) => match op {
                ThirBinOp::And => Ok(CtfeValue::Bool(*a && *b)),
                ThirBinOp::Or => Ok(CtfeValue::Bool(*a || *b)),
                ThirBinOp::Eq => Ok(CtfeValue::Bool(a == b)),
                ThirBinOp::NotEq => Ok(CtfeValue::Bool(a != b)),
                ThirBinOp::BitAnd => Ok(CtfeValue::Bool(*a && *b)),
                ThirBinOp::BitOr => Ok(CtfeValue::Bool(*a || *b)),
                _ => Err(CtfeError::NotImplemented(format!(
                    "binary op {:?} on Bool",
                    op
                ))),
            },

            // String equality
            (CtfeValue::String(a), CtfeValue::String(b)) => match op {
                ThirBinOp::Eq => Ok(CtfeValue::Bool(a == b)),
                ThirBinOp::NotEq => Ok(CtfeValue::Bool(a != b)),
                ThirBinOp::Add => Ok(CtfeValue::String(format!("{}{}", a, b))),
                _ => Err(CtfeError::NotImplemented(format!(
                    "binary op {:?} on String",
                    op
                ))),
            },

            // Null equality
            (CtfeValue::Null, CtfeValue::Null) => match op {
                ThirBinOp::Eq => Ok(CtfeValue::Bool(true)),
                ThirBinOp::NotEq => Ok(CtfeValue::Bool(false)),
                _ => Err(CtfeError::TypeMismatch(
                    "only == and != are valid for null".to_string(),
                )),
            },

            _ => Err(CtfeError::TypeMismatch(format!(
                "cannot apply {:?} to {} and {}",
                op, left, right
            ))),
        }
    }

    // ========================================================================
    // Unary Operation Evaluation
    // ========================================================================

    fn eval_unary(&self, op: ThirUnOp, val: CtfeValue) -> Result<CtfeValue, CtfeError> {
        match (op, &val) {
            (ThirUnOp::Neg, CtfeValue::Int(i)) => i
                .checked_neg()
                .map(CtfeValue::Int)
                .ok_or(CtfeError::Overflow),
            (ThirUnOp::Neg, CtfeValue::Float(f)) => Ok(CtfeValue::Float(-f)),
            (ThirUnOp::Not, CtfeValue::Bool(b)) => Ok(CtfeValue::Bool(!b)),
            (ThirUnOp::Not, CtfeValue::Int(i)) => Ok(CtfeValue::Int(!i)),
            _ => Err(CtfeError::TypeMismatch(format!(
                "cannot apply {:?} to {}",
                op, val
            ))),
        }
    }

    // ========================================================================
    // Cast Evaluation
    // ========================================================================

    fn eval_cast(&self, val: CtfeValue, to_type: TypeId) -> Result<CtfeValue, CtfeError> {
        if to_type == builtin::FLOAT {
            match val {
                CtfeValue::Int(i) => Ok(CtfeValue::Float(i as f64)),
                _ => Ok(val),
            }
        } else if to_type == builtin::INT {
            match val {
                CtfeValue::Float(f) => Ok(CtfeValue::Int(f as i64)),
                CtfeValue::Bool(b) => Ok(CtfeValue::Int(if b { 1 } else { 0 })),
                _ => Ok(val),
            }
        } else if to_type == builtin::BOOL {
            match val {
                CtfeValue::Int(i) => Ok(CtfeValue::Bool(i != 0)),
                _ => Ok(val),
            }
        } else if to_type == builtin::STR {
            match val {
                CtfeValue::Int(i) => Ok(CtfeValue::String(i.to_string())),
                CtfeValue::Float(f) => Ok(CtfeValue::String(f.to_string())),
                CtfeValue::Bool(b) => Ok(CtfeValue::String(b.to_string())),
                _ => Ok(val),
            }
        } else {
            Ok(val)
        }
    }

    // ========================================================================
    // Match Evaluation
    // ========================================================================

    fn eval_match(&mut self, value: &CtfeValue, arms: &[ThirArm]) -> Result<CtfeValue, CtfeError> {
        for arm in arms {
            self.push_frame();
            let matched = self.match_pattern(&arm.pattern, value)?;

            if matched {
                // Evaluate guard if present
                if let Some(guard) = &arm.guard {
                    let guard_val = self.eval(guard);
                    match guard_val {
                        Ok(v) => {
                            if !v.as_bool()? {
                                self.pop_frame();
                                continue;
                            }
                        }
                        Err(e) => {
                            self.pop_frame();
                            return Err(e);
                        }
                    }
                }

                // Evaluate body
                let result = self.eval(&arm.body);
                self.pop_frame();
                return result;
            }

            self.pop_frame();
        }

        Err(CtfeError::NonExhaustiveMatch)
    }

    /// Try to match a pattern against a value.
    /// Binds variables in the current stack frame on success.
    fn match_pattern(
        &mut self,
        pattern: &ThirPattern,
        value: &CtfeValue,
    ) -> Result<bool, CtfeError> {
        self.tick()?;

        match &pattern.kind {
            ThirPatternKind::Wildcard => Ok(true),

            ThirPatternKind::Ident(name, _) => {
                self.frame_mut().locals.insert(name.clone(), value.clone());
                Ok(true)
            }

            ThirPatternKind::Literal(lit) => {
                let lit_val = literal_to_value(lit);
                Ok(lit_val.equals(value))
            }

            ThirPatternKind::Condition(expr) => {
                // Store the value as a temporary variable for the condition
                let cond_result = self.eval(expr)?;
                cond_result.as_bool()
            }

            ThirPatternKind::Tuple(patterns) => {
                if let CtfeValue::Tuple(elements) = value {
                    if patterns.len() != elements.len() {
                        return Ok(false);
                    }
                    for (pat, elem) in patterns.iter().zip(elements.iter()) {
                        if !self.match_pattern(pat, elem)? {
                            return Ok(false);
                        }
                    }
                    return Ok(true);
                }
                Ok(false)
            }

            ThirPatternKind::Array(patterns) => {
                if let CtfeValue::Array(elements) = value {
                    if patterns.len() != elements.len() {
                        return Ok(false);
                    }
                    for (pat, elem) in patterns.iter().zip(elements.iter()) {
                        if !self.match_pattern(pat, elem)? {
                            return Ok(false);
                        }
                    }
                    return Ok(true);
                }
                Ok(false)
            }

            ThirPatternKind::Struct { name, fields } => {
                if let CtfeValue::Struct {
                    name: struct_name,
                    fields: struct_fields,
                } = value
                {
                    if struct_name != name {
                        return Ok(false);
                    }
                    for (field_name, field_pattern) in fields {
                        if let Some(field_val) = struct_fields.get(field_name) {
                            if !self.match_pattern(field_pattern, field_val)? {
                                return Ok(false);
                            }
                        } else {
                            return Ok(false);
                        }
                    }
                    return Ok(true);
                }
                Ok(false)
            }

            ThirPatternKind::Enum {
                name,
                variant,
                payload,
            } => {
                if let CtfeValue::Enum {
                    name: enum_name,
                    variant: enum_variant,
                    payload: enum_payload,
                } = value
                {
                    if enum_name != name || enum_variant != variant {
                        return Ok(false);
                    }
                    // Match payload if present
                    if let Some(payload_pattern) = payload {
                        if let Some(payload_vals) = enum_payload {
                            if payload_vals.len() == 1 {
                                return self.match_pattern(payload_pattern, &payload_vals[0]);
                            }
                        }
                        return Ok(false);
                    }
                    return Ok(true);
                }
                Ok(false)
            }

            ThirPatternKind::Rest(_) => {
                // Rest patterns are not needed for const evaluation
                Ok(false)
            }
        }
    }

    // ========================================================================
    // Statement Evaluation
    // ========================================================================

    /// Evaluate a sequence of statements.
    /// Returns Ok(()) on normal completion.
    /// Returns Err(ReturnSignal/BreakSignal/ContinueSignal) on control flow.
    fn eval_block_stmts(&mut self, stmts: &[ThirStmt]) -> Result<(), CtfeError> {
        for stmt in stmts {
            self.eval_stmt(stmt)?;
        }
        Ok(())
    }

    /// Evaluate a single statement.
    fn eval_stmt(&mut self, stmt: &ThirStmt) -> Result<Option<CtfeValue>, CtfeError> {
        self.tick()?;

        if let Some(reason) = is_forbidden_stmt(stmt) {
            return Err(CtfeError::ForbiddenOperation(reason));
        }

        match &stmt.kind {
            // === Variable Declaration ===
            ThirStmtKind::Let { name, value, .. } => {
                let val = self.eval(value)?;
                self.frame_mut().locals.insert(name.clone(), val);
                Ok(None)
            }

            ThirStmtKind::Const { name, value, .. } => {
                let val = self.eval(value)?;
                self.frame_mut().locals.insert(name.clone(), val);
                Ok(None)
            }

            // === Tuple Destructuring ===
            ThirStmtKind::TupleLet { names, value, .. } => {
                let val = self.eval(value)?;
                if let CtfeValue::Tuple(elements) = val {
                    for (name, elem) in names.iter().zip(elements.into_iter()) {
                        self.frame_mut().locals.insert(name.clone(), elem);
                    }
                }
                Ok(None)
            }

            // === Expression Statement ===
            ThirStmtKind::Expr(expr) => {
                let val = self.eval(expr)?;
                Ok(Some(val))
            }

            // === Assignment ===
            ThirStmtKind::Assign { target, value } => {
                let val = self.eval(value)?;
                self.eval_assign(target, val)?;
                Ok(None)
            }

            // === Return ===
            ThirStmtKind::Return(value) => {
                let val = match value {
                    Some(expr) => self.eval(expr)?,
                    None => CtfeValue::Null,
                };
                Err(CtfeError::ReturnSignal(val))
            }

            // === Break ===
            ThirStmtKind::Break(value) => {
                let val = match value {
                    Some(expr) => Some(self.eval(expr)?),
                    None => None,
                };
                Err(CtfeError::BreakSignal(val))
            }

            // === Continue ===
            ThirStmtKind::Continue => Err(CtfeError::ContinueSignal),

            // === While Loop ===
            ThirStmtKind::While {
                cond,
                body,
                increment,
            } => {
                // Reject `while true` explicitly
                if let ThirExprKind::Literal(ThirLiteral::Bool(true)) = &cond.kind {
                    return Err(CtfeError::ForbiddenOperation(
                        "infinite loop (while true) is not allowed in const context".to_string(),
                    ));
                }

                loop {
                    let cond_val = self.eval(cond)?;
                    if !cond_val.as_bool()? {
                        break;
                    }

                    let mut should_continue = false;
                    for stmt in body {
                        match self.eval_stmt(stmt) {
                            Ok(_) => {}
                            Err(CtfeError::BreakSignal(val)) => {
                                return Ok(val);
                            }
                            Err(CtfeError::ContinueSignal) => {
                                should_continue = true;
                                break;
                            }
                            Err(e) => return Err(e),
                        }
                    }

                    for stmt in increment {
                        self.eval_stmt(stmt)?;
                    }

                    if should_continue {
                        continue;
                    }
                }
                Ok(None)
            }

            // === Infinite Loop (for-loop desugared to while, this is raw `loop`) ===
            ThirStmtKind::Loop { .. } => Err(CtfeError::ForbiddenOperation(
                "infinite loop is not allowed in const context".to_string(),
            )),

            // === Go (forbidden) ===
            ThirStmtKind::Go { .. } => Err(CtfeError::ForbiddenOperation(
                "go blocks are not allowed in const context".to_string(),
            )),

            // === Scope ===
            ThirStmtKind::Scope { stmts } => {
                self.push_frame();
                let result = self.eval_block_stmts(stmts);
                self.pop_frame();
                match result {
                    Ok(()) => Ok(None),
                    Err(e) => Err(e),
                }
            }

            // === Drop (no-op in CTFE) ===
            ThirStmtKind::Drop { .. } => Ok(None),

            // === Manual Error Extract ===
            ThirStmtKind::ManualErrorExtract {
                ok_names,
                error_name,
                expr,
            } => {
                let val = self.eval(expr)?;
                if let CtfeValue::Enum {
                    name,
                    variant,
                    payload,
                } = val
                {
                    if name == "Result" && variant == "Ok" {
                        if let Some(values) = payload {
                            for (name, value) in ok_names.iter().zip(values.into_iter()) {
                                self.frame_mut().locals.insert(name.clone(), value);
                            }
                        }
                    } else if name == "Result" && variant == "Err" {
                        if let Some(values) = payload {
                            if let Some(err_val) = values.into_iter().next() {
                                self.frame_mut().locals.insert(error_name.clone(), err_val);
                            }
                        }
                    }
                }
                Ok(None)
            }
        }
    }

    /// Evaluate an assignment target (variable, field, or index).
    fn eval_assign(&mut self, target: &ThirExpr, value: CtfeValue) -> Result<(), CtfeError> {
        match &target.kind {
            // Variable assignment
            ThirExprKind::Var(name) => {
                self.frame_mut().locals.insert(name.clone(), value);
                Ok(())
            }

            // Array element assignment: arr[idx] = val
            ThirExprKind::Index { object, index } => {
                let obj_name = match &object.kind {
                    ThirExprKind::Var(name) => name.clone(),
                    _ => {
                        return Err(CtfeError::NotImplemented(
                            "complex assignment target".to_string(),
                        ))
                    }
                };

                let idx = self.eval(index)?.as_int()?;
                let mut arr = self.lookup(&obj_name)?;

                match &mut arr {
                    CtfeValue::Array(elements) => {
                        let len = elements.len() as i64;
                        if idx < 0 || idx >= len {
                            return Err(CtfeError::TypeMismatch(format!(
                                "array index {} out of bounds (len {})",
                                idx, len
                            )));
                        }
                        elements[idx as usize] = value;
                    }
                    _ => {
                        return Err(CtfeError::TypeMismatch(format!(
                            "cannot index {:?} for assignment",
                            arr
                        )))
                    }
                }

                self.frame_mut().locals.insert(obj_name, arr);
                Ok(())
            }

            // Field assignment: obj.field = val
            ThirExprKind::FieldAccess { object, field, .. } => {
                let obj_name = match &object.kind {
                    ThirExprKind::Var(name) => name.clone(),
                    _ => {
                        return Err(CtfeError::NotImplemented(
                            "complex assignment target".to_string(),
                        ))
                    }
                };

                let mut obj = self.lookup(&obj_name)?;
                match &mut obj {
                    CtfeValue::Struct { fields, .. } => {
                        fields.insert(field.clone(), value);
                    }
                    _ => {
                        return Err(CtfeError::TypeMismatch(format!(
                            "cannot set field on {:?}",
                            obj
                        )))
                    }
                }

                self.frame_mut().locals.insert(obj_name, obj);
                Ok(())
            }

            _ => Err(CtfeError::NotImplemented(format!(
                "assignment target not supported: {:?}",
                target.kind
            ))),
        }
    }

    // ========================================================================
    // Stack Management
    // ========================================================================

    fn push_frame(&mut self) {
        self.stack.push(StackFrame::new());
    }

    fn pop_frame(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }
}

impl Default for CtfeInterpreter {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Convert a THIR literal to a CTFE value.
fn literal_to_value(lit: &ThirLiteral) -> CtfeValue {
    match lit {
        ThirLiteral::Int(i) => CtfeValue::Int(*i),
        ThirLiteral::Float(f) => CtfeValue::Float(*f),
        ThirLiteral::String(s) => CtfeValue::String(s.clone()),
        ThirLiteral::Bool(b) => CtfeValue::Bool(*b),
        ThirLiteral::Null => CtfeValue::Null,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use doo_core::types::builtin;
    use doo_core::Span;

    fn make_expr(kind: ThirExprKind) -> ThirExpr {
        ThirExpr {
            kind,
            ty: builtin::ANY,
            span: Span::dummy(),
        }
    }

    fn int_expr(i: i64) -> ThirExpr {
        make_expr(ThirExprKind::Literal(ThirLiteral::Int(i)))
    }

    fn bool_expr(b: bool) -> ThirExpr {
        make_expr(ThirExprKind::Literal(ThirLiteral::Bool(b)))
    }

    fn var_expr(name: &str) -> ThirExpr {
        make_expr(ThirExprKind::Var(name.to_string()))
    }

    #[test]
    fn test_eval_int_literal() {
        let mut interp = CtfeInterpreter::new();
        let expr = int_expr(42);
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Int(42));
    }

    #[test]
    fn test_eval_float_literal() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Literal(ThirLiteral::Float(3.14)));
        let result = interp.eval(&expr).unwrap();
        assert!(matches!(result, CtfeValue::Float(f) if (f - 3.14).abs() < 0.001));
    }

    #[test]
    fn test_eval_bool_literal() {
        let mut interp = CtfeInterpreter::new();
        let expr = bool_expr(true);
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Bool(true));
    }

    #[test]
    fn test_eval_string_literal() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Literal(ThirLiteral::String(
            "hello".to_string(),
        )));
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::String("hello".to_string()));
    }

    #[test]
    fn test_eval_add() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Binary {
            op: ThirBinOp::Add,
            lhs: Box::new(int_expr(3)),
            rhs: Box::new(int_expr(4)),
        });
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Int(7));
    }

    #[test]
    fn test_eval_sub() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Binary {
            op: ThirBinOp::Sub,
            lhs: Box::new(int_expr(10)),
            rhs: Box::new(int_expr(3)),
        });
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Int(7));
    }

    #[test]
    fn test_eval_mul() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Binary {
            op: ThirBinOp::Mul,
            lhs: Box::new(int_expr(6)),
            rhs: Box::new(int_expr(7)),
        });
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Int(42));
    }

    #[test]
    fn test_eval_div_zero() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Binary {
            op: ThirBinOp::Div,
            lhs: Box::new(int_expr(10)),
            rhs: Box::new(int_expr(0)),
        });
        let result = interp.eval(&expr);
        assert!(matches!(result, Err(CtfeError::DivisionByZero)));
    }

    #[test]
    fn test_eval_comparison() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Binary {
            op: ThirBinOp::Lt,
            lhs: Box::new(int_expr(3)),
            rhs: Box::new(int_expr(5)),
        });
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Bool(true));
    }

    #[test]
    fn test_eval_and() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Binary {
            op: ThirBinOp::And,
            lhs: Box::new(bool_expr(true)),
            rhs: Box::new(bool_expr(false)),
        });
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Bool(false));
    }

    #[test]
    fn test_eval_neg() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Unary {
            op: ThirUnOp::Neg,
            expr: Box::new(int_expr(5)),
        });
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Int(-5));
    }

    #[test]
    fn test_eval_not() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Unary {
            op: ThirUnOp::Not,
            expr: Box::new(bool_expr(true)),
        });
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Bool(false));
    }

    #[test]
    fn test_eval_if_true() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::If {
            cond: Box::new(bool_expr(true)),
            then: Box::new(int_expr(42)),
            else_: Some(Box::new(int_expr(0))),
        });
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Int(42));
    }

    #[test]
    fn test_eval_if_false() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::If {
            cond: Box::new(bool_expr(false)),
            then: Box::new(int_expr(42)),
            else_: Some(Box::new(int_expr(0))),
        });
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Int(0));
    }

    #[test]
    fn test_eval_array_literal() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::ArrayLiteral(vec![
            int_expr(1),
            int_expr(2),
            int_expr(3),
        ]));
        let result = interp.eval(&expr).unwrap();
        assert!(matches!(result, CtfeValue::Array(ref a) if a.len() == 3));
    }

    #[test]
    fn test_eval_array_index() {
        let mut interp = CtfeInterpreter::new();
        let arr_expr = make_expr(ThirExprKind::ArrayLiteral(vec![
            int_expr(10),
            int_expr(20),
            int_expr(30),
        ]));
        let idx_expr = int_expr(1);
        let expr = make_expr(ThirExprKind::Index {
            object: Box::new(arr_expr),
            index: Box::new(idx_expr),
        });
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Int(20));
    }

    #[test]
    fn test_eval_block_with_let() {
        let mut interp = CtfeInterpreter::new();

        let let_stmt = doo_thir::ThirStmt {
            kind: ThirStmtKind::Let {
                name: "x".to_string(),
                ty: builtin::INT,
                value: int_expr(42),
                mutable: false,
            },
            span: Span::dummy(),
        };

        let expr = make_expr(ThirExprKind::Block(
            vec![let_stmt],
            Some(Box::new(var_expr("x"))),
        ));

        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Int(42));
    }

    #[test]
    fn test_eval_while_loop() {
        let mut interp = CtfeInterpreter::new();

        // let mut i = 0
        // while i < 3 { i = i + 1 }
        // i

        let init_i = doo_thir::ThirStmt {
            kind: ThirStmtKind::Let {
                name: "i".to_string(),
                ty: builtin::INT,
                value: int_expr(0),
                mutable: true,
            },
            span: Span::dummy(),
        };

        let cond = make_expr(ThirExprKind::Binary {
            op: ThirBinOp::Lt,
            lhs: Box::new(var_expr("i")),
            rhs: Box::new(int_expr(3)),
        });

        let assign_i = doo_thir::ThirStmt {
            kind: ThirStmtKind::Assign {
                target: var_expr("i"),
                value: make_expr(ThirExprKind::Binary {
                    op: ThirBinOp::Add,
                    lhs: Box::new(var_expr("i")),
                    rhs: Box::new(int_expr(1)),
                }),
            },
            span: Span::dummy(),
        };

        let while_stmt = doo_thir::ThirStmt {
            kind: ThirStmtKind::While {
                cond,
                body: vec![assign_i],
                increment: vec![],
            },
            span: Span::dummy(),
        };

        let expr = make_expr(ThirExprKind::Block(
            vec![init_i, while_stmt],
            Some(Box::new(var_expr("i"))),
        ));

        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::Int(3));
    }

    #[test]
    fn test_eval_while_true_rejected() {
        let mut interp = CtfeInterpreter::new();

        let while_stmt = doo_thir::ThirStmt {
            kind: ThirStmtKind::While {
                cond: bool_expr(true),
                body: vec![],
                increment: vec![],
            },
            span: Span::dummy(),
        };

        let expr = make_expr(ThirExprKind::Block(
            vec![while_stmt],
            Some(Box::new(int_expr(0))),
        ));

        let result = interp.eval(&expr);
        assert!(matches!(result, Err(CtfeError::ForbiddenOperation(_))));
    }

    #[test]
    fn test_step_limit() {
        let mut interp = CtfeInterpreter::with_step_limit(5);

        // This expression has more than 5 sub-expressions
        let expr = make_expr(ThirExprKind::Binary {
            op: ThirBinOp::Add,
            lhs: Box::new(make_expr(ThirExprKind::Binary {
                op: ThirBinOp::Add,
                lhs: Box::new(make_expr(ThirExprKind::Binary {
                    op: ThirBinOp::Add,
                    lhs: Box::new(int_expr(1)),
                    rhs: Box::new(int_expr(2)),
                })),
                rhs: Box::new(int_expr(3)),
            })),
            rhs: Box::new(int_expr(4)),
        });

        let result = interp.eval(&expr);
        assert!(matches!(result, Err(CtfeError::StepLimitExceeded)));
    }

    #[test]
    fn test_eval_struct_literal() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::StructLiteral {
            name: "Point".to_string(),
            fields: vec![
                ("x".to_string(), int_expr(1)),
                ("y".to_string(), int_expr(2)),
            ],
        });
        let result = interp.eval(&expr).unwrap();
        match result {
            CtfeValue::Struct { name, fields } => {
                assert_eq!(name, "Point");
                assert_eq!(fields.get("x"), Some(&CtfeValue::Int(1)));
                assert_eq!(fields.get("y"), Some(&CtfeValue::Int(2)));
            }
            _ => panic!("expected Struct"),
        }
    }

    #[test]
    fn test_eval_forbidden_call() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Call {
            func: Box::new(var_expr("some_fn")),
            args: vec![],
        });
        let result = interp.eval(&expr);
        assert!(matches!(result, Err(CtfeError::ForbiddenOperation(_))));
    }

    #[test]
    fn test_eval_forbidden_spawn() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Spawn(Box::new(int_expr(0))));
        let result = interp.eval(&expr);
        assert!(matches!(result, Err(CtfeError::ForbiddenOperation(_))));
    }

    #[test]
    fn test_eval_cast_int_to_float() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Cast {
            value: Box::new(int_expr(42)),
            to_type: builtin::FLOAT,
        });
        let result = interp.eval(&expr).unwrap();
        assert!(matches!(result, CtfeValue::Float(f) if f == 42.0));
    }

    #[test]
    fn test_eval_string_concat() {
        let mut interp = CtfeInterpreter::new();
        let expr = make_expr(ThirExprKind::Binary {
            op: ThirBinOp::Add,
            lhs: Box::new(make_expr(ThirExprKind::Literal(ThirLiteral::String(
                "hello".to_string(),
            )))),
            rhs: Box::new(make_expr(ThirExprKind::Literal(ThirLiteral::String(
                " world".to_string(),
            )))),
        });
        let result = interp.eval(&expr).unwrap();
        assert_eq!(result, CtfeValue::String("hello world".to_string()));
    }

    #[test]
    fn test_eval_const_api() {
        let expr = int_expr(42);
        let result = crate::eval_const(&expr).unwrap();
        assert_eq!(result, CtfeValue::Int(42));
    }
}
