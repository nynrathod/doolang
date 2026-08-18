//! Type Inference Utilities
//!
/// Provides type inference helpers used during semantic analysis.
/// The main type inference (unification) happens during HIR lowering;
/// these utilities support post-inference lookups like method return
/// type resolution and closure context tracking.
use doo_core::types::{builtin, TypeId, TypeKind, TypeRegistry};
use doo_core::Span;
use rustc_hash::FxHashMap;

/// Type inference error.
#[derive(Debug, Clone)]
pub struct InferenceError {
    pub message: String,
    pub span: Span,
}

impl InferenceError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl std::fmt::Display for InferenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Context for tracking closure captures and their types.
#[derive(Debug, Clone, Default)]
pub struct ClosureContext {
    /// Captured variable name → type.
    captures: FxHashMap<String, TypeId>,
    /// Whether the closure escapes its defining scope.
    escapes: bool,
}

impl ClosureContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a captured variable.
    pub fn capture(&mut self, name: impl Into<String>, ty: TypeId) {
        self.captures.insert(name.into(), ty);
    }

    /// Mark the closure as escaping its scope (requires heap allocation).
    pub fn mark_escapes(&mut self) {
        self.escapes = true;
    }

    /// Check if the closure escapes.
    pub fn escapes(&self) -> bool {
        self.escapes
    }

    /// Get all captures.
    pub fn captures(&self) -> &FxHashMap<String, TypeId> {
        &self.captures
    }
}

/// Type inference engine for post-HIR type resolution.
///
/// This is NOT the main Hindley-Milner unification engine (that lives
/// in `doo_hir/src/lowering/unify.rs`). This struct provides utility
/// methods for looking up inferred types during semantic analysis.
pub struct TypeInference<'a> {
    registry: &'a mut TypeRegistry,
    /// Variable name → inferred type (within current function scope).
    var_types: FxHashMap<String, TypeId>,
}

impl<'a> TypeInference<'a> {
    pub fn new(registry: &'a mut TypeRegistry) -> Self {
        Self {
            registry,
            var_types: FxHashMap::default(),
        }
    }

    /// Record the inferred type for a variable.
    pub fn record_var(&mut self, name: impl Into<String>, ty: TypeId) {
        self.var_types.insert(name.into(), ty);
    }

    /// Look up the inferred type for a variable.
    pub fn lookup_var(&self, name: &str) -> Option<TypeId> {
        self.var_types.get(name).copied()
    }

    /// Infer the return type of a method call on a given receiver type.
    ///
    /// Uses known patterns (constructors, accessors) and the type registry
    /// to determine what a method returns without a full trait solver pass.
    pub fn infer_method_return(
        &mut self,
        receiver_ty: TypeId,
        method: &str,
        arg_types: &[TypeId],
    ) -> Option<TypeId> {
        // Extract the necessary type information first to avoid holding an
        // immutable borrow of `self.registry` while calling `&mut self` methods.
        if let Some(info) = self.registry.get(receiver_ty) {
            match &info.kind {
                TypeKind::Array { element } => {
                    let element = *element;
                    return self.infer_array_method(element, method, arg_types);
                }
                TypeKind::Map { key, value } => {
                    let key = *key;
                    let value = *value;
                    return self.infer_map_method(key, value, method, arg_types);
                }
                TypeKind::Str => {
                    return self.infer_str_method(method, arg_types);
                }
                TypeKind::Int | TypeKind::Int64 | TypeKind::Int32 => {
                    return self.infer_int_method(method, arg_types);
                }
                TypeKind::Float64 | TypeKind::Float32 => {
                    return self.infer_float_method(method, arg_types);
                }
                _ => {}
            }
        }

        // Self-returning methods (constructors, accessors)
        if false {
            return Some(receiver_ty);
        }

        None
    }

