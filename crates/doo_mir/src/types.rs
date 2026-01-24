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

use std::collections::HashMap;

use doo_core::types::TypeId;

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
    pub structs: HashMap<String, StructDef>,
    /// Enum metadata: name -> variant definitions  
    pub enums: HashMap<String, EnumDef>,
    /// Entry point function name (usually "main")
    pub entry_point: Option<String>,
}

/// Global constant or variable
#[derive(Debug, Clone)]
pub struct MirGlobal {
    pub name: String,
    pub type_id: TypeId,
    pub value: Option<MirConst>,
}

/// Struct definition metadata
#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
    pub decorators: Vec<Decorator>,
}

/// Field definition with decorators
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
    pub type_id: TypeId,
    pub optional: bool,
    pub decorators: Vec<Decorator>,
    pub default_value: Option<MirConst>,
}

/// Enum definition metadata
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<VariantDef>,
}

/// Enum variant definition
#[derive(Debug, Clone)]
pub struct VariantDef {
    pub name: String,
    pub index: u32,
    pub payload_type: Option<TypeId>,
}

/// Decorator on struct/field
#[derive(Debug, Clone)]
pub struct Decorator {
    pub name: String,
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

    /// Validate all functions
    pub fn validate(&self) -> Result<(), MirError> {
        for func in &self.functions {
            func.validate()?;
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
    pub name: String,
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
}

/// Parameter definition
#[derive(Debug, Clone)]
pub struct ParamDef {
    pub name: String,
    pub type_id: TypeId,
}

/// Local variable definition
#[derive(Debug, Clone)]
pub struct LocalDef {
    pub name: String,
    pub type_id: TypeId,
    pub mutable: bool,
}

/// FFI linkage information
#[derive(Debug, Clone)]
pub struct FfiLinkage {
    pub library: String,
    pub symbol: Option<String>,
}

impl MirFunction {
    pub fn new(name: String) -> Self {
        Self {
            name,
            params: Vec::new(),
            return_type: None,
            error_type: None,
            blocks: Vec::new(),
            locals: Vec::new(),
            ffi: None,
            span: Span::default(),
        }
    }

    /// Validate function's CFG and value definitions
    pub fn validate(&self) -> Result<(), MirError> {
        // Collect all defined values
        let mut defined: std::collections::HashSet<String> = std::collections::HashSet::new();
        
        // Parameters are pre-defined
        for param in &self.params {
            defined.insert(param.name.clone());
        }
        
        // Check each block
        for block in &self.blocks {
            for instr in &block.instructions {
                // Check operands are defined
                for operand in instr.operands() {
                    if let MirOperand::Local(name) | MirOperand::Temp(name) = operand {
                        if !defined.contains(name) && !name.starts_with('@') {
                            return Err(MirError::UndefinedValue {
                                name: name.clone(),
                                block: block.label.clone(),
                            });
                        }
                    }
                }
                
                // Add defined value
                if let Some(dest) = instr.destination() {
                    defined.insert(dest.clone());
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
    pub label: String,
    /// Instructions (no terminators)
    pub instructions: Vec<MirInstr>,
    /// Block terminator
    pub terminator: MirTerminator,
}

impl MirBlock {
    pub fn new(label: String) -> Self {
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
    Goto { target: String },
    /// Conditional branch
    Branch {
        cond: MirOperand,
        then_block: String,
        else_block: String,
    },
    /// Multi-way branch (switch/match)
    Switch {
        value: MirOperand,
        cases: Vec<(i64, String)>,
        default: String,
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
    Local(String),
    /// Temporary value (SSA)
    Temp(String),
    /// Global reference
    Global(String),
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
    Move { dest: String, src: MirOperand },
    
    /// Copy value (for primitives or explicit Copy types)
    Copy { dest: String, src: MirOperand },
    
    /// Clone value (deep copy for non-Copy types)
    Clone { dest: String, src: MirOperand },
    
    /// Drop value (cleanup, inserted by analysis)
    Drop { value: String },
    
    /// Borrow value (create reference for function call)
    Borrow { dest: String, src: String, mutable: bool },

    // ========================================================================
    // Assignment
    // ========================================================================
    
    /// Assign constant or operand to local
    Assign { dest: String, value: MirOperand },

    // ========================================================================
    // Arithmetic & Logic
    // ========================================================================
    
    /// Binary operation
    BinaryOp {
        dest: String,
        op: BinaryOp,
        lhs: MirOperand,
        rhs: MirOperand,
    },
    
    /// Unary operation
    UnaryOp {
        dest: String,
        op: UnaryOp,
        operand: MirOperand,
    },

    // ========================================================================
    // Collections
    // ========================================================================
    
    /// Create array
    ArrayCreate {
        dest: String,
        elements: Vec<MirOperand>,
        elem_type: TypeId,
    },
    
    /// Get array element
    ArrayGet {
        dest: String,
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
    ArrayLen {
        dest: String,
        array: MirOperand,
    },
    
    /// Check if array contains value
    ArrayContains {
        dest: String,
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
        dest: String,
        array: MirOperand,
        start: MirOperand,
        end: MirOperand,
        elem_type: TypeId,
    },

    
    /// Create map
    MapCreate {
        dest: String,
        entries: Vec<(MirOperand, MirOperand)>,
        key_type: TypeId,
        val_type: TypeId,
    },
    
    /// Get map value
    MapGet {
        dest: String,
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
        dest: String,
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
        dest: String,
        struct_name: String,
        fields: Vec<(String, MirOperand)>,
    },
    
    /// Get struct field
    FieldGet {
        dest: String,
        object: MirOperand,
        field: String,
    },
    
    /// Set struct field
    FieldSet {
        object: MirOperand,
        field: String,
        value: MirOperand,
    },

    // ========================================================================
    // Enums
    // ========================================================================
    
    /// Create enum variant
    EnumCreate {
        dest: String,
        enum_name: String,
        variant: String,
        payload: Option<MirOperand>,
    },
    
    /// Get enum discriminant (tag)
    EnumTag {
        dest: String,
        value: MirOperand,
    },
    
    /// Get enum discriminant (tag) with enum name for type lookup
    EnumGetTag {
        dest: String,
        value: MirOperand,
        enum_name: String,
    },
    
    /// Compare enum tag with expected variant
    EnumTagEquals {
        dest: String,
        tag: MirOperand,
        variant_name: String,
        enum_name: String,
    },
    
    /// Extract enum payload
    EnumPayload {
        dest: String,
        value: MirOperand,
        variant: String,
    },
    
    /// Extract enum payload with index (for ADT pattern matching)
    EnumGetPayload {
        dest: String,
        value: MirOperand,
        variant_name: String,
        enum_name: String,
        index: u32,
    },

    // ========================================================================
    // Tuples
    // ========================================================================
    
    /// Create tuple
    TupleCreate {
        dest: String,
        elements: Vec<MirOperand>,
    },
    
    /// Extract tuple element
    TupleGet {
        dest: String,
        tuple: MirOperand,
        index: usize,
    },

    // ========================================================================
    // Calls
    // ========================================================================
    
    /// Function call
    Call {
        dest: Option<String>,
        func: String,
        args: Vec<MirOperand>,
    },
    
    /// Method call
    MethodCall {
        dest: Option<String>,
        receiver: MirOperand,
        receiver_type: TypeId,
        method: String,
        args: Vec<MirOperand>,
        arg_types: Vec<TypeId>,
    },
    
    /// FFI call
    FfiCall {
        dest: Option<String>,
        lib: String,
        symbol: String,
        args: Vec<MirOperand>,
    },

    // ========================================================================
    // Closures
    // ========================================================================
    
    /// Create closure
    ClosureCreate {
        dest: String,
        func: String,
        captures: Vec<MirOperand>,
    },
    
    /// Call closure
    ClosureCall {
        dest: Option<String>,
        closure: MirOperand,
        args: Vec<MirOperand>,
    },

    // ========================================================================
    // Result/Error Handling
    // ========================================================================
    
    /// Wrap value in Ok
    WrapOk {
        dest: String,
        value: MirOperand,
    },
    
    /// Wrap value in Err
    WrapErr {
        dest: String,
        value: MirOperand,
    },
    
    /// Check if result is Ok
    IsOk {
        dest: String,
        value: MirOperand,
    },
    
    /// Unwrap Ok value
    UnwrapOk {
        dest: String,
        value: MirOperand,
    },
    
    /// Unwrap Err value
    UnwrapErr {
        dest: String,
        value: MirOperand,
    },

    // ========================================================================
    // Cast & Conversion
    // ========================================================================
    
    /// Type cast
    Cast {
        dest: String,
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
    },
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    // Arithmetic
    Add, Sub, Mul, Div, Mod,
    // Comparison
    Eq, Ne, Lt, Le, Gt, Ge,
    // Logical
    And, Or,
    // String
    Concat,
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,    // -x
    Not,    // !x
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
        Self { kind, span: Span::default() }
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
            MirInstrKind::ArraySet { array, index, value, .. } => vec![array, index, value],
            MirInstrKind::ArrayLen { array, .. } => vec![array],
            MirInstrKind::ArrayContains { array, value, .. } => vec![array, value],
            MirInstrKind::ArrayPush { array, value } => vec![array, value],
            MirInstrKind::ArrayExtend { array, other, .. } => vec![array, other],
            MirInstrKind::ArraySlice { array, start, end, .. } => vec![array, start, end],


            MirInstrKind::MapCreate { entries, .. } => {
                entries.iter().flat_map(|(k, v)| vec![k, v]).collect()
            }
            MirInstrKind::MapGet { map, key, .. } => vec![map, key],
            MirInstrKind::MapSet { map, key, value, .. } => vec![map, key, value],
            MirInstrKind::MapHas { map, key, .. } => vec![map, key],
            MirInstrKind::StructCreate { fields, .. } => {
                fields.iter().map(|(_, v)| v).collect()
            }
            MirInstrKind::FieldGet { object, .. } => vec![object],
            MirInstrKind::FieldSet { object, value, .. } => vec![object, value],
            MirInstrKind::EnumCreate { payload: Some(p), .. } => vec![p],
            MirInstrKind::EnumTag { value, .. } => vec![value],
            MirInstrKind::EnumPayload { value, .. } => vec![value],
            MirInstrKind::TupleCreate { elements, .. } => elements.iter().collect(),
            MirInstrKind::TupleGet { tuple, .. } => vec![tuple],
            MirInstrKind::Call { args, .. } => args.iter().collect(),
            MirInstrKind::MethodCall { receiver, receiver_type: _, method: _, args, .. } => {
                std::iter::once(receiver).chain(args.iter()).collect()
            }
            MirInstrKind::FfiCall { args, .. } => args.iter().collect(),
            // ...
            MirInstrKind::UnwrapErr { value, .. } => vec![value],
            MirInstrKind::Cast { value, .. } => vec![value],
            MirInstrKind::Print { values, value_types: _ } => values.iter().collect(),
            _ => vec![],
        }
    }
    
    /// Get destination (defined value) if any
    pub fn destination(&self) -> Option<&String> {
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
            | MirInstrKind::Cast { dest, .. } => Some(dest),
            MirInstrKind::Call { dest, .. }
            | MirInstrKind::MethodCall { dest, .. }
            | MirInstrKind::FfiCall { dest, .. }
            | MirInstrKind::ClosureCall { dest, .. } => dest.as_ref(),
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
    UndefinedValue { name: String, block: String },
    TypeMismatch { expected: TypeId, found: TypeId },
    InvalidTerminator { block: String },
}

impl std::fmt::Display for MirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UndefinedValue { name, block } => {
                write!(f, "undefined value '{}' in block '{}'", name, block)
            }
            Self::TypeMismatch { expected, found } => {
                write!(f, "type mismatch: expected {}, found {}", expected, found)
            }
            Self::InvalidTerminator { block } => {
                write!(f, "invalid terminator in block '{}'", block)
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
    use doo_core::types::TypeId;

    #[test]
    fn test_pure_ownership_instructions() {
        // Move - transfers ownership
        let move_instr = MirInstr::new(MirInstrKind::Move {
            dest: "%1".to_string(),
            src: MirOperand::Local("x".to_string()),
        });
        assert_eq!(move_instr.destination(), Some(&"%1".to_string()));
        
        // Clone - deep copy
        let clone_instr = MirInstr::new(MirInstrKind::Clone {
            dest: "%2".to_string(),
            src: MirOperand::Local("y".to_string()),
        });
        assert_eq!(clone_instr.destination(), Some(&"%2".to_string()));
        
        // Drop - cleanup
        let drop_instr = MirInstr::new(MirInstrKind::Drop {
            value: "z".to_string(),
        });
        assert_eq!(drop_instr.destination(), None);
    }

    #[test]
    fn test_no_incref_decref() {
        // Verify we have NO reference counting instructions
        // This is intentional - pure ownership model
        let valid_kinds = vec![
            "Move", "Copy", "Clone", "Drop", "Borrow",
            "Assign", "BinaryOp", "UnaryOp",
            "ArrayCreate", "ArrayGet", "ArraySet", "ArrayLen",
            "ArrayContains",
            "MapCreate", "MapGet", "MapSet", "MapHas",
            "StructCreate", "FieldGet", "FieldSet",
            "EnumCreate", "EnumTag", "EnumPayload",
            "TupleCreate", "TupleGet",
            "Call", "MethodCall", "FfiCall",
            "ClosureCreate", "ClosureCall",
            "WrapOk", "WrapErr", "IsOk", "UnwrapOk", "UnwrapErr",
            "Cast", "Print",
        ];
        
        // No "IncRef" or "DecRef" in the list
        assert!(!valid_kinds.contains(&"IncRef"));
        assert!(!valid_kinds.contains(&"DecRef"));
    }

    #[test]
    fn test_function_validation() {
        let mut func = MirFunction::new("test".to_string());
        func.params.push(ParamDef {
            name: "x".to_string(),
            type_id: TypeId(100),
        });
        
        let mut block = MirBlock::new("entry".to_string());
        block.instructions.push(MirInstr::new(MirInstrKind::BinaryOp {
            dest: "%1".to_string(),
            op: BinaryOp::Add,
            lhs: MirOperand::Local("x".to_string()),
            rhs: MirOperand::Const(MirConst::Int(1)),
        }));
        block.terminator = MirTerminator::Return {
            values: vec![MirOperand::Temp("%1".to_string())],
        };
        func.blocks.push(block);
        
        assert!(func.validate().is_ok());
    }
}
