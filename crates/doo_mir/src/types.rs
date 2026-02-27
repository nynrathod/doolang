//! # MIR Types - New Design
//!
//! Mid-level Intermediate Representation for Doo.
//!
//! ## Design Philosophy
//!
//! This MIR follows the NEW compiler design:
//! - **Pure Ownership**: Move/Copy/Clone/Drop - NO reference counting
//! - **TypeId-based**: All types referenced by ID, not inlined
//! - **Minimal IR**: Only essential instructions, LLVM handles optimization
//! - **Single Source of Truth**: Types come from doo_core::TypeRegistry
//!
//! ## Ownership Model
//!
//! | Situation | MIR Instruction |
//! |-----------|-----------------|
//! | Last use of variable | Move (zero-cost) |
//! | Primitive reused | Copy (bitwise) |
//! | Non-primitive reused | Clone (deep copy) |
//! | Scope exit | Drop (cleanup) |
//!
//! **NO IncRef/DecRef** - the compiler decides ownership at analysis time.

use crate::sym::{resolve, Sym};
use doo_core::doo_debug;
use doo_core::types::TypeId;
use std::collections::HashMap;

// ============================================================================
// Type System Integration
// ============================================================================

/// Span for source locations (will be replaced with doo_core::Span)
#[derive(Debug, Clone, Copy, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

// ============================================================================
// MIR Program
// ============================================================================

/// A complete MIR program.
/// Contains all functions, types, and metadata needed for codegen.
#[derive(Debug, Clone)]
pub struct MirProgram {
    /// All functions in the program
    pub functions: Vec<MirFunction>,
    /// Global constants and initializers
    pub globals: Vec<MirGlobal>,
    /// Struct metadata: name -> field definitions
    pub structs: HashMap<Sym, StructDef>,
    /// Enum metadata: name -> variant definitions
    pub enums: HashMap<Sym, EnumDef>,
    /// Entry point function name (usually "main")
    pub entry_point: Option<Sym>,
}

/// Global constant or variable
#[derive(Debug, Clone)]
pub struct MirGlobal {
    pub name: Sym,
    pub type_id: TypeId,
    pub value: Option<MirConst>,
}

/// Struct definition metadata
#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: Sym,
    pub fields: Vec<FieldDef>,
    pub decorators: Vec<Decorator>,
}

/// Field definition with decorators
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: Sym,
    pub type_id: TypeId,
    pub optional: bool,
    pub decorators: Vec<Decorator>,
    pub default_value: Option<MirConst>,
}

/// Enum definition metadata
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: Sym,
    pub variants: Vec<VariantDef>,
}

/// Enum variant definition
#[derive(Debug, Clone)]
pub struct VariantDef {
    pub name: Sym,
    pub index: u32,
    pub payload_type: Option<TypeId>,
}

/// Decorator on struct/field
#[derive(Debug, Clone)]
pub struct Decorator {
    pub name: Sym,
    pub args: Vec<String>,
}

impl MirProgram {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            globals: Vec::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            entry_point: None,
        }
    }

    /// Check if any function in the program uses async features.
    /// Used by codegen to emit `doo_runtime_init()` in `main()`.
    /// Note: Process FFI async detection is handled by the linker (compile.rs)
    /// which links doo_ffi_runtime when doo_ffi_process is present.
    pub fn has_async_features(&self) -> bool {
        self.functions.iter().any(|f| {
            f.is_async
                || f.blocks.iter().any(|b| {
                    b.instructions.iter().any(|i| {
                        matches!(
                            &i.kind,
                            MirInstrKind::Sleep { .. }
                                | MirInstrKind::Await { .. }
                                | MirInstrKind::Spawn { .. }
                                | MirInstrKind::ScopeCreate { .. }
                                | MirInstrKind::ScopeSpawn { .. }
                                | MirInstrKind::ScopeWait { .. }
                        )
                    })
                })
        })
    }

    /// Validate all functions
    pub fn validate(&self) -> Result<(), MirError> {
        for func in &self.functions {
            if let Err(e) = func.validate() {
                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                    doo_debug!(
                        "MIR",
                        "Validation failed in function '{}': {}",
                        resolve(func.name),
                        e
                    );
                    doo_debug!(
                        "MIR",
                        "  params: {:?}",
                        func.params.iter().map(|p| &p.name).collect::<Vec<_>>()
                    );
                    doo_debug!(
                        "MIR",
                        "  locals: {:?}",
                        func.locals.iter().map(|l| &l.name).collect::<Vec<_>>()
                    );
                }
                return Err(e);
            }
        }
        Ok(())
    }
}

