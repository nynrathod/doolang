//! Type inference constraint generation infrastructure.
//!
//! Sets up constraint types and type variable generation for the unification engine.

use doo_core::types::TypeId;
use doo_core::Span;
use rustc_hash::FxHashMap;

/// A constraint generated during type inference.
#[derive(Debug, Clone)]
pub enum TypeConstraint {
    /// Two types must be equal.
    Eq(TypeId, TypeId, Span),
    /// The type must implement Copy.
    IsCopy(TypeId, Span),
    /// The type must implement the given interface.
    IsInterface {
        ty: TypeId,
        iface: String,
        span: Span,
    },
    /// The type must have a field, and the field's type must be the result.
    FieldAccess {
        ty: TypeId,
        field: String,
        result: TypeId,
        span: Span,
    },
    /// The type must have a method, and the method's return type must be the result.
    MethodCall {
        receiver: TypeId,
        method: String,
        args: Vec<TypeId>,
        result: TypeId,
        span: Span,
    },
}

/// A table of type variable substitutions.
#[derive(Debug, Clone, Default)]
pub struct TypeVarTable {
    /// Maps a TypeVar TypeId to its concrete TypeId.
    bindings: FxHashMap<TypeId, TypeId>,
    /// Counter for generating fresh type variables.
    counter: u32,
}

impl TypeVarTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Generate a fresh type variable.
    pub fn fresh_var(&mut self, registry: &mut doo_core::types::TypeRegistry) -> TypeId {
        let name = format!("__t{}", self.counter);
        self.counter += 1;
        registry.register_type_param(&name)
    }

    /// Substitute all type variables in the given type with their bindings.
    pub fn substitute(&self, ty: TypeId, registry: &doo_core::types::TypeRegistry) -> TypeId {
        if let Some(&concrete) = self.bindings.get(&ty) {
            return self.substitute(concrete, registry);
        }
        ty
    }

    /// Bind a type variable to a concrete type.
    pub fn bind(&mut self, var: TypeId, concrete: TypeId) {
        self.bindings.insert(var, concrete);
    }
}