    /// Infer return type for Array methods.
    fn infer_array_method(
        &mut self,
        element: TypeId,
        method: &str,
        _arg_types: &[TypeId],
    ) -> Option<TypeId> {
        match method {
            "len" | "isEmpty" => Some(builtin::INT),
            "contains" | "startsWith" | "endsWith" => Some(builtin::BOOL),
            "first" | "last" | "pop" => Some(self.registry.register_optional(element)),
            "push" | "insert" | "set" | "clear" | "reverse" | "sort" => Some(builtin::VOID),
            "join" => Some(builtin::STR),
            "slice" => self
                .registry
                .lookup(&format!("[{}]", self.registry.display_name(element))),
            "map" | "filter" | "reduce" => Some(element),
            "indexOf" => Some(builtin::INT),
            _ => None,
        }
    }

    /// Infer return type for Map methods.
    fn infer_map_method(
        &mut self,
        key: TypeId,
        value: TypeId,
        method: &str,
        _arg_types: &[TypeId],
    ) -> Option<TypeId> {
        match method {
            "len" | "isEmpty" => Some(builtin::INT),
            "has" => Some(builtin::BOOL),
            "get" => Some(self.registry.register_optional(value)),
            "set" | "remove" | "clear" => Some(builtin::VOID),
            "keys" => Some(self.registry.register_array(key)),
            "values" => Some(self.registry.register_array(value)),
            _ => None,
        }
    }

    /// Infer return type for Str methods.
    fn infer_str_method(&mut self, method: &str, _arg_types: &[TypeId]) -> Option<TypeId> {
        match method {
            "len" | "indexOf" | "countSubstr" | "charCode" => Some(builtin::INT),
            "charAt" => Some(builtin::CHAR),
            "isEmpty" | "startsWith" | "endsWith" | "contains" => Some(builtin::BOOL),
            "substring" | "concat" | "replace" | "replaceAll" | "trim" | "trimStart"
            | "trimEnd" | "toUpper" | "toLower" | "repeat" => Some(builtin::STR),
            "split" => Some(self.registry.register_array(builtin::STR)),
            _ => None,
        }
    }

    /// Infer return type for Int methods.
    fn infer_int_method(&mut self, method: &str, _arg_types: &[TypeId]) -> Option<TypeId> {
        match method {
            "toStr" => Some(builtin::STR),
            "toChar" => Some(builtin::CHAR),
            _ => None,
        }
    }

    /// Infer return type for Float methods.
    fn infer_float_method(&mut self, method: &str, _arg_types: &[TypeId]) -> Option<TypeId> {
        match method {
            "toStr" => Some(builtin::STR),
            _ => None,
        }
    }
}

/// Convenience function to infer a method's return type.
pub fn infer_method_return_type(
    registry: &mut TypeRegistry,
    receiver_ty: TypeId,
    method: &str,
    arg_types: &[TypeId],
) -> Option<TypeId> {
    let mut inferrer = TypeInference::new(registry);
    inferrer.infer_method_return(receiver_ty, method, arg_types)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_inference_creation() {
        let mut registry = TypeRegistry::new();
        let mut inferrer = TypeInference::new(&mut registry);
        assert!(inferrer.lookup_var("x").is_none());
    }

    #[test]
    fn test_var_type_recording() {
        let mut registry = TypeRegistry::new();
        let mut inferrer = TypeInference::new(&mut registry);
        inferrer.record_var("x", builtin::INT);
        assert_eq!(inferrer.lookup_var("x"), Some(builtin::INT));
    }

    #[test]
    fn test_closure_context() {
        let mut ctx = ClosureContext::new();
        ctx.capture("x", builtin::INT);
        ctx.capture("y", builtin::STR);
        assert_eq!(ctx.captures().len(), 2);
        assert!(!ctx.escapes());
        ctx.mark_escapes();
        assert!(ctx.escapes());
    }

    #[test]
    fn test_infer_str_len() {
        let mut registry = TypeRegistry::new();
        let result = infer_method_return_type(&mut registry, builtin::STR, "len", &[]);
        assert_eq!(result, Some(builtin::INT));
    }

    #[test]
    fn test_infer_str_to_upper() {
        let mut registry = TypeRegistry::new();
        let result = infer_method_return_type(&mut registry, builtin::STR, "toUpper", &[]);
        assert_eq!(result, Some(builtin::STR));
    }
}