impl Default for MirProgram {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// MIR Function
// ============================================================================

/// A function in MIR form.
#[derive(Debug, Clone)]
pub struct MirFunction {
    /// Function name
    pub name: Sym,
    /// Parameter definitions
    pub params: Vec<ParamDef>,
    /// Return type (None = void)
    pub return_type: Option<TypeId>,
    /// Error type for fallible functions
    pub error_type: Option<TypeId>,
    /// Basic blocks (CFG)
    pub blocks: Vec<MirBlock>,
    /// Local variable definitions
    pub locals: Vec<LocalDef>,
    /// FFI linkage info
    pub ffi: Option<FfiLinkage>,
    /// Source span
    pub span: Span,
    /// Whether this is a closure function (requires env param and i64 calling convention)
    pub is_closure: bool,
    /// Whether this closure captures by value (true for fire-and-forget Spawn/go{})
    /// vs by reference (false for ScopeSpawn where parent waits)
    pub captures_by_value: bool,
    /// Whether this is an async function
    pub is_async: bool,
    /// Captured variable names from outer scope (for spawn/closure env unpacking)
    pub captures: Vec<Sym>,
}

/// Parameter definition
#[derive(Debug, Clone)]
pub struct ParamDef {
    pub name: Sym,
    pub type_id: TypeId,
}

/// Local variable definition
#[derive(Debug, Clone)]
pub struct LocalDef {
    pub name: Sym,
    pub type_id: TypeId,
    pub mutable: bool,
}

/// FFI linkage information.
///
/// Carries the full type signature from the Doo declaration so codegen can
/// generate correct LLVM types **without** a hardcoded symbol→signature table.
/// This is the key enabler for package-readiness: any third-party `@extern`
/// function automatically gets the right LLVM signature because the types
/// flow through MIR from the Doo source.
#[derive(Debug, Clone)]
pub struct FfiLinkage {
    pub library: Sym,
    pub symbol: Option<Sym>,
    /// Parameter types from the Doo declaration (already resolved in HIR/analysis).
    /// Used by codegen to emit correct LLVM parameter types instead of defaulting to `ptr`.
    pub param_types: Vec<TypeId>,
    /// Return type from the Doo declaration (None = void).
    /// Used by codegen to emit correct LLVM return type instead of defaulting to `ptr`.
    pub return_type: Option<TypeId>,
    /// Whether this function returns a Result (has error_type).
    /// When true, FFI returns `*mut SimpleResult` (pointer) on the ABI boundary.
    pub is_result: bool,
}

impl MirFunction {
    pub fn new(name: Sym) -> Self {
        Self {
            name,
            params: Vec::new(),
            return_type: None,
            error_type: None,
            blocks: Vec::new(),
            locals: Vec::new(),
            ffi: None,
            span: Span::default(),
            is_closure: false,
            captures_by_value: false,
            is_async: false,
            captures: Vec::new(),
        }
    }

