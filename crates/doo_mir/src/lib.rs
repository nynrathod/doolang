//! # Doo MIR
//!
//! Mid-level Intermediate Representation for the Doo compiler.
//!
//! ## Design Philosophy
//!
//! - **Pure Ownership**: Move/Copy/Clone/Drop semantics, NO reference counting
//! - **TypeId-based**: All types referenced by ID from central TypeRegistry
//! - **Minimal IR**: Only essential instructions, LLVM handles optimization
//! - **Single Source of Truth**: Integrates with doo_core for types

pub mod builder;
pub mod optimize;
pub mod sym;
pub mod types;

pub use sym::{MirLocal, MirSymbolTable, Sym};
pub use types::{
    // Operators
    BinaryOp,
    BlockId,
    Decorator,
    EnumDef,
    FfiLinkage,
    FieldDef,
    GlobalKind,
    LocalDef,
    MirBlock,
    MirConst,
    // Errors
    MirError,
    MirFunction,
    MirGlobal,
    // Instructions
    MirInstr,
    MirInstrKind,
    MirInstruction,
    // Operands
    MirOperand,
    // Program structure
    MirProgram,
    MirTerminator,
    MirType,
    MirValue,
    ParamDef,
    // Metadata
    Span,
    // Metadata
    StructDef,
    UnaryOp,
    VariantDef,
};

pub use doo_core::types::TypeId;
