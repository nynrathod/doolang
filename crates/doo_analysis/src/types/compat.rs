//! Type Compatibility Checking
//!
//! Determines whether two types are assignment-compatible.
//! Enforces strict type rules: no implicit numeric conversion,
//! no mixed-type collections, contravariant function parameters.

use doo_core::types::{builtin, TypeId, TypeKind, TypeRegistry};

/// Wrapper type for compatibility checking operations.
pub struct TypeCompat<'a> {
    registry: &'a TypeRegistry,
}

impl<'a> TypeCompat<'a> {
    pub fn new(registry: &'a TypeRegistry) -> Self {
        Self { registry }
    }

    /// Check if type `a` is assignable to type `b`.
    pub fn assignable(&self, a: TypeId, b: TypeId) -> bool {
        types_compatible(a, b, self.registry)
    }
}

/// Check if two types are compatible.
///
/// Same primitive → compatible. Same named type → compatible.
/// Function types: contravariant params, covariant return.
/// Option/Result: compatible if inner types compatible.
/// No implicit numeric conversion.
pub fn types_compatible(a: TypeId, b: TypeId, registry: &TypeRegistry) -> bool {
    if a == b {
        return true;
    }

    // Any accepts everything (used for type inference recovery)
    if a == builtin::ANY || b == builtin::ANY {
        return true;
    }

    let a_info = match registry.get(a) {
        Some(i) => i,
        None => return false,
    };
    let b_info = match registry.get(b) {
        Some(i) => i,
        None => return false,
    };

    // Optional: T assignable to T?
    match (&a_info.kind, &b_info.kind) {
        (TypeKind::Optional { inner }, _) => return types_compatible(*inner, b, registry),
        (_, TypeKind::Optional { inner }) => return types_compatible(a, *inner, registry),
        _ => {}
    }

    // Result
    if let (
        TypeKind::Result {
            ok: ok_a,
            err: err_a,
        },
        TypeKind::Result {
            ok: ok_b,
            err: err_b,
        },
    ) = (&a_info.kind, &b_info.kind)
    {
        return types_compatible(*ok_a, *ok_b, registry)
            && types_compatible(*err_a, *err_b, registry);
    }

    // Array
    if let (TypeKind::Array { element: elem_a }, TypeKind::Array { element: elem_b }) =
        (&a_info.kind, &b_info.kind)
    {
        return types_compatible(*elem_a, *elem_b, registry);
    }

    // Map
    if let (
        TypeKind::Map {
            key: k_a,
            value: v_a,
        },
        TypeKind::Map {
            key: k_b,
            value: v_b,
        },
    ) = (&a_info.kind, &b_info.kind)
    {
        return types_compatible(*k_a, *k_b, registry) && types_compatible(*v_a, *v_b, registry);
    }

    // Tuple
    if let (TypeKind::Tuple { elements: elems_a }, TypeKind::Tuple { elements: elems_b }) =
        (&a_info.kind, &b_info.kind)
    {
        if elems_a.len() != elems_b.len() {
            return false;
        }
        return elems_a
            .iter()
            .zip(elems_b.iter())
            .all(|(a, b)| types_compatible(*a, *b, registry));
    }

    // Function: contravariant params, covariant return
    if let (TypeKind::Function { sig: sig_a }, TypeKind::Function { sig: sig_b }) =
        (&a_info.kind, &b_info.kind)
    {
        if sig_a.params.len() != sig_b.params.len() {
            return false;
        }
        let params_ok = sig_a
            .params
            .iter()
            .zip(sig_b.params.iter())
            .all(|(pa, pb)| types_compatible(*pb, *pa, registry));
        let ret_ok = types_compatible(sig_a.return_type, sig_b.return_type, registry);
        return params_ok && ret_ok;
    }

    // Struct/Enum: same name
    if let (TypeKind::Struct { def: def_a }, TypeKind::Struct { def: def_b }) =
        (&a_info.kind, &b_info.kind)
    {
        return def_a.name == def_b.name;
    }
    if let (TypeKind::Enum { def: def_a }, TypeKind::Enum { def: def_b }) =
        (&a_info.kind, &b_info.kind)
    {
        return def_a.name == def_b.name;
    }

    if !a_info.name.is_empty() && a_info.name == b_info.name {
        return true;
    }

    false
}

/// Check if a type is recursive without a Box indirection.
///
/// Recursive types must use `Box<T>` to break the infinite size cycle.
/// Returns the name of the offending field type if the recursion is direct.
pub fn check_recursive_type(
    type_id: TypeId,
    registry: &TypeRegistry,
) -> Result<(), RecursiveTypeError> {
    let info = match registry.get(type_id) {
        Some(i) => i,
        None => return Ok(()),
    };

    if let TypeKind::Struct { def } = &info.kind {
        for field in &def.fields {
            if field.type_id == type_id {
                return Err(RecursiveTypeError {
                    type_name: def.name.resolve().to_string(),
                    field_name: field.name.resolve().to_string(),
                });
            }
        }
    }

    Ok(())
}

/// Error when a recursive type lacks a Box indirection.
#[derive(Debug, Clone)]
pub struct RecursiveTypeError {
    pub type_name: String,
    pub field_name: String,
}

impl std::fmt::Display for RecursiveTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "recursive type '{}' has field '{}' of same type — use Box<{}> to break the cycle",
            self.type_name, self.field_name, self.type_name
        )
    }
}