    /// Validate function's CFG and value definitions
    pub fn validate(&self) -> Result<(), MirError> {
        // Collect all defined values (Sym is Copy, so HashSet<Sym> is cheap)
        let mut defined: std::collections::HashSet<Sym> = std::collections::HashSet::new();

        // Parameters are pre-defined
        for param in &self.params {
            defined.insert(param.name);
        }

        // Locals are pre-defined (including variables defined by ManualErrorExtract, etc.)
        for local in &self.locals {
            defined.insert(local.name);
        }

        // Check each block
        for block in &self.blocks {
            for instr in &block.instructions {
                // Check operands are defined
                for operand in instr.operands() {
                    if let MirOperand::Local(name) | MirOperand::Temp(name) = operand {
                        if !defined.contains(name) && !resolve(*name).starts_with('@') {
                            return Err(MirError::UndefinedValue {
                                name: *name,
                                block: block.label,
                            });
                        }
                    }
                }

                // Check Borrow and Drop sources which use Sym instead of MirOperand
                // These are special cases that operands() doesn't return
                match &instr.kind {
                    MirInstrKind::Borrow { src, .. } => {
                        if !defined.contains(src) && !resolve(*src).starts_with('@') {
                            return Err(MirError::UndefinedValue {
                                name: *src,
                                block: block.label,
                            });
                        }
                    }
                    MirInstrKind::Drop { value } => {
                        if !defined.contains(value) && !resolve(*value).starts_with('@') {
                            return Err(MirError::UndefinedValue {
                                name: *value,
                                block: block.label,
                            });
                        }
                    }
                    _ => {}
                }

                // Add defined value
                if let Some(dest) = instr.destination() {
                    defined.insert(*dest);
                }
            }
        }

        Ok(())
    }
}

// ============================================================================
// MIR Block
// ============================================================================

/// A basic block in the CFG.
#[derive(Debug, Clone)]
pub struct MirBlock {
    /// Unique label
    pub label: Sym,
    /// Instructions (no terminators)
    pub instructions: Vec<MirInstr>,
    /// Block terminator
    pub terminator: MirTerminator,
}

impl MirBlock {
    pub fn new(label: Sym) -> Self {
        Self {
            label,
            instructions: Vec::new(),
            terminator: MirTerminator::Unreachable,
        }
    }
}

// ============================================================================
// MIR Terminator
// ============================================================================

/// Block terminators - control flow instructions.
#[derive(Debug, Clone)]
pub enum MirTerminator {
    /// Return from function
    Return { values: Vec<MirOperand> },
    /// Unconditional branch
    Goto { target: Sym },
    /// Conditional branch
    Branch {
        cond: MirOperand,
        then_block: Sym,
        else_block: Sym,
    },
    /// Multi-way branch (switch/match)
    Switch {
        value: MirOperand,
        cases: Vec<(i64, Sym)>,
        default: Sym,
    },
    /// Unreachable (for dead code)
    Unreachable,
}

// ============================================================================
// MIR Operand
// ============================================================================

/// An operand in a MIR instruction.
#[derive(Debug, Clone)]
pub enum MirOperand {
    /// Constant value
    Const(MirConst),
    /// Local variable or parameter
    Local(Sym),
    /// Temporary value (SSA)
    Temp(Sym),
    /// Global reference
    Global(Sym),
    /// Function reference (for passing functions as values to FFI)
    FuncRef(Sym),
}

/// Constant values in MIR.
#[derive(Debug, Clone, PartialEq)]
pub enum MirConst {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Nil,
}

// ============================================================================
// MIR Instruction - NEW DESIGN (Pure Ownership)
// ============================================================================

/// MIR instruction with pure ownership semantics.
///
/// ## Key Differences from Legacy
/// - NO IncRef/DecRef - ownership is compile-time
/// - Uses MirOperand for all values
/// - Explicit Move/Copy/Clone/Drop for memory
/// - Minimal instruction set - LLVM optimizes
#[derive(Debug, Clone)]
pub enum MirInstrKind {
    // ========================================================================
    // Ownership Operations (NEW - replaces RC)
    // ========================================================================
    /// Move value to destination (zero-cost, transfers ownership)
    Move { dest: Sym, src: MirOperand },

    /// Copy value (for primitives or explicit Copy types)
    Copy { dest: Sym, src: MirOperand },

    /// Clone value (deep copy for non-Copy types)
    Clone { dest: Sym, src: MirOperand },

    /// Drop value (cleanup, inserted by analysis)
    Drop { value: Sym },

    /// Borrow value (create reference for function call)
    Borrow { dest: Sym, src: Sym, mutable: bool },

    // ========================================================================
    // Assignment
    // ========================================================================
    /// Assign constant or operand to local
    Assign { dest: Sym, value: MirOperand },

    // ========================================================================
    // Arithmetic & Logic
    // ========================================================================
    /// Binary operation
    BinaryOp {
        dest: Sym,
        op: BinaryOp,
        lhs: MirOperand,
        rhs: MirOperand,
    },

    /// Unary operation
    UnaryOp {
        dest: Sym,
        op: UnaryOp,
        operand: MirOperand,
    },

    // ========================================================================
    // Collections
    // ========================================================================
    /// Create array
    ArrayCreate {
        dest: Sym,
        elements: Vec<MirOperand>,
        elem_type: TypeId,
    },

    /// Get array element
    ArrayGet {
        dest: Sym,
        array: MirOperand,
        index: MirOperand,
        elem_type: TypeId,
    },

    /// Set array element
    ArraySet {
        array: MirOperand,
        index: MirOperand,
        value: MirOperand,
        elem_type: TypeId,
    },

