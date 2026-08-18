//! Type Context (TyCtxt) — The central compilation context.
//!
//! Instead of a complex query database, Doo uses a `TyCtxt` struct that holds
//! references to the global, immutable compilation state (Arena, Interner,
//! TypeRegistry). This is passed by reference to every compiler pass.
//!
//! ## Design (Architecture Part VI)
//!
//! - **Zero-Cost**: Passing `&TyCtxt` is a single pointer copy.
//! - **Immutable**: The context is read-only, preventing passes from mutating global state.
//! - **Future-Proof**: When incremental compilation (Phase 45) is added, this struct
//!   can be wrapped in a query engine (like Salsa) without rewriting the pass logic.

use crate::arena::CompilerArena;
use crate::intern::Interner;
use crate::types::registry::TypeRegistry;
use std::hash::{Hash, Hasher};

/// The central compilation context.
///
/// Contains references to all global, immutable state required during compilation.
/// Every compiler pass (Parser, HIR, THIR, MIR) receives a `&TyCtxt` to access
/// shared resources.
pub struct TyCtxt<'tcx> {
    /// The arena allocator for AST/HIR/MIR nodes.
    pub arena: &'tcx CompilerArena,

    /// The global string interner.
    pub interner: &'tcx Interner,

    /// The central type registry (Single Source of Truth for types).
    pub type_registry: &'tcx TypeRegistry,
}

impl<'tcx> TyCtxt<'tcx> {
    /// Create a new type context.
    #[inline]
    pub fn new(
        arena: &'tcx CompilerArena,
        interner: &'tcx Interner,
        type_registry: &'tcx TypeRegistry,
    ) -> Self {
        Self {
            arena,
            interner,
            type_registry,
        }
    }

    /// Intern a string into the global interner.
    #[inline]
    pub fn intern(&self, s: &str) -> crate::symbol::Symbol {
        self.interner.intern(s)
    }

    /// Resolve a symbol back to its string.
    #[inline]
    pub fn resolve(&self, sym: crate::symbol::Symbol) -> &'static str {
        crate::intern::resolve(sym)
    }
}

// ======================================================================
// Incremental Compilation Support
// ======================================================================

use rustc_hash::FxHashMap;

/// 128-bit fingerprint for deterministic query result caching.
///
/// Used by the red-green incremental compilation algorithm to detect
/// whether a query's output has changed between compilations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    /// High 64 bits.
    pub hi: u64,
    /// Low 64 bits.
    pub lo: u64,
}

impl Fingerprint {
    /// Create a zero fingerprint (for uninitialized queries).
    pub const ZERO: Self = Self { hi: 0, lo: 0 };

    /// Check if this fingerprint is zero.
    pub fn is_zero(&self) -> bool {
        self.hi == 0 && self.lo == 0
    }
}

/// Identifies a query in the incremental compilation system.
///
/// Format: `"<query_kind>:<input_key>"` (e.g. `"parse_file:main.doo"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QueryId(pub String);

impl QueryId {
    /// Create a new query ID.
    pub fn new(kind: &str, key: &str) -> Self {
        Self(format!("{}:{}", kind, key))
    }

    /// Get the query kind (portion before ':').
    pub fn kind(&self) -> &str {
        self.0.split(':').next().unwrap_or("")
    }

    /// Get the query key (portion after ':').
    pub fn key(&self) -> &str {
        self.0.split(':').nth(1).unwrap_or("")
    }
}

impl std::fmt::Display for QueryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Dependency graph tracking query input/output relationships.
///
/// When a query's input changes (RED), all queries that depend on it
/// are marked for recomputation. If a recomputed query's output
/// fingerprint is unchanged, its dependents remain valid (GREEN).
pub struct DependencyGraph {
    /// Maps a query to the queries it depends on (its inputs).
    dependencies: FxHashMap<QueryId, Vec<QueryId>>,
    /// Maps a query to the queries that depend on it (its dependents).
    dependents: FxHashMap<QueryId, Vec<QueryId>>,
}

impl DependencyGraph {
    /// Create an empty dependency graph.
    pub fn new() -> Self {
        Self {
            dependencies: FxHashMap::default(),
            dependents: FxHashMap::default(),
        }
    }

    /// Register a dependency: `query` depends on `dep`.
    pub fn add_dependency(&mut self, query: QueryId, dep: QueryId) {
        self.dependencies
            .entry(query.clone())
            .or_default()
            .push(dep.clone());
        self.dependents.entry(dep).or_default().push(query);
    }

    /// Get all queries that `query` depends on.
    pub fn get_dependencies(&self, query: &QueryId) -> Option<&Vec<QueryId>> {
        self.dependencies.get(query)
    }

