//! THIR Type System Support and Core Types

use doo_core::types::TypeId;

/// How a method call was resolved during Type Check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImplResolution {
    /// Inherent method defined directly on the struct.
    Direct,
    /// Trait method resolved to a specific interface.
    Trait(String),
    /// Static dispatch via vtable index (for dynamic dispatch, if added later).
    StaticDispatch { trait_name: String, impl_idx: usize },
}

/// How a variable is captured by a closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// bitwise copy (primitives only)
    Copy,
    /// temporary read access
    Borrow,
    /// transfer ownership
    Move,
}

/// A captured variable's metadata in a closure environment.
#[derive(Debug, Clone)]
pub struct ThirCapture {
    pub name: String,
    pub ty: TypeId,
    pub mode: CaptureMode,
}

/// A THIR program with fully resolved types.
#[derive(Debug, Clone)]
pub struct ThirProgram {
    pub items: Vec<ThirItem>,
    pub span: doo_core::Span,
}