    /// Get array length
    ArrayLen { dest: Sym, array: MirOperand },

    /// Check if array contains value
    ArrayContains {
        dest: Sym,
        array: MirOperand,
        value: MirOperand,
        elem_type: TypeId,
    },

    /// Append value to array (for spread/mutation)
    ArrayPush {
        array: MirOperand,
        value: MirOperand,
    },

    /// Extend array with another array (for spread)
    ArrayExtend {
        array: MirOperand,
        other: MirOperand,
        elem_type: TypeId,
    },

    /// Slice array
    ArraySlice {
        dest: Sym,
        array: MirOperand,
        start: MirOperand,
        end: MirOperand,
        elem_type: TypeId,
    },

    /// Create map
    MapCreate {
        dest: Sym,
        entries: Vec<(MirOperand, MirOperand)>,
        key_type: TypeId,
        val_type: TypeId,
    },

    /// Get map value
    MapGet {
        dest: Sym,
        map: MirOperand,
        key: MirOperand,
        key_type: TypeId,
        val_type: TypeId,
    },

    /// Set map value
    MapSet {
        map: MirOperand,
        key: MirOperand,
        value: MirOperand,
        key_type: TypeId,
        val_type: TypeId,
    },

    /// Check if map contains key
    MapHas {
        dest: Sym,
        map: MirOperand,
        key: MirOperand,
        key_type: TypeId,
        val_type: TypeId,
    },

    // ========================================================================
    // Structs
    // ========================================================================
    /// Create struct instance
    StructCreate {
        dest: Sym,
        struct_name: Sym,
        fields: Vec<(Sym, MirOperand)>,
    },

    /// Get struct field
    FieldGet {
        dest: Sym,
        object: MirOperand,
        field: Sym,
    },

    /// Set struct field
    FieldSet {
        object: MirOperand,
        field: Sym,
        value: MirOperand,
    },

    // ========================================================================
    // Enums
    // ========================================================================
    /// Create enum variant
    EnumCreate {
        dest: Sym,
        enum_name: Sym,
        variant: Sym,
        payload: Option<MirOperand>,
    },

    /// Get enum discriminant (tag)
    EnumTag { dest: Sym, value: MirOperand },

    /// Get enum discriminant (tag) with enum name for type lookup
    EnumGetTag {
        dest: Sym,
        value: MirOperand,
        enum_name: Sym,
    },

    /// Compare enum tag with expected variant
    EnumTagEquals {
        dest: Sym,
        tag: MirOperand,
        variant_name: Sym,
        enum_name: Sym,
    },

    /// Extract enum payload
    EnumPayload {
        dest: Sym,
        value: MirOperand,
        variant: Sym,
    },

    /// Extract enum payload with index (for ADT pattern matching)
    EnumGetPayload {
        dest: Sym,
        value: MirOperand,
        variant_name: Sym,
        enum_name: Sym,
        index: u32,
    },

    // ========================================================================
    // Tuples
    // ========================================================================
    /// Create tuple
    TupleCreate {
        dest: Sym,
        elements: Vec<MirOperand>,
    },

    /// Extract tuple element
    TupleGet {
        dest: Sym,
        tuple: MirOperand,
        index: usize,
        /// TypeId of the tuple (so codegen can look up element types)
        tuple_type: Option<TypeId>,
    },

    // ========================================================================
    // Calls
    // ========================================================================
    /// Function call
    Call {
        dest: Option<Sym>,
        func: Sym,
        args: Vec<MirOperand>,
    },

    /// Method call
    MethodCall {
        dest: Option<Sym>,
        receiver: MirOperand,
        receiver_type: TypeId,
        method: Sym,
        args: Vec<MirOperand>,
        arg_types: Vec<TypeId>,
        /// Return type of the method call (for JSON.parse and similar)
        return_type: Option<TypeId>,
    },

    /// FFI call
    FfiCall {
        dest: Option<Sym>,
        lib: Sym,
        symbol: Sym,
        args: Vec<MirOperand>,
    },

    // ========================================================================
    // Closures
    // ========================================================================
    /// Create closure
    ClosureCreate {
        dest: Sym,
        func: Sym,
        captures: Vec<MirOperand>,
    },

    /// Call closure
    ClosureCall {
        dest: Option<Sym>,
        closure: MirOperand,
        args: Vec<MirOperand>,
    },

