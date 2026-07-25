//! Declaration AST nodes.
//!
//! Top-level declarations: functions, structs, enums, imports, consts.

use super::{Expr, Stmt, TypeExpr};
use doo_core::Span;

// ============================================================================
// Const Declaration
// ============================================================================

/// A compile-time constant declaration: `const Name = expr`
///
/// - PascalCase name → public const (accessible via import)
/// - camelCase name  → private const (module-internal)
/// - Value must be a compile-time literal expression (no function calls, no structs)
/// - Can hold: primitives (Int, Float, Bool, Str), arrays of primitives, maps of primitives
#[derive(Debug, Clone)]
pub struct ConstDecl {
    pub name: String,
    pub is_public: bool,
    pub value: Expr,
    pub span: Span,
}

impl ConstDecl {
    pub fn new(name: String, value: Expr, span: Span) -> Self {
        let is_public = name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);
        Self {
            name,
            is_public,
            value,
            span,
        }
    }
}

// ============================================================================
// Static Declaration
// ============================================================================

/// A runtime global variable declaration: `static Name: Type`
///
/// - PascalCase name → public static (accessible across files)
/// - camelCase name  → private static (module-internal)
/// - Type annotation is required (compiler needs to allocate OnceLock)
/// - Set exactly once in main(), immutable after
/// - Compiles to OnceLock behind the scenes for thread safety
#[derive(Debug, Clone)]
pub struct StaticDecl {
    pub name: String,
    pub is_public: bool,
    pub type_expr: TypeExpr,
    pub span: Span,
}

impl StaticDecl {
    pub fn new(name: String, type_expr: TypeExpr, span: Span) -> Self {
        let is_public = name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);
        Self {
            name,
            is_public,
            type_expr,
            span,
        }
    }
}

// ============================================================================
// Generic Type Parameters
// ============================================================================

/// A generic type parameter declaration: `<T>` or `<T: SomeInterface>`.
///
/// Used on function and struct declarations to declare type variables.
/// The compiler monomorphizes generic definitions at call/construction sites.
#[derive(Debug, Clone)]
pub struct TypeParam {
    /// Type parameter name (e.g. "T", "A", "B").
    pub name: String,
    /// Optional interface constraint: `<T: Displayable>`.
    pub constraint: Option<String>,
    /// Source location.
    pub span: Span,
}

/// Function declaration.
#[derive(Debug, Clone)]
pub struct FunctionDecl {
    /// Function name.
    pub name: String,
    /// Visibility (public/private based on name casing).
    pub is_public: bool,
    /// Generic type parameters: `fn first<T>(...)` → `[TypeParam("T")]`.
    pub type_params: Vec<TypeParam>,
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
            type_params: Vec::new(),
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
    /// Generic type parameters: `struct Wrapper<T>` → `[TypeParam("T")]`.
    pub type_params: Vec<TypeParam>,
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
            type_params: Vec::new(),
            fields: Vec::new(),
            decorators: Vec::new(),
            span,
        }
    }
}

/// Impl block for struct methods.
/// Desugars to individual `fn TypeName.method()` items during lowering.
#[derive(Debug, Clone)]
pub struct ImplDecl {
    /// Target struct name.
    pub struct_name: String,
    /// Methods defined in the impl block.
    pub methods: Vec<FunctionDecl>,
    /// Impl-level decorators.
    pub decorators: Vec<Decorator>,
    /// Source location.
    pub span: Span,
}

impl ImplDecl {
    pub fn new(struct_name: String, span: Span) -> Self {
        Self {
            struct_name,
            methods: Vec::new(),
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
// Interface
// ============================================================================

/// Interface declaration.
///
/// Defines a contract that structs can satisfy by implementing all its methods.
/// Satisfaction is implicit (like Go) — no `implements` keyword needed.
///
/// Example:
/// ```doo
/// interface CloudProvider {
///     fn deploy(slug: Str, image: Str) -> Str ! Str
///     fn delete(slug: Str, region: Str) -> Str ! Str
/// }
/// ```
#[derive(Debug, Clone)]
pub struct InterfaceDecl {
    /// Interface name.
    pub name: String,
    /// Whether this interface is public.
    pub is_public: bool,
    /// Method signatures (bodies are NOT in the interface, only signatures).
    pub methods: Vec<InterfaceMethodDecl>,
    /// Source location.
    pub span: Span,
}

impl InterfaceDecl {
    pub fn new(name: String, span: Span) -> Self {
        let is_public = name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);
        Self {
            name,
            is_public,
            methods: Vec::new(),
            span,
        }
    }
}

/// A method signature inside an interface declaration.
/// Only the signature — no body.
#[derive(Debug, Clone)]
pub struct InterfaceMethodDecl {
    /// Method name.
    pub name: String,
    /// Parameters: (name, type?)
    pub params: Vec<(String, Option<TypeExpr>)>,
    /// Return type.
    pub return_type: Option<TypeExpr>,
    /// Error type for fallible methods.
    pub error_type: Option<TypeExpr>,
    /// Source location.
    pub span: Span,
}

impl InterfaceMethodDecl {
    pub fn new(name: String, span: Span) -> Self {
        Self {
            name,
            params: Vec::new(),
            return_type: None,
            error_type: None,
            span,
        }
    }
}

// ============================================================================
// RBAC Policy (Framework domain - slated for removal in Audit Phase 2)
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
        Self {
            name,
            for_struct,
            rules: Vec::new(),
            span,
        }
    }
}
