//! Composite types in Doo.
//!
//! Composites are user-defined types: Struct, Enum, Function.
//! These provide the foundation for structured data and behavior.

use super::TypeId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Composite types - user-defined structured types.
#[derive(Clone, Debug, PartialEq)]
pub enum CompositeType {
    /// User-defined struct with named fields.
    Struct(StructDef),

    /// User-defined enum with variants.
    Enum(EnumDef),

    /// Function signature: (params) -> returns
    Function(FunctionType),

    /// Type reference by name (resolved during analysis).
    TypeRef(String),

    /// Builtin type provided by FFI (e.g., "Request", "Response", "DB").
    Builtin(String),
}

/// Definition of a struct type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructDef {
    /// Struct name (PascalCase for public, camelCase for private)
    pub name: String,
    /// Fields in declaration order
    pub fields: Vec<FieldDef>,
    /// Whether this struct is public (exported)
    pub is_public: bool,
    /// Struct-level decorators (e.g., @table)
    pub decorators: Vec<DecoratorDef>,
}

/// Definition of a struct field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldDef {
    /// Field name
    pub name: String,
    /// Field type
    pub type_id: TypeId,
    /// Whether this field is public
    pub is_public: bool,
    /// Whether this field is optional (T?)
    pub is_optional: bool,
    /// Default value expression (stored as string for simplicity)
    pub default_value: Option<String>,
    /// Field decorators (@email, @unique, etc.)
    pub decorators: Vec<DecoratorDef>,
}

/// Definition of a decorator/annotation.
///
/// ## Invariant (Phase 0 — Compiler↔Framework Separation)
///
/// `DecoratorDef` is **opaque** to the compiler. The compiler stores
/// decorator names and arguments but does NOT validate their meaning.
/// Domain-specific validation (`@email`, `@table`, `@primary`, etc.)
/// belongs to macro-provider crates (Level 3), not the compiler core.
///
/// The compiler ONLY:
/// - Parses `@name(args)` syntax in the parser
/// - Stores the parsed data in this struct
/// - Attaches it to the relevant item (struct, field, etc.)
/// - Passes it to macro expansion (Phase 6)
///
/// The compiler NEVER:
/// - Validates decorator argument counts
/// - Checks type constraints for specific decorators
/// - Resolves decorator combinations or conflicts
/// - Knows what `@email`, `@table`, or any other decorator means
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecoratorDef {
    /// Decorator name (without @)
    pub name: String,
    /// Decorator arguments as strings
    pub args: Vec<String>,
}

/// Definition of an enum type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnumDef {
    /// Enum name
    pub name: String,
    /// Variants in declaration order
    pub variants: Vec<VariantDef>,
    /// Whether this enum is public
    pub is_public: bool,
}

/// Definition of an enum variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VariantDef {
    /// Variant name
    pub name: String,
    /// Optional payload type for data-carrying variants
    pub payload: Option<TypeId>,
}

/// Function type signature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunctionType {
    /// Parameter types in order
    pub params: Vec<TypeId>,
    /// Return type(s) - can be multiple for tuple returns
    pub returns: Vec<TypeId>,
    /// Error type if this function can fail (Result-returning)
    pub error_type: Option<TypeId>,
    /// Whether this is a closure (captures environment)
    pub is_closure: bool,
}

impl StructDef {
    /// Create a new struct definition.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let is_public = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
        Self {
            name,
            fields: Vec::new(),
            is_public,
            decorators: Vec::new(),
        }
    }

    /// Add a field to this struct.
    pub fn with_field(mut self, field: FieldDef) -> Self {
        self.fields.push(field);
        self
    }

    /// Add a decorator to this struct.
    pub fn with_decorator(mut self, decorator: DecoratorDef) -> Self {
        self.decorators.push(decorator);
        self
    }

    /// Get a field by name.
    pub fn get_field(&self, name: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Get field index by name.
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name == name)
    }

    /// Convert to a HashMap for compatibility with existing code.
    pub fn fields_map(&self) -> HashMap<String, TypeId> {
        self.fields
            .iter()
            .map(|f| (f.name.clone(), f.type_id))
            .collect()
    }

    /// Check if this struct has a decorator with the given name.
    pub fn has_decorator(&self, name: &str) -> bool {
        self.decorators.iter().any(|d| d.name == name)
    }
}

impl FieldDef {
    /// Create a new field definition.
    pub fn new(name: impl Into<String>, type_id: TypeId) -> Self {
        let name = name.into();
        let is_public = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
        Self {
            name,
            type_id,
            is_public,
            is_optional: false,
            default_value: None,
            decorators: Vec::new(),
        }
    }

    /// Mark this field as optional.
    pub fn optional(mut self) -> Self {
        self.is_optional = true;
        self
    }

    /// Set default value.
    pub fn with_default(mut self, value: impl Into<String>) -> Self {
        self.default_value = Some(value.into());
        self
    }

    /// Add a decorator.
    pub fn with_decorator(mut self, decorator: DecoratorDef) -> Self {
        self.decorators.push(decorator);
        self
    }