    // ========================================================================
    // Result/Error Handling
    // ========================================================================
    /// Wrap value in Ok
    WrapOk { dest: Sym, value: MirOperand },

    /// Wrap value in Err
    WrapErr { dest: Sym, value: MirOperand },

    /// Check if result is Ok
    IsOk { dest: Sym, value: MirOperand },

    /// Unwrap Ok value (with expected type for proper conversion)
    UnwrapOk {
        dest: Sym,
        value: MirOperand,
        expected_type: Option<TypeId>,
    },

    /// Unwrap Err value
    UnwrapErr { dest: Sym, value: MirOperand },

    /// Extract Ok and Error values from Result (manual error handling)
    /// let a, b, err = expr;
    ManualErrorExtract {
        ok_names: Vec<Sym>, // Names for Ok values (single or tuple)
        error_name: Sym,    // Name for error variable (or "_" to ignore)
        result: MirOperand, // Result to extract from
        ok_type: TypeId,    // Type of the Ok value
        err_type: TypeId,   // Type of the Error value
    },

    // ========================================================================
    // Cast & Conversion
    // ========================================================================
    /// Type cast
    Cast {
        dest: Sym,
        value: MirOperand,
        to_type: TypeId,
    },

    // ========================================================================
    // I/O
    // ========================================================================
    /// Print values
    Print {
        values: Vec<MirOperand>,
        value_types: Vec<TypeId>,
        /// When false, no space separator between values (used for string interpolation)
        separator: bool,
    },

    // ========================================================================
    // Intrinsics
    // ========================================================================
    /// Get runtime type name as string
    TypeOf {
        dest: Sym,
        value: MirOperand,
        value_type: TypeId,
    },

    // ========================================================================
    // Control Flow / Panic
    // ========================================================================
    /// Panic with message (abort program execution)
    Panic { message: MirOperand },

    // ========================================================================
    // Async & Concurrency
    // ========================================================================
    /// Sleep for N milliseconds: `sleep(ms)` — blocking
    Sleep { ms: MirOperand },
    /// Await a task handle: `await handle` → result
    Await { dest: Sym, handle: MirOperand },
    /// Spawn a task: `go { body }` → task handle
    Spawn {
        dest: Sym,
        func: Sym,
        captures: Vec<MirOperand>,
    },
    /// Create an empty scope: `scope {` → scope handle
    ScopeCreate { dest: Sym },
    /// Spawn a task into a scope: `go { body }` inside scope
    ScopeSpawn {
        scope: MirOperand,
        func: Sym,
        captures: Vec<MirOperand>,
    },
    /// Wait for all scope tasks to finish: `}` of scope → result
    ScopeWait { dest: Sym, scope: MirOperand },
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    // Comparison
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // Logical
    And,
    Or,
    // String
    Concat,
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg, // -x
    Not, // !x
}

// ============================================================================
// MIR Instruction Wrapper
// ============================================================================

/// Complete MIR instruction with span.
#[derive(Debug, Clone)]
pub struct MirInstr {
    pub kind: MirInstrKind,
    pub span: Span,
}

impl MirInstr {
    pub fn new(kind: MirInstrKind) -> Self {
        Self {
            kind,
            span: Span::default(),
        }
    }

