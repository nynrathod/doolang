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

pub mod types;
pub mod builder;
pub mod optimize;
pub mod sym;

pub use sym::Sym;
pub use types::{
    // Program structure
    MirProgram, MirFunction, MirBlock, MirGlobal,
    // Instructions
    MirInstr, MirInstrKind, MirTerminator,
    // Operands
    MirOperand, MirConst,
    // Operators
    BinaryOp, UnaryOp,
    // Metadata
    StructDef, EnumDef, FieldDef, VariantDef, Decorator,
    ParamDef, LocalDef, FfiLinkage,
    // Metadata
    Span,
    // Errors
    MirError,
};

pub use doo_core::types::TypeId;
