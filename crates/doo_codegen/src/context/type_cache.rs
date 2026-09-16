//! Type Cache — caches LLVM types for MIR types to avoid recomputation.
//!
//! Struct layouts and complex type translations are computed once and cached.
//! Keyed by TypeId for O(1) lookup on subsequent uses.

use doo_core::types::TypeId;
use inkwell::types::BasicTypeEnum;
use rustc_hash::FxHashMap;

/// Cache for LLVM type representations of Doo types.
///
/// Avoids recomputing struct layouts and complex type translations
/// on every use. Keyed by TypeId for O(1) lookup.
pub struct TypeCache<'ctx> {
    types: FxHashMap<TypeId, BasicTypeEnum<'ctx>>,
}

impl<'ctx> TypeCache<'ctx> {
    /// Create an empty type cache.
    pub fn new() -> Self {
        Self {
            types: FxHashMap::default(),
        }
    }

    /// Look up a cached LLVM type by Doo TypeId.
    pub fn get(&self, id: TypeId) -> Option<BasicTypeEnum<'ctx>> {
        self.types.get(&id).copied()
    }

    /// Insert a computed LLVM type into the cache.
    pub fn insert(&mut self, id: TypeId, ty: BasicTypeEnum<'ctx>) {
        self.types.insert(id, ty);
    }

    /// Get a cached type or compute and cache it.
    ///
    /// The compute closure is only called on cache miss.
    pub fn get_or_compute<F>(&mut self, id: TypeId, compute: F) -> BasicTypeEnum<'ctx>
    where
        F: FnOnce() -> BasicTypeEnum<'ctx>,
    {
        if let Some(&ty) = self.types.get(&id) {
            return ty;
        }
        let ty = compute();
        self.types.insert(id, ty);
        ty
    }

    /// Clear all cached types (e.g., between compilation units).
    pub fn clear(&mut self) {
        self.types.clear();
    }

    /// Number of cached types.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }
}

impl<'ctx> Default for TypeCache<'ctx> {
    fn default() -> Self {
        Self::new()
    }
}