    /// Get operands used by this instruction
    pub fn operands(&self) -> Vec<&MirOperand> {
        match &self.kind {
            MirInstrKind::Move { src, .. } => vec![src],
            MirInstrKind::Copy { src, .. } => vec![src],
            MirInstrKind::Clone { src, .. } => vec![src],
            MirInstrKind::Assign { value, .. } => vec![value],
            MirInstrKind::BinaryOp { lhs, rhs, .. } => vec![lhs, rhs],
            MirInstrKind::UnaryOp { operand, .. } => vec![operand],
            MirInstrKind::ArrayCreate { elements, .. } => elements.iter().collect(),
            MirInstrKind::ArrayGet { array, index, .. } => vec![array, index],
            MirInstrKind::ArraySet {
                array,
                index,
                value,
                ..
            } => vec![array, index, value],
            MirInstrKind::ArrayLen { array, .. } => vec![array],
            MirInstrKind::ArrayContains { array, value, .. } => vec![array, value],
            MirInstrKind::ArrayPush { array, value } => vec![array, value],
            MirInstrKind::ArrayExtend { array, other, .. } => vec![array, other],
            MirInstrKind::ArraySlice {
                array, start, end, ..
            } => vec![array, start, end],

            MirInstrKind::MapCreate { entries, .. } => {
                entries.iter().flat_map(|(k, v)| vec![k, v]).collect()
            }
            MirInstrKind::MapGet { map, key, .. } => vec![map, key],
            MirInstrKind::MapSet {
                map, key, value, ..
            } => vec![map, key, value],
            MirInstrKind::MapHas { map, key, .. } => vec![map, key],
            MirInstrKind::StructCreate { fields, .. } => fields.iter().map(|(_, v)| v).collect(),
            MirInstrKind::FieldGet { object, .. } => vec![object],
            MirInstrKind::FieldSet { object, value, .. } => vec![object, value],
            MirInstrKind::EnumCreate {
                payload: Some(p), ..
            } => vec![p],
            MirInstrKind::EnumTag { value, .. } => vec![value],
            MirInstrKind::EnumPayload { value, .. } => vec![value],
            MirInstrKind::TupleCreate { elements, .. } => elements.iter().collect(),
            MirInstrKind::TupleGet { tuple, .. } => vec![tuple],
            MirInstrKind::Call { args, .. } => args.iter().collect(),
            MirInstrKind::MethodCall {
                receiver,
                receiver_type: _,
                method: _,
                args,
                ..
            } => std::iter::once(receiver).chain(args.iter()).collect(),
            MirInstrKind::FfiCall { args, .. } => args.iter().collect(),
            MirInstrKind::ClosureCreate { captures, .. } => captures.iter().collect(),
            MirInstrKind::ClosureCall { closure, args, .. } => {
                std::iter::once(closure).chain(args.iter()).collect()
            }
            MirInstrKind::WrapOk { value, .. } => vec![value],
            MirInstrKind::WrapErr { value, .. } => vec![value],
            MirInstrKind::IsOk { value, .. } => vec![value],
            MirInstrKind::UnwrapOk { value, .. } => vec![value],
            MirInstrKind::UnwrapErr { value, .. } => vec![value],
            MirInstrKind::Cast { value, .. } => vec![value],
            MirInstrKind::Print {
                values,
                value_types: _,
                separator: _,
            } => values.iter().collect(),
            MirInstrKind::TypeOf { value, .. } => vec![value],
            MirInstrKind::Panic { message } => vec![message],
            MirInstrKind::Sleep { ms } => vec![ms],
            MirInstrKind::Await { handle, .. } => vec![handle],
            MirInstrKind::ScopeWait { scope, .. } => vec![scope],
            MirInstrKind::ScopeSpawn {
                scope, captures, ..
            } => {
                let mut ops = vec![scope];
                ops.extend(captures.iter());
                ops
            }
            MirInstrKind::Spawn { captures, .. } => captures.iter().collect(),
            _ => vec![],
        }
    }

