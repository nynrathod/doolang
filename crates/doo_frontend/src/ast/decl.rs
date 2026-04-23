//! Declaration AST nodes.
//!
//! Top-level declarations: functions, structs, enums, imports.

use super::{Stmt, TypeExpr};
use doo_core::Span;

/// Function declaration.
#[derive(Debug, Clone)]
pub struct FunctionDecl {
    /// Function name.
    pub name: String,
    /// Visibility (public/private based on name casing).
    pub is_public: bool,
    /// Parameters: (name, type?)
    pub params: Vec<(String, Option<TypeExpr>)>,
    /// Return type.
    pub return_type: Option<TypeExpr>,
    /// Error type for Result-returning functions.
    pub error_type: Option<TypeExpr>,
    /// Function body.
    pub body: Vec<Stmt>,
    /// Decorators like @ffi, @extern.
    pub decorators: Vec<Decorator>,
    /// Receiver type for methods: `fn TypeName.method(self)`
    pub receiver: Option<String>,
    /// Associated type for static/instance methods.
    pub associated_type: Option<String>,
    /// Is this an expression function (uses =>)?
    pub is_expr_fn: bool,
    /// Is this an async function (`async fn`)?
    pub is_async: bool,
    /// Source location.
    pub span: Span,
}

impl FunctionDecl {
    pub fn new(name: String, span: Span) -> Self {
        let is_public = name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);
        Self {
            name,
            is_public,
            params: Vec::new(),
            return_type: None,
            error_type: None,
            body: Vec::new(),
            decorators: Vec::new(),
            receiver: None,
            associated_type: None,
            is_expr_fn: false,
            is_async: false,
            span,
        }
    }
}

/// Struct declaration.
#[derive(Debug, Clone)]
pub struct StructDecl {
    /// Struct name.
    pub name: String,
    /// Whether this struct is public.
    pub is_public: bool,
    /// Fields.
    pub fields: Vec<FieldDecl>,
    /// Struct-level decorators like @table.
    pub decorators: Vec<Decorator>,
    /// Source location.
    pub span: Span,
}

impl StructDecl {
    pub fn new(name: String, span: Span) -> Self {
        let is_public = name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);
        Self {
            name,
            is_public,
            fields: Vec::new(),
            decorators: Vec::new(),
            span,
        }
    }
}

/// A struct field declaration.
#[derive(Debug, Clone)]
pub struct FieldDecl {
    /// Field name.
    pub name: String,
    /// Field type.
    pub type_expr: TypeExpr,
    /// Is this field public?
    pub is_public: bool,
    /// Is this field optional (T?)?
    pub is_optional: bool,
    /// Default value expression.
    pub default: Option<super::Expr>,
    /// Field decorators.
    pub decorators: Vec<Decorator>,
    /// Source location.
    pub span: Span,
}

/// Enum declaration.
#[derive(Debug, Clone)]
pub struct EnumDecl {
    /// Enum name.
    pub name: String,
    /// Whether this enum is public.
    pub is_public: bool,
    /// Variants.
    pub variants: Vec<VariantDecl>,
    /// Source location.
    pub span: Span,
}

impl EnumDecl {
    pub fn new(name: String, span: Span) -> Self {
        let is_public = name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);
        Self {
            name,
            is_public,
            variants: Vec::new(),
            span,
        }
    }
}

/// An enum variant declaration.
#[derive(Debug, Clone)]
pub struct VariantDecl {
    /// Variant name.
    pub name: String,
    /// Payload type (for data-carrying variants).
    pub payload: Option<TypeExpr>,
    /// Decorators (e.g. @inherits(User)).
    pub decorators: Vec<Decorator>,
    /// Source location.
    pub span: Span,
}

/// Import declaration.
#[derive(Debug, Clone)]
pub struct ImportDecl {
    /// Import path: `std::io`
    pub path: Vec<String>,
    /// Imported items: `{ Foo, Bar }`
    pub items: Vec<ImportItem>,
    /// Module Alias: `import std::io as io`
    pub alias: Option<String>,
    /// Wildcard import: `import std::io::*`
    pub wildcard: bool,
    /// Source location.
    pub span: Span,
}

/// An imported item.
#[derive(Debug, Clone)]
pub enum ImportItem {
    /// Single symbol: `Foo`
    Symbol(String),
    /// Aliased: `Foo as Bar`
    Alias { name: String, alias: String },
    /// Wildcard: `*`
    Wildcard,
}

/// A decorator/annotation.
#[derive(Debug, Clone)]
pub struct Decorator {
    /// Decorator name (without @).
    pub name: String,
    /// Arguments: @min(8) -> ["8"]
    pub args: Vec<super::Expr>,
    /// Source location.
    pub span: Span,
}

impl Decorator {
    pub fn new(name: String, span: Span) -> Self {
        Self {
            name,
            args: Vec::new(),
            span,
        }
    }

    pub fn with_args(name: String, args: Vec<super::Expr>, span: Span) -> Self {
        Self { name, args, span }
    }
}

// ============================================================================
// RBAC Policy
// ============================================================================

/// A policy block: `policy FooPolicy for Foo { create: public, ... }`.
///
/// Policies are compiled into RBAC metadata and enforced by the HTTP FFI at
/// runtime. They are NOT function bodies — just declarative rule tables.
#[derive(Debug, Clone)]
pub struct PolicyDecl {
    /// Policy name (e.g. "PostPolicy").
    pub name: String,
    /// The struct this policy guards (e.g. "Post").
    pub for_struct: String,
    /// CRUD + custom actions with their access rules (serialised as strings).
    /// e.g. `("create", "authenticated")`, `("update", "own|Admin")`
    pub rules: Vec<(String, String)>,
    /// Source location.
    pub span: Span,
}

impl PolicyDecl {
    pub fn new(name: String, for_struct: String, span: Span) -> Self {
        Self { name, for_struct, rules: Vec::new(), span }
    }
}
