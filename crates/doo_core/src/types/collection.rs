//! Collection types in Doo.
//!
//! Collections are parameterized container types: Array<T>, Map<K, V>, Set<T>, Tuple.

use super::TypeId;

/// Collection types - containers for other values.
///
/// All collections are heap-allocated.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CollectionType {
    /// Dynamic array: `[T]`
    /// Internally: ptr to { len, cap, data }
    Array { element: TypeId },
    /// Hash map: `{K: V}`
    /// Keys must be hashable (primitives or strings).
    Map { key: TypeId, value: TypeId },
    /// Hash set: `{T}`
    Set { element: TypeId },
    /// Fixed-size tuple: `(T1, T2, ...)`
    /// Heterogeneous, inline layout.
    Tuple { elements: Vec<TypeId> },
}

impl CollectionType {
    /// Get all type parameters for this collection.
    pub fn type_params(&self) -> Vec<TypeId> {
        match self {
            Self::Array { element } => vec![*element],
            Self::Map { key, value } => vec![*key, *value],
            Self::Set { element } => vec![*element],
            Self::Tuple { elements } => elements.clone(),
        }
    }

    /// Check if the collection contains a specific type.
    pub fn contains_type(&self, target: TypeId) -> bool {
        self.type_params().contains(&target)
    }
}
