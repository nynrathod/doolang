//! THIR Item and Program Definitions

use doo_core::types::TypeId;
use doo_core::Span;

use crate::expr::ThirExpr;
use crate::stmt::ThirStmt;

/// Top-level THIR items.
#[derive(Debug, Clone)]
pub enum ThirItem {
    Const(ThirConst),
    Static(ThirStatic),
    Function(ThirFunction),
    Struct(ThirStruct),
    Enum(ThirEnum),
    Interface(ThirInterface),
    Import(ThirImport),
}

#[derive(Debug, Clone)]
pub struct ThirConst {
    pub name: String,
    pub is_public: bool,
    pub value_expr: ThirExpr,
    pub ty: TypeId,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ThirStatic {
    pub name: String,
    pub is_public: bool,
    pub ty: Option<TypeId>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ThirFunction {
    pub name: String,
    pub type_params: Vec<String>,
    pub params: Vec<ThirParam>,
    pub return_type: Option<TypeId>,
    pub error_type: Option<TypeId>,
    pub body: Vec<ThirStmt>,
    pub is_async: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ThirParam {
    pub name: String,
    pub ty: Option<TypeId>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ThirStruct {
    pub name: String,
    pub type_params: Vec<String>,
    pub fields: Vec<ThirField>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ThirField {
    pub name: String,
    pub ty: Option<TypeId>,
    pub is_public: bool,
    pub is_optional: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ThirEnum {
    pub name: String,
    pub variants: Vec<ThirVariant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ThirVariant {
    pub name: String,
    pub payload: Option<TypeId>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ThirInterface {
    pub name: String,
    pub methods: Vec<ThirInterfaceMethod>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ThirInterfaceMethod {
    pub name: String,
    pub params: Vec<ThirParam>,
    pub return_type: Option<TypeId>,
    pub error_type: Option<TypeId>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ThirImport {
    pub path: Vec<String>,
    pub items: Vec<ThirImportItem>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ThirImportItem {
    Symbol(String),
    Alias { name: String, alias: String },
    Wildcard,
}
