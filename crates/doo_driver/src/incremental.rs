//! Incremental Compilation — file hashing, cache manifest, selective rebuild.
//!
//! ## Architecture
//!
//! The incremental compilation system works by:
//! 1. Hashing each source file's contents
//! 2. Comparing hashes against a cached manifest
//! 3. Only recompiling files whose hashes have changed
//! 4. Re-linking only when any object file has changed
//!
//! ## Cache Layout
//!
//! ```text
//! .doo-cache/
//!   manifest.json       # { file -> hash } mapping
//!   obj/                # Cached object files
//!     main.doo.o
//!     models.doo.o
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! let mut cache = CompilationCache::load(".doo-cache")?;
//!
//! for file in source_files {
//!     if cache.needs_rebuild(&file) {
//!         compile(file);
//!         cache.update(&file, new_hash);
//!     }
//! }
//!
//! if cache.has_changes() {
//!     link_all();
//! }
//!
//! cache.save()?;
//! ```

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

/// Size of the read buffer for file hashing.
const HASH_BUF_SIZE: usize = 8192;

/// Cache manifest for incremental compilation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CacheManifest {
    /// Version of the cache format (for invalidation on changes).
    pub version: u32,
    /// Source file path → content hash.
    pub file_hashes: HashMap<String, u64>,
    /// Compiler version hash (invalidate on compiler changes).
    pub compiler_hash: u64,
}

impl Default for CacheManifest {
    fn default() -> Self {
        Self {
            version: 1,
            file_hashes: HashMap::new(),
            compiler_hash: 0,
        }
    }
}

/// Manages the incremental compilation cache.
pub struct CompilationCache {
    /// Path to the cache directory.
    cache_dir: PathBuf,
    /// Previous build's manifest (loaded from disk).
    previous: CacheManifest,
    /// Current build's manifest (built during compilation).
    current: CacheManifest,
    /// Files that need rebuilding.
    dirty_files: Vec<String>,
    /// Whether any file has changed since last build.
    has_changes: bool,
}

impl CompilationCache {
    /// Load or create a compilation cache at the given directory.
    pub fn load(cache_dir: &Path) -> io::Result<Self> {
        let manifest_path = cache_dir.join("manifest.json");

        let previous = if manifest_path.exists() {
            let contents = std::fs::read_to_string(&manifest_path)?;
            serde_json::from_str(&contents).unwrap_or_default()
        } else {
            CacheManifest::default()
        };

        let compiler_hash = Self::current_compiler_hash();

        let mut cache = Self {
            cache_dir: cache_dir.to_path_buf(),
            previous,
            current: CacheManifest::default(),
            dirty_files: Vec::new(),
            has_changes: false,
        };

        // Invalidate cache if compiler binary changed
        if cache.previous.compiler_hash != compiler_hash {
            cache.invalidate();
        }
        cache.current.compiler_hash = compiler_hash;

        Ok(cache)
    }

    /// Create an empty cache (no previous manifest).
    pub fn new_empty(cache_dir: &Path) -> Self {
        let mut current = CacheManifest::default();
        current.compiler_hash = Self::current_compiler_hash();
        Self {
            cache_dir: cache_dir.to_path_buf(),
            previous: CacheManifest::default(),
            current,
            dirty_files: Vec::new(),
            has_changes: false,
        }
    }

    /// Hash the current compiler binary by reading its header bytes + file size.
    /// Different builds produce different binary content, making this reliable
    /// across WSL/Windows filesystem boundaries (unlike mtime).
    fn current_compiler_hash() -> u64 {
        use std::io::Read;
        if let Ok(exe) = std::env::current_exe() {
            if let Ok(mut f) = std::fs::File::open(&exe) {
                let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                // Read first 8KB of binary (PE/ELF header + code start)
                let mut buf = [0u8; 8192];
                let n = f.read(&mut buf).unwrap_or(0);
                // Append file size for additional discrimination
                let mut data = Vec::with_capacity(n + 8);
                data.extend_from_slice(&buf[..n]);
                data.extend_from_slice(&len.to_le_bytes());
                return Self::hash_bytes(&data);
            }
        }
        0
    }

    /// Check if a source file needs recompilation.
    ///
    /// Computes the file's content hash and compares with the cached hash.
    /// Returns `true` if the file is new or has changed.
    pub fn needs_rebuild(&mut self, file_path: &Path) -> io::Result<bool> {
        let path_str = file_path.to_string_lossy().to_string();
        let content = std::fs::read(file_path)?;
        let hash = Self::hash_bytes(&content);

        self.current.file_hashes.insert(path_str.clone(), hash);

        let needs_rebuild = match self.previous.file_hashes.get(&path_str) {
            Some(prev_hash) => *prev_hash != hash,
            None => true, // New file
        };

        if needs_rebuild {
            self.dirty_files.push(path_str);
            self.has_changes = true;
        }

        Ok(needs_rebuild)
    }

    /// Check if any files have changed since the last build.
    pub fn has_changes(&self) -> bool {
        self.has_changes
    }

    /// Get the list of files that need rebuilding.
    pub fn dirty_files(&self) -> &[String] {
        &self.dirty_files
    }

    /// Save the current manifest to disk.
    pub fn save(&self) -> io::Result<()> {
        // Create cache directory if needed
        std::fs::create_dir_all(&self.cache_dir)?;
        std::fs::create_dir_all(self.cache_dir.join("obj"))?;

        let manifest_path = self.cache_dir.join("manifest.json");
        let json = serde_json::to_string_pretty(&self.current)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        std::fs::write(manifest_path, json)?;

        Ok(())
    }

    /// Get the path where a cached object file should be stored.
    pub fn object_path(&self, source_file: &Path) -> PathBuf {
        let stem = source_file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        self.cache_dir.join("obj").join(format!("{}.o", stem))
    }

    /// Invalidate the entire cache (force full rebuild).
    pub fn invalidate(&mut self) {
        self.previous = CacheManifest::default();
        self.has_changes = true;
    }

    /// Simple FNV-1a hash for file contents.
    /// Fast, deterministic, good enough for cache invalidation.
    fn hash_bytes(data: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325; // FNV offset basis
        for &byte in data {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3); // FNV prime
        }
        hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_deterministic() {
        let data = b"hello world";
        assert_eq!(
            CompilationCache::hash_bytes(data),
            CompilationCache::hash_bytes(data)
        );
    }

    #[test]
    fn test_hash_different() {
        assert_ne!(
            CompilationCache::hash_bytes(b"hello"),
            CompilationCache::hash_bytes(b"world")
        );
    }

    #[test]
    fn test_default_manifest() {
        let manifest = CacheManifest::default();
        assert_eq!(manifest.version, 1);
        assert!(manifest.file_hashes.is_empty());
    }
}
