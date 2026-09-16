//! doo.lock Lockfile Parser/Writer
//!
//! Generates and parses the lockfile that pins exact versions and hashes
//! for reproducible builds. Auto-generated — never hand-edited.

use crate::manifest::DependencySource;
use crate::manifest::Version;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ============================================================================
// LockedPackage
// ============================================================================

/// A package with its exact resolved version and content hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    pub hash: String,
    pub source: LockedSource,
    pub dependencies: Vec<String>,
}

/// Serializable representation of a dependency source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LockedSource {
    Registry,
    Git { url: String, tag: Option<String> },
    Path { path: String },
}

impl From<&DependencySource> for LockedSource {
    fn from(source: &DependencySource) -> Self {
        match source {
            DependencySource::Registry => Self::Registry,
            DependencySource::Git { url, tag } => Self::Git {
                url: url.clone(),
                tag: tag.clone(),
            },
            DependencySource::Path { path } => Self::Path {
                path: path.to_string_lossy().to_string(),
            },
        }
    }
}

// ============================================================================
// DooLock
// ============================================================================

/// The complete lockfile with all resolved packages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DooLock {
    pub version: u32,
    pub packages: Vec<LockedPackage>,
}

impl DooLock {
    /// Create an empty lockfile (format version 1).
    pub fn new() -> Self {
        Self {
            version: 1,
            packages: Vec::new(),
        }
    }

    /// Parse doo.lock from a file path.
    ///
    /// Returns an empty lockfile if the file doesn't exist.
    pub fn from_file(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::new());
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

        Self::from_str(&content)
    }

    /// Parse doo.lock from a TOML string.
    pub fn from_str(content: &str) -> Result<Self, String> {
        let lock: Self =
            toml::from_str(content).map_err(|e| format!("Failed to parse doo.lock: {}", e))?;

        if lock.version != 1 {
            return Err(format!(
                "unsupported lockfile version {} — only version 1 is supported",
                lock.version
            ));
        }

        Ok(lock)
    }

    /// Write the lockfile to a file path.
    pub fn to_file(&self, path: &Path) -> Result<(), String> {
        let content = self.to_string();
        std::fs::write(path, content)
            .map_err(|e| format!("Failed to write {}: {}", path.display(), e))
    }

    /// Serialize the lockfile to a TOML string.
    pub fn to_string(&self) -> String {
        let mut out = String::from("# AUTO-GENERATED — do not edit\n");
        match toml::to_string_pretty(self) {
            Ok(toml_str) => out.push_str(&toml_str),
            Err(_) => return String::new(),
        }
        out
    }
    /// Find a locked package by name.
    pub fn get(&self, name: &str) -> Option<&LockedPackage> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Check if a package is in the lockfile.
    pub fn has(&self, name: &str) -> bool {
        self.packages.iter().any(|p| p.name == name)
    }

    /// Add or update a locked package.
    pub fn upsert(&mut self, package: LockedPackage) {
        if let Some(existing) = self.packages.iter_mut().find(|p| p.name == package.name) {
            *existing = package;
        } else {
            self.packages.push(package);
        }
    }

    /// Number of locked packages.
    pub fn len(&self) -> usize {
        self.packages.len()
    }

    /// Check if the lockfile is empty.
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// Check if the lockfile is stale (doesn't match the manifest dependencies).
    ///
    /// Returns true if:
    /// - A manifest dependency is not in the lockfile
    /// - A lockfile package is not in the manifest dependencies
    pub fn is_stale(&self, manifest_dependencies: &[String]) -> bool {
        for dep_name in manifest_dependencies {
            if !self.has(dep_name) {
                return true;
            }
        }
        for pkg in &self.packages {
            if !manifest_dependencies.contains(&pkg.name) {
                return true;
            }
        }
        false
    }

    /// Sort packages alphabetically for deterministic output.
    pub fn sort(&mut self) {
        self.packages.sort_by(|a, b| a.name.cmp(&b.name));
    }
}

impl Default for DooLock {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_lockfile() {
        let lock = DooLock::new();
        assert_eq!(lock.version, 1);
        assert!(lock.is_empty());
    }

    #[test]
    fn test_lockfile_roundtrip() {
        let mut lock = DooLock::new();
        lock.upsert(LockedPackage {
            name: "httpkit".to_string(),
            version: "1.2.0".to_string(),
            hash: "sha256:abc123".to_string(),
            source: LockedSource::Registry,
            dependencies: vec!["jsoncodec".to_string()],
        });
        lock.upsert(LockedPackage {
            name: "jsoncodec".to_string(),
            version: "0.9.4".to_string(),
            hash: "sha256:def456".to_string(),
            source: LockedSource::Registry,
            dependencies: vec![],
        });

        let toml_str = lock.to_string();
        let parsed = DooLock::from_str(&toml_str).unwrap();

        assert_eq!(parsed.len(), 2);
        assert!(parsed.has("httpkit"));
        assert!(parsed.has("jsoncodec"));

        let pkg = parsed.get("httpkit").unwrap();
        assert_eq!(pkg.version, "1.2.0");
        assert_eq!(pkg.hash, "sha256:abc123");
        assert_eq!(pkg.dependencies.len(), 1);
        assert_eq!(pkg.dependencies[0], "jsoncodec");
    }

