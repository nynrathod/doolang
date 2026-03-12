//! Query-Based Architecture — on-demand, memoized computation for compiler data.
//!
//! ## Overview
//!
//! Instead of a linear pipeline (parse → HIR → analysis → MIR → codegen),
//! a query-based architecture computes results on demand:
//!
//! ```text
//! query!(type_of(var))
//!   → triggers → query!(hir_of(func))
//!     → triggers → query!(parse(file))
//! ```
//!
//! Each query result is memoized in the `QueryDatabase`. Only re-computed
//! when its inputs change. This is the foundation for:
//! - **Incremental compilation**: Only recompute what changed
//! - **LSP integration**: Re-check on every keystroke (fast)
//! - **Parallel compilation**: Independent queries can run concurrently
//!
//! ## Design
//!
//! Based on the Salsa/rustc query model:
//! - `QueryKey`: Describes what to compute (e.g., "parse file X")
//! - `QueryValue`: The cached result
//! - `QueryDatabase`: Central store for all cached results
//! - `Revision`: Monotonic counter for cache invalidation

use rustc_hash::FxHashMap;
use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};

/// A monotonically increasing revision counter.
/// Used for cache invalidation — when a source file changes,
/// its revision is bumped, invalidating all dependent queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(pub u64);

impl Revision {
    pub const ZERO: Revision = Revision(0);
}

/// Global revision counter.
static GLOBAL_REVISION: AtomicU64 = AtomicU64::new(1);

/// Get the current global revision.
pub fn current_revision() -> Revision {
    Revision(GLOBAL_REVISION.load(Ordering::Acquire))
}

/// Bump the global revision (call when source files change).
pub fn bump_revision() -> Revision {
    Revision(GLOBAL_REVISION.fetch_add(1, Ordering::AcqRel) + 1)
}

/// A query key describing what to compute.
///
/// Each variant represents a different query type in the compiler.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum QueryKey {
    /// Parse a source file → AST
    ParseFile(String),
    /// Lower a file's AST → HIR
    HirOfFile(String),
    /// Analyze a file → diagnostics
    AnalyzeFile(String),
    /// Build MIR for a function
    MirOfFunction(String),
    /// Get the type of a variable in a scope
    TypeOf {
        file: String,
        scope: String,
        name: String,
    },
    /// Get struct definition metadata
    StructDef(String),
    /// Get function signature
    FunctionSignature(String),
    /// Custom query for extensibility
    Custom(String, String),
}

/// A cached query entry.
struct QueryEntry {
    /// The cached value (type-erased).
    value: Box<dyn Any + Send + Sync>,
    /// The revision when this entry was last computed.
    computed_at: Revision,
    /// The revision of the inputs when this entry was computed.
    /// If any input has a newer revision, this entry is stale.
    input_revision: Revision,
}

/// The central query database.
///
/// Stores memoized results for all compiler queries.
/// Thread-safe for concurrent access (future: parallel compilation).
pub struct QueryDatabase {
    /// Cached query results.
    cache: FxHashMap<QueryKey, QueryEntry>,
    /// Input revisions: source file → last-modified revision.
    input_revisions: FxHashMap<String, Revision>,
}

impl Default for QueryDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryDatabase {
    /// Create a new empty query database.
    pub fn new() -> Self {
        Self {
            cache: FxHashMap::default(),
            input_revisions: FxHashMap::default(),
        }
    }

    /// Mark a source file as modified (bumps its revision).
    ///
    /// This invalidates all queries that depend on this file.
    pub fn set_file_changed(&mut self, file: &str) {
        let rev = bump_revision();
        self.input_revisions.insert(file.to_string(), rev);
    }

    /// Get the revision of a source file.
    pub fn file_revision(&self, file: &str) -> Revision {
        self.input_revisions
            .get(file)
            .copied()
            .unwrap_or(Revision::ZERO)
    }

    /// Store a computed query result.
    pub fn store<T: Any + Send + Sync + Clone>(
        &mut self,
        key: QueryKey,
        value: T,
        input_rev: Revision,
    ) {
        let entry = QueryEntry {
            value: Box::new(value),
            computed_at: current_revision(),
            input_revision: input_rev,
        };
        self.cache.insert(key, entry);
    }

    /// Try to get a cached query result.
    ///
    /// Returns `Some(value)` if the cache entry exists and is still valid
    /// (its input revision matches the current input revision).
    /// Returns `None` if the entry is missing or stale.
    pub fn get<T: Any + Send + Sync + Clone>(
        &self,
        key: &QueryKey,
        current_input_rev: Revision,
    ) -> Option<T> {
        let entry = self.cache.get(key)?;

        // Check if the cached result is still valid
        if entry.input_revision < current_input_rev {
            return None; // Stale — input has been modified since computation
        }

        entry.value.downcast_ref::<T>().cloned()
    }

    /// Check if a query result is cached and still valid.
    pub fn is_cached(&self, key: &QueryKey, current_input_rev: Revision) -> bool {
        match self.cache.get(key) {
            Some(entry) => entry.input_revision >= current_input_rev,
            None => false,
        }
    }

    /// Clear all cached results. Used for full rebuild.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Get the number of cached entries.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_revision_ordering() {
        let r1 = bump_revision();
        let r2 = bump_revision();
        assert!(r2 > r1);
    }

    #[test]
    fn test_store_and_get() {
        let mut db = QueryDatabase::new();
        let key = QueryKey::ParseFile("main.doo".to_string());
        let rev = current_revision();

        db.store(key.clone(), 42i32, rev);

        assert_eq!(db.get::<i32>(&key, rev), Some(42));
    }

    #[test]
    fn test_cache_invalidation() {
        let mut db = QueryDatabase::new();
        let key = QueryKey::ParseFile("main.doo".to_string());
        let rev = current_revision();

        db.store(key.clone(), 42i32, rev);

        // Simulate file change
        let new_rev = bump_revision();

        // Old result should be stale
        assert_eq!(db.get::<i32>(&key, new_rev), None);
    }

    #[test]
    fn test_file_changed() {
        let mut db = QueryDatabase::new();

        assert_eq!(db.file_revision("main.doo"), Revision::ZERO);

        db.set_file_changed("main.doo");
        let rev = db.file_revision("main.doo");
        assert!(rev > Revision::ZERO);
    }
}