    /// Get destination (defined value) if any
    pub fn destination(&self) -> Option<&Sym> {
        match &self.kind {
            MirInstrKind::Move { dest, .. }
            | MirInstrKind::Copy { dest, .. }
            | MirInstrKind::Clone { dest, .. }
            | MirInstrKind::Borrow { dest, .. }
            | MirInstrKind::Assign { dest, .. }
            | MirInstrKind::BinaryOp { dest, .. }
            | MirInstrKind::UnaryOp { dest, .. }
            | MirInstrKind::ArrayCreate { dest, .. }
            | MirInstrKind::ArrayGet { dest, .. }
            | MirInstrKind::ArrayLen { dest, .. }
            | MirInstrKind::ArrayContains { dest, .. }
            | MirInstrKind::ArraySlice { dest, .. }
            | MirInstrKind::MapCreate { dest, .. }
            | MirInstrKind::MapGet { dest, .. }
            | MirInstrKind::MapHas { dest, .. }
            | MirInstrKind::StructCreate { dest, .. }
            | MirInstrKind::FieldGet { dest, .. }
            | MirInstrKind::EnumCreate { dest, .. }
            | MirInstrKind::EnumTag { dest, .. }
            | MirInstrKind::EnumPayload { dest, .. }
            | MirInstrKind::TupleCreate { dest, .. }
            | MirInstrKind::TupleGet { dest, .. }
            | MirInstrKind::ClosureCreate { dest, .. }
            | MirInstrKind::WrapOk { dest, .. }
            | MirInstrKind::WrapErr { dest, .. }
            | MirInstrKind::IsOk { dest, .. }
            | MirInstrKind::UnwrapOk { dest, .. }
            | MirInstrKind::UnwrapErr { dest, .. }
            | MirInstrKind::Cast { dest, .. }
            | MirInstrKind::TypeOf { dest, .. } => Some(dest),
            MirInstrKind::Call { dest, .. }
            | MirInstrKind::MethodCall { dest, .. }
            | MirInstrKind::FfiCall { dest, .. }
            | MirInstrKind::ClosureCall { dest, .. } => dest.as_ref(),
            MirInstrKind::Await { dest, .. }
            | MirInstrKind::Spawn { dest, .. }
            | MirInstrKind::ScopeCreate { dest, .. }
            | MirInstrKind::ScopeWait { dest, .. } => Some(dest),
            _ => None,
        }
    }
}

// ============================================================================
// Errors
// ============================================================================

/// MIR validation errors
#[derive(Debug, Clone)]
pub enum MirError {
    UndefinedValue { name: Sym, block: Sym },
    TypeMismatch { expected: TypeId, found: TypeId },
    InvalidTerminator { block: Sym },
}

impl std::fmt::Display for MirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UndefinedValue { name, block } => {
                write!(
                    f,
                    "undefined value '{}' in block '{}'",
                    resolve(*name),
                    resolve(*block)
                )
            }
            Self::TypeMismatch { expected, found } => {
                write!(f, "type mismatch: expected {}, found {}", expected, found)
            }
            Self::InvalidTerminator { block } => {
                write!(f, "invalid terminator in block '{}'", resolve(*block))
            }
        }
    }
}

impl std::error::Error for MirError {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sym::sym;
    use doo_core::types::TypeId;

    #[test]
    fn test_pure_ownership_instructions() {
        // Move - transfers ownership
        let move_instr = MirInstr::new(MirInstrKind::Move {
            dest: sym("%1"),
            src: MirOperand::Local(sym("x")),
        });
        assert_eq!(move_instr.destination(), Some(&sym("%1")));

        // Clone - deep copy
        let clone_instr = MirInstr::new(MirInstrKind::Clone {
            dest: sym("%2"),
            src: MirOperand::Local(sym("y")),
        });
        assert_eq!(clone_instr.destination(), Some(&sym("%2")));

        // Drop - cleanup
        let drop_instr = MirInstr::new(MirInstrKind::Drop { value: sym("z") });
        assert_eq!(drop_instr.destination(), None);
    }

    #[test]
    fn test_no_incref_decref() {
        // Verify we have NO reference counting instructions
        // This is intentional - pure ownership model
        let valid_kinds = vec![
            "Move",
            "Copy",
            "Clone",
            "Drop",
            "Borrow",
            "Assign",
            "BinaryOp",
            "UnaryOp",
            "ArrayCreate",
            "ArrayGet",
            "ArraySet",
            "ArrayLen",
            "ArrayContains",
            "MapCreate",
            "MapGet",
            "MapSet",
            "MapHas",
            "StructCreate",
            "FieldGet",
            "FieldSet",
            "EnumCreate",
            "EnumTag",
            "EnumPayload",
            "TupleCreate",
            "TupleGet",
            "Call",
            "MethodCall",
            "FfiCall",
            "ClosureCreate",
            "ClosureCall",
            "WrapOk",
            "WrapErr",
            "IsOk",
            "UnwrapOk",
            "UnwrapErr",
            "Cast",
            "Print",
        ];

        // No "IncRef" or "DecRef" in the list
        assert!(!valid_kinds.contains(&"IncRef"));
        assert!(!valid_kinds.contains(&"DecRef"));
    }

    #[test]
    fn test_function_validation() {
        let mut func = MirFunction::new(sym("test"));
        func.params.push(ParamDef {
            name: sym("x"),
            type_id: TypeId(100),
        });

        let mut block = MirBlock::new(sym("entry"));
        block
            .instructions
            .push(MirInstr::new(MirInstrKind::BinaryOp {
                dest: sym("%1"),
                op: BinaryOp::Add,
                lhs: MirOperand::Local(sym("x")),
                rhs: MirOperand::Const(MirConst::Int(1)),
            }));
        block.terminator = MirTerminator::Return {
            values: vec![MirOperand::Temp(sym("%1"))],
        };
        func.blocks.push(block);

        assert!(func.validate().is_ok());
    }
}