    #[test]
    fn test_lockfile_with_path_source() {
        let mut lock = DooLock::new();
        lock.upsert(LockedPackage {
            name: "localutils".to_string(),
            version: "0.1.0".to_string(),
            hash: "sha256:path123".to_string(),
            source: LockedSource::Path {
                path: "../utils".to_string(),
            },
            dependencies: vec![],
        });

        let toml_str = lock.to_string();
        let parsed = DooLock::from_str(&toml_str).unwrap();
        let pkg = parsed.get("localutils").unwrap();
        match &pkg.source {
            LockedSource::Path { path } => assert_eq!(path, "../utils"),
            _ => panic!("expected Path source"),
        }
    }

    #[test]
    fn test_lockfile_with_git_source() {
        let mut lock = DooLock::new();
        lock.upsert(LockedPackage {
            name: "authlib".to_string(),
            version: "2.0.0".to_string(),
            hash: "sha256:git789".to_string(),
            source: LockedSource::Git {
                url: "https://github.com/org/authlib".to_string(),
                tag: Some("v2.0.0".to_string()),
            },
            dependencies: vec![],
        });

        let toml_str = lock.to_string();
        let parsed = DooLock::from_str(&toml_str).unwrap();
        let pkg = parsed.get("authlib").unwrap();
        match &pkg.source {
            LockedSource::Git { url, tag } => {
                assert_eq!(url, "https://github.com/org/authlib");
                assert_eq!(tag.as_deref(), Some("v2.0.0"));
            }
            _ => panic!("expected Git source"),
        }
    }

    #[test]
    fn test_lockfile_upsert() {
        let mut lock = DooLock::new();
        lock.upsert(LockedPackage {
            name: "pkg".to_string(),
            version: "1.0.0".to_string(),
            hash: "sha256:aaa".to_string(),
            source: LockedSource::Registry,
            dependencies: vec![],
        });
        assert_eq!(lock.len(), 1);

        // Update existing
        lock.upsert(LockedPackage {
            name: "pkg".to_string(),
            version: "2.0.0".to_string(),
            hash: "sha256:bbb".to_string(),
            source: LockedSource::Registry,
            dependencies: vec![],
        });
        assert_eq!(lock.len(), 1);
        assert_eq!(lock.get("pkg").unwrap().version, "2.0.0");
    }

    #[test]
    fn test_lockfile_is_stale() {
        let mut lock = DooLock::new();
        lock.upsert(LockedPackage {
            name: "httpkit".to_string(),
            version: "1.0.0".to_string(),
            hash: "sha256:abc".to_string(),
            source: LockedSource::Registry,
            dependencies: vec![],
        });

        // Not stale — all manifest deps are in lockfile
        assert!(!lock.is_stale(&["httpkit".to_string()]));

        // Stale — manifest has a dep not in lockfile
        assert!(lock.is_stale(&["httpkit".to_string(), "missing".to_string()]));
    }

    #[test]
    fn test_lockfile_sort() {
        let mut lock = DooLock::new();
        lock.upsert(LockedPackage {
            name: "zzz".to_string(),
            version: "1.0.0".to_string(),
            hash: "sha256:z".to_string(),
            source: LockedSource::Registry,
            dependencies: vec![],
        });
        lock.upsert(LockedPackage {
            name: "aaa".to_string(),
            version: "1.0.0".to_string(),
            hash: "sha256:a".to_string(),
            source: LockedSource::Registry,
            dependencies: vec![],
        });

        lock.sort();
        assert_eq!(lock.packages[0].name, "aaa");
        assert_eq!(lock.packages[1].name, "zzz");
    }

    #[test]
    fn test_lockfile_parse_nonexistent() {
        let lock = DooLock::from_file(Path::new("/nonexistent/doo.lock")).unwrap();
        assert!(lock.is_empty());
    }

    #[test]
    fn test_lockfile_parse_invalid_version() {
        let toml_str = r#"
version = 999

[[package]]
name = "test"
version = "1.0.0"
hash = "sha256:abc"
dependencies = []
"#;
        let result = DooLock::from_str(toml_str);
        assert!(result.is_err());
    }
}