    /// Check if this field has a specific decorator.
    pub fn has_decorator(&self, name: &str) -> bool {
        self.decorators.iter().any(|d| d.name == name)
    }

    /// Get a decorator by name.
    pub fn get_decorator(&self, name: &str) -> Option<&DecoratorDef> {
        self.decorators.iter().find(|d| d.name == name)
    }
}

impl DecoratorDef {
    /// Create a simple decorator without arguments.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            args: Vec::new(),
        }
    }

    /// Create a decorator with arguments.
    pub fn with_args(name: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }

    /// Get the first argument if present.
    pub fn first_arg(&self) -> Option<&str> {
        self.args.first().map(|s| s.as_str())
    }
}

impl EnumDef {
    /// Create a new enum definition.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let is_public = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
        Self {
            name,
            variants: Vec::new(),
            is_public,
        }
    }

    /// Add a variant to this enum.
    pub fn with_variant(mut self, variant: VariantDef) -> Self {
        self.variants.push(variant);
        self
    }

    /// Get a variant by name.
    pub fn get_variant(&self, name: &str) -> Option<&VariantDef> {
        self.variants.iter().find(|v| v.name == name)
    }

    /// Get variant index by name.
    pub fn variant_index(&self, name: &str) -> Option<usize> {
        self.variants.iter().position(|v| v.name == name)
    }

    /// Convert to a HashMap for compatibility.
    pub fn variants_map(&self) -> HashMap<String, Option<TypeId>> {
        self.variants
            .iter()
            .map(|v| (v.name.clone(), v.payload))
            .collect()
    }
}

impl VariantDef {
    /// Create a unit variant (no payload).
    pub fn unit(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            payload: None,
        }
    }

    /// Create a data-carrying variant.
    pub fn with_payload(name: impl Into<String>, payload: TypeId) -> Self {
        Self {
            name: name.into(),
            payload: Some(payload),
        }
    }

    /// Whether this variant carries data.
    pub fn has_payload(&self) -> bool {
        self.payload.is_some()
    }
}

impl FunctionType {
    /// Create a new function type.
    pub fn new(params: Vec<TypeId>, returns: Vec<TypeId>) -> Self {
        Self {
            params,
            returns,
            error_type: None,
            is_closure: false,
        }
    }

    /// Mark this function as fallible (returns Result).
    pub fn with_error(mut self, error_type: TypeId) -> Self {
        self.error_type = Some(error_type);
        self
    }

    /// Mark this as a closure.
    pub fn as_closure(mut self) -> Self {
        self.is_closure = true;
        self
    }

    /// Get the single return type (or Void if none).
    pub fn return_type(&self) -> Option<TypeId> {
        if self.returns.len() == 1 {
            Some(self.returns[0])
        } else {
            None
        }
    }

    /// Whether this function can fail.
    pub fn is_fallible(&self) -> bool {
        self.error_type.is_some()
    }
}

impl CompositeType {
    /// Get the name of this composite type.
    pub fn name(&self) -> &str {
        match self {
            Self::Struct(s) => &s.name,
            Self::Enum(e) => &e.name,
            Self::Function(_) => "Function",
            Self::TypeRef(name) => name,
            Self::Builtin(name) => name,
        }
    }

    /// Whether this type is public.
    pub fn is_public(&self) -> bool {
        match self {
            Self::Struct(s) => s.is_public,
            Self::Enum(e) => e.is_public,
            Self::Function(_) => true,
            Self::TypeRef(_) => true,
            Self::Builtin(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_struct_def() {
        let user = StructDef::new("User")
            .with_field(FieldDef::new("Id", TypeId(100)))
            .with_field(FieldDef::new("name", TypeId(101)).optional())
            .with_decorator(DecoratorDef::new("table"));

        assert!(user.is_public);
        assert_eq!(user.fields.len(), 2);
        assert!(user.has_decorator("table"));
        assert_eq!(user.get_field("Id").map(|f| f.type_id), Some(TypeId(100)));
    }

    #[test]
    fn test_field_visibility() {
        let public = FieldDef::new("PublicField", TypeId(100));
        let private = FieldDef::new("privateField", TypeId(101));

        assert!(public.is_public);
        assert!(!private.is_public);
    }

    #[test]
    fn test_enum_def() {
        let status = EnumDef::new("Status")
            .with_variant(VariantDef::unit("Active"))
            .with_variant(VariantDef::with_payload("Error", TypeId(100)));

        assert!(status.is_public);
        assert_eq!(status.variants.len(), 2);
        assert!(!status.get_variant("Active").unwrap().has_payload());
        assert!(status.get_variant("Error").unwrap().has_payload());
    }

    #[test]
    fn test_function_type() {
        let fn_type = FunctionType::new(
            vec![TypeId(100), TypeId(101)],
            vec![TypeId(102)],
        )
        .with_error(TypeId(200));

        assert_eq!(fn_type.params.len(), 2);
        assert!(fn_type.is_fallible());
        assert_eq!(fn_type.return_type(), Some(TypeId(102)));
    }
}