    /// Get all queries that depend on `query`.
    pub fn get_dependents(&self, query: &QueryId) -> Option<&Vec<QueryId>> {
        self.dependents.get(query)
    }

    /// Collect all transitive dependents of `query` (for red-green invalidation).
    pub fn transitive_dependents(&self, query: &QueryId) -> Vec<QueryId> {
        let mut visited = FxHashMap::default();
        let mut stack = vec![query.clone()];
        let mut result = Vec::new();

        while let Some(q) = stack.pop() {
            if visited.contains_key(&q) {
                continue;
            }
            visited.insert(q.clone(), true);

            if let Some(deps) = self.dependents.get(&q) {
                for dep in deps {
                    if !visited.contains_key(dep) {
                        result.push(dep.clone());
                        stack.push(dep.clone());
                    }
                }
            }
        }

        result
    }

    /// Remove all edges for a query (when it's recomputed with new dependencies).
    pub fn clear_dependencies(&mut self, query: &QueryId) {
        if let Some(old_deps) = self.dependencies.remove(query) {
            for dep in &old_deps {
                if let Some(dep_dependents) = self.dependents.get_mut(dep) {
                    dep_dependents.retain(|d| d != query);
                }
            }
        }
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Caches query fingerprints for red-green incremental compilation.
///
/// Stores the last-computed fingerprint for each query, along with
/// the dependency graph that tracks which queries depend on which.
pub struct QueryCache {
    /// Last-computed fingerprint for each query.
    fingerprints: FxHashMap<QueryId, Fingerprint>,
    /// Dependency graph tracking query relationships.
    graph: DependencyGraph,
}

impl QueryCache {
    /// Create an empty query cache.
    pub fn new() -> Self {
        Self {
            fingerprints: FxHashMap::default(),
            graph: DependencyGraph::new(),
        }
    }

    /// Store a query result with its fingerprint and dependencies.
    ///
    /// Replaces any previous dependencies for this query.
    pub fn store(&mut self, id: QueryId, fingerprint: Fingerprint, deps: Vec<QueryId>) {
        self.graph.clear_dependencies(&id);
        for dep in &deps {
            self.graph.add_dependency(id.clone(), dep.clone());
        }
        self.fingerprints.insert(id, fingerprint);
    }

    /// Get the stored fingerprint for a query.
    pub fn get_fingerprint(&self, id: &QueryId) -> Option<Fingerprint> {
        self.fingerprints.get(id).copied()
    }

    /// Check if a query's fingerprint matches the stored value.
    pub fn fingerprint_matches(&self, id: &QueryId, new_fp: Fingerprint) -> bool {
        self.fingerprints
            .get(id)
            .map_or(false, |&old| old == new_fp)
    }

    /// Get all transitive dependents of a query (for invalidation).
    pub fn transitive_dependents(&self, id: &QueryId) -> Vec<QueryId> {
        self.graph.transitive_dependents(id)
    }

    /// Get the dependencies of a query.
    pub fn get_dependencies(&self, id: &QueryId) -> Option<&Vec<QueryId>> {
        self.graph.get_dependencies(id)
    }

    /// Remove a query from the cache.
    pub fn remove(&mut self, id: &QueryId) {
        self.graph.clear_dependencies(id);
        self.fingerprints.remove(id);
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.fingerprints.clear();
        self.graph = DependencyGraph::new();
    }
}

impl Default for QueryCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrapper to use blake3 with std::hash::Hasher interface.
struct Blake3Hasher(blake3::Hasher);

impl Hasher for Blake3Hasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        let hash = self.0.finalize();
        let bytes = hash.as_bytes();
        u64::from_be_bytes(bytes[0..8].try_into().unwrap())
    }
}

impl Fingerprint {
    /// Compute a deterministic 128-bit fingerprint using blake3.
    ///
    /// Stable across process invocations and Rust compiler versions,
    /// suitable for persistent on-disk cache files.
    pub fn of<T: Hash + ?Sized>(value: &T) -> Fingerprint {
        let mut hasher = Blake3Hasher(blake3::Hasher::new());
        value.hash(&mut hasher);
        let hash = hasher.0.finalize();
        let bytes = hash.as_bytes();
        Fingerprint {
            hi: u64::from_be_bytes(bytes[0..8].try_into().unwrap()),
            lo: u64::from_be_bytes(bytes[8..16].try_into().unwrap()),
        }
    }

    /// Compute a fingerprint for source text, including file path.
    pub fn of_source(path: &str, source: &str) -> Fingerprint {
        let mut combined = String::with_capacity(path.len() + source.len() + 1);
        combined.push_str(path);
        combined.push('\0');
        combined.push_str(source);
        Self::of(&combined)
    }
}
