//! Collection types in Doo.
//!
//! Collections are parameterized container types: Array<T>, Map<K, V>, Tuple<...>.
//! These are heap-allocated and reference-counted.

use super::TypeId;
use serde::{Deserialize, Serialize};

/// Collection types - containers for other values.
///
/// All collections are heap-allocated and use reference counting for memory management.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CollectionType {
    /// Dynamic array: [T]
    /// 
    /// Internally: ptr to { len: i64, cap: i64, data: [T] }
    Array {
        element: TypeId,
    },

    /// Hash map: Map<K, V>
    /// 
    /// Keys must be hashable (primitives or strings).
    Map {
        key: TypeId,
        value: TypeId,
    },

    /// Fixed-size tuple: (T1, T2, ...)
    /// 
    /// Unlike arrays, tuples can have heterogeneous types.
    Tuple {
        elements: Vec<TypeId>,
    },

    /// Range type for iteration: start..end or start..=end
    Range {
        element: TypeId,
        inclusive: bool,
    },

    /// Optional type: T?
    /// 
    /// Nullable wrapper around any type.
    Optional {
        inner: TypeId,
    },

    /// Result type: Result<Ok, Err>
    /// 
    /// For error handling without exceptions.
    Result {
        ok: TypeId,
        err: TypeId,
    },
}

impl CollectionType {
    /// Get the primary element type of this collection.
    ///
    /// For Array: the element type
    /// For Map: the value type
    /// For Tuple: the first element type (if any)
    /// For Optional/Result: the inner/ok type
    pub fn element_type(&self) -> Option<TypeId> {
        match self {
            Self::Array { element } => Some(*element),
            Self::Map { value, .. } => Some(*value),
            Self::Tuple { elements } => elements.first().copied(),
            Self::Range { element, .. } => Some(*element),
            Self::Optional { inner } => Some(*inner),
            Self::Result { ok, .. } => Some(*ok),
        }
    }

    /// Get all type parameters for this collection.
    pub fn type_params(&self) -> Vec<TypeId> {
        match self {
            Self::Array { element } => vec![*element],
            Self::Map { key, value } => vec![*key, *value],
            Self::Tuple { elements } => elements.clone(),
            Self::Range { element, .. } => vec![*element],
            Self::Optional { inner } => vec![*inner],
            Self::Result { ok, err } => vec![*ok, *err],
        }
    }

    /// Size in bytes for the collection header (not including elements).
    ///
    /// All collections use pointer indirection, so the value stored is always a pointer.
    pub fn size_bytes(&self) -> usize {
        8 // All collections are pointer-sized (reference to heap data)
    }

    /// Alignment requirement.
    pub fn alignment(&self) -> usize {
        8 // Pointer alignment
    }

    /// Whether this collection type can contain the given element type.
    pub fn is_valid_element(&self, element_type: TypeId, registry_check: impl Fn(TypeId) -> bool) -> bool {
        match self {
            Self::Map { key, .. } => {
                // Map keys must be hashable (delegated to registry)
                registry_check(*key)
            }
            _ => true,
        }
    }

    /// Human-readable type name with parameters.
    pub fn format_name(&self, format_type: impl Fn(TypeId) -> String) -> String {
        match self {
            Self::Array { element } => format!("[{}]", format_type(*element)),
            Self::Map { key, value } => {
                format!("Map<{}, {}>", format_type(*key), format_type(*value))
            }
            Self::Tuple { elements } => {
                let parts: Vec<_> = elements.iter().map(|e| format_type(*e)).collect();
                format!("({})", parts.join(", "))
            }
            Self::Range { element, inclusive } => {
                let op = if *inclusive { "..=" } else { ".." };
                format!("Range<{}>{}", format_type(*element), op)
            }
            Self::Optional { inner } => format!("{}?", format_type(*inner)),
            Self::Result { ok, err } => {
                format!("Result<{}, {}>", format_type(*ok), format_type(*err))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_array_element_type() {
        let arr = CollectionType::Array { element: TypeId(100) };
        assert_eq!(arr.element_type(), Some(TypeId(100)));
    }

    #[test]
    fn test_map_type_params() {
        let map = CollectionType::Map {
            key: TypeId(101),
            value: TypeId(102),
        };
        assert_eq!(map.type_params(), vec![TypeId(101), TypeId(102)]);
    }

    #[test]
    fn test_tuple_type_params() {
        let tuple = CollectionType::Tuple {
            elements: vec![TypeId(100), TypeId(101), TypeId(102)],
        };
        assert_eq!(tuple.type_params().len(), 3);
    }

    #[test]
    fn test_result_type() {
        let result = CollectionType::Result {
            ok: TypeId(100),
            err: TypeId(200),
        };
        assert_eq!(result.element_type(), Some(TypeId(100)));
        assert_eq!(result.type_params(), vec![TypeId(100), TypeId(200)]);
    }

    #[test]
    fn test_format_name() {
        let arr = CollectionType::Array { element: TypeId(100) };
        let name = arr.format_name(|id| format!("T{}", id.0));
        assert_eq!(name, "[T100]");
    }
}
