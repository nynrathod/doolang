//! Composite types in Doo.
//!
//! These are user-defined types or complex built-ins that aggregate primitives.

use super::TypeId;
use crate::span::Span;
use crate::symbol::Symbol;
use serde::{Deserialize, Serialize};

/// A decorator/annotation (e.g., `@table("users")`).
/// Opaque to the compiler core; macro packages give it meaning.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DecoratorDef {
    pub name: Symbol,
    pub args: Vec<String>,
    pub span: Span,
}

/// A struct field definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: Symbol,
    pub type_id: TypeId,
    pub is_public: bool,
    pub is_optional: bool,
    pub default_value: Option<String>,
    pub decorators: Vec<DecoratorDef>,
}

/// A struct definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StructDef {
    pub name: Symbol,
    pub fields: Vec<FieldDef>,
    pub is_public: bool,
    pub decorators: Vec<DecoratorDef>,
}

/// An enum variant definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VariantDef {
    pub name: Symbol,
    pub payload: Option<TypeId>,
    pub decorators: Vec<DecoratorDef>,
}

/// An enum definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnumDef {
    pub name: Symbol,
    pub variants: Vec<VariantDef>,
    pub is_public: bool,
}

/// A method signature inside an interface.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MethodSig {
    pub name: Symbol,
    pub params: Vec<TypeId>,
    pub return_type: TypeId,
    pub error_type: Option<TypeId>,
}

/// An interface definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InterfaceDef {
    pub name: Symbol,
    pub methods: Vec<MethodSig>,
    pub is_public: bool,
}

/// A function type signature (for function pointers/closures).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FunctionSig {
    pub params: Vec<TypeId>,
    pub return_type: TypeId,
    pub error_type: Option<TypeId>,
    pub is_closure: bool,
}

/// A unified enum for all composite types.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CompositeDef {
    Struct(StructDef),
    Enum(EnumDef),
    Interface(InterfaceDef),
    Function(FunctionSig),
    Box(TypeId),
}
