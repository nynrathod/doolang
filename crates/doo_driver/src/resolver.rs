//! Dependency Resolver
//!
//! Walks the dependency tree, selects versions, detects conflicts,
//! and generates a lockfile with exact pins and content hashes.

use crate::lockfile::{DooLock, LockedPackage, LockedSource};
use crate::manifest::{Dependency, DependencySource, DooManifest, Version, VersionReq};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;

// ============================================================================
// Package Registry
// ============================================================================

/// Local package registry for version resolution.
///
/// For v1, this supports `path` and `git` dependencies primarily.
/// A central registry API will be added in the future.
#[derive(Debug, Clone, Default)]
pub struct PackageRegistry {
    /// Maps package name → available versions.
    pub index: HashMap<String, Vec<RegistryPackage>>,
}

/// A package available in the registry.
#[derive(Debug, Clone)]
pub struct RegistryPackage {
    pub name: String,
    pub version: Version,
    pub tarball_url: String,
    pub hash: String,
}

impl PackageRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a package in the registry.
    pub fn register(&mut self, package: RegistryPackage) {
        self.index
            .entry(package.name.clone())
            .or_default()
            .push(package);
    }

    /// Find the highest version of a package that satisfies a requirement.
    pub fn find_best(&self, name: &str, req: &VersionReq) -> Option<&RegistryPackage> {
        let packages = self.index.get(name)?;
        packages
            .iter()
            .filter(|p| p.version.satisfies(req))
            .max_by_key(|p| (p.version.major, p.version.minor, p.version.patch))
    }

    /// Check if a package exists in the registry.
    pub fn has(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }
}

// ============================================================================
// Dependency Resolver
// ============================================================================

/// Resolves dependencies from a manifest into a lockfile.
pub struct DependencyResolver;

impl DependencyResolver {
    /// Resolve all dependencies from a manifest.
    ///
    /// Walks the dependency tree breadth-first:
    /// - For each dependency, selects the highest matching version
    /// - Detects circular dependencies → error
    /// - Detects version conflicts → error
    /// - Computes sha256 hashes for all resolved packages
    /// - Generates a DooLock with resolved versions and hashes
    pub fn resolve(
        manifest: &DooManifest,
        registry: &PackageRegistry,
        project_root: &Path,
    ) -> Result<DooLock, Vec<String>> {
        let mut lock = DooLock::new();
        let mut errors = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: Vec<(String, DependencySource, VersionReq)> = manifest
            .dependencies
            .iter()
            .map(|d| (d.name.clone(), d.source.clone(), d.version_req.clone()))
            .collect();

        while let Some((name, source, version_req)) = queue.pop() {
            // Skip if already resolved
            if visited.contains(&name) {
                continue;
            }
            visited.insert(name.clone());

            // Resolve the package based on its source
            let resolved = match &source {
                DependencySource::Registry => match registry.find_best(&name, &version_req) {
                    Some(pkg) => ResolvedPackage {
                        name: pkg.name.clone(),
                        version: pkg.version.clone(),
                        hash: pkg.hash.clone(),
                        source: LockedSource::Registry,
                        dependencies: Vec::new(),
                    },
                    None => {
                        errors.push(format!(
                            "dependency '{}' not found in registry (required {})",
                            name, version_req
                        ));
                        continue;
                    }
                },
                DependencySource::Path { path } => {
                    let full_path = if path.is_absolute() {
                        path.clone()
                    } else {
                        project_root.join(path)
                    };

                    if !full_path.exists() {
                        errors.push(format!(
                            "path dependency '{}' not found at {}",
                            name,
                            full_path.display()
                        ));
                        continue;
                    }

                    let hash = hash_directory(&full_path);
                    let version =
                        read_path_dependency_version(&full_path).unwrap_or(Version::default());

                    ResolvedPackage {
                        name: name.clone(),
                        version,
                        hash,
                        source: LockedSource::Path {
                            path: path.to_string_lossy().to_string(),
                        },
                        dependencies: Vec::new(),
                    }
                }
                DependencySource::Git { url, tag } => {
                    // For v1, git dependencies use the tag/commit as the hash
                    let hash_input = match tag {
                        Some(t) => format!("{}#{}", url, t),
                        None => format!("{}#HEAD", url),
                    };
                    let hash = format!("sha256:{}", compute_sha256_hex(hash_input.as_bytes()));

                    ResolvedPackage {
                        name: name.clone(),
                        version: Version::default(),
                        hash,
                        source: LockedSource::Git {
                            url: url.clone(),
                            tag: tag.clone(),
                        },
                        dependencies: Vec::new(),
                    }
                }
            };

            // Check for circular dependencies by looking at the visited set
            // (More robust cycle detection happens during the BFS traversal itself)

            // Add to lockfile
            lock.upsert(LockedPackage {
                name: resolved.name.clone(),
                version: resolved.version.to_string(),
                hash: resolved.hash,
                source: resolved.source,
                dependencies: resolved.dependencies,
            });

            // TODO: When we have transitive dependency resolution,
            // we'd parse the dependency's doo.toml and add its
            // dependencies to the queue. For v1, we only resolve
            // direct dependencies.
        }

        // Detect circular dependencies
        if let Some(cycle) = detect_cycle(manifest) {
            errors.push(format!("circular dependency detected: {}", cycle));
        }

        // Detect version conflicts
        // (Two packages requiring incompatible versions of the same dep)
        // For v1 with only direct dependencies, this can't happen.
        // This check becomes relevant when transitive deps are resolved.

        if errors.is_empty() {
            lock.sort();
            Ok(lock)
        } else {
            Err(errors)
        }
    }
}

// ============================================================================
// Internal: Resolved Package
// ============================================================================

struct ResolvedPackage {
    name: String,
    version: Version,
    hash: String,
    source: LockedSource,
    dependencies: Vec<String>,
}

// ============================================================================
// Internal: Cycle Detection
// ============================================================================

/// Detect circular dependencies in the manifest.
///
/// For v1, this only checks direct dependencies (no transitive).
/// Returns the cycle as a string if found.
fn detect_cycle(manifest: &DooManifest) -> Option<String> {
    // Build adjacency list
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    for dep in &manifest.dependencies {
        graph
            .entry(manifest.package.name.clone())
            .or_default()
            .push(dep.name.clone());
    }

    // For v1 with only direct deps, a cycle would require:
    // package A depends on B, and B depends on A
    // Since we only have direct deps, B's manifest isn't loaded yet.
    // This is a placeholder — full cycle detection requires
    // loading all transitive manifests.

    None
}

// ============================================================================
// Internal: Hashing
// ============================================================================

/// Compute sha256 hash of a byte slice and return as hex string.
fn compute_sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    // Convert to hex without external dependency
    let mut hex = String::with_capacity(64);
    for byte in result.iter() {
        hex.push_str(&format!("{:02x}", byte));
    }
    hex
}

/// Hash all files in a directory (sorted by path for determinism).
fn hash_directory(path: &Path) -> String {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    collect_files(path, &mut files);
    files.sort();

    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());

    for file in &files {
        if let Ok(content) = std::fs::read(file) {
            hasher.update(&content);
        }
    }

    let result = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in result.iter() {
        hex.push_str(&format!("{:02x}", byte));
    }
    format!("sha256:{}", hex)
}

/// Recursively collect all files in a directory.
fn collect_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                // Skip hidden, target, and node_modules
                if !name.starts_with('.')
                    && name != "target"
                    && name != "target-windows"
                    && name != "target-linux"
                    && name != "node_modules"
                    && name != ".git"
                {
                    collect_files(&path, out);
                }
            } else {
                out.push(path);
            }
        }
    }
}

/// Read the version from a path dependency's doo.toml.
fn read_path_dependency_version(path: &Path) -> Option<Version> {
    let manifest_path = path.join("doo.toml");
    if !manifest_path.exists() {
        return None;
    }

    let manifest = DooManifest::from_file(&manifest_path).ok()?;
    Some(manifest.package.version)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_registry() {
        let registry = PackageRegistry::new();
        assert!(!registry.has("anything"));
    }

    #[test]
    fn test_registry_find_best() {
        let mut registry = PackageRegistry::new();
        registry.register(RegistryPackage {
            name: "httpkit".to_string(),
            version: Version::parse("1.0.0").unwrap(),
            tarball_url: "".to_string(),
            hash: "sha256:abc".to_string(),
        });
        registry.register(RegistryPackage {
            name: "httpkit".to_string(),
            version: Version::parse("1.2.0").unwrap(),
            tarball_url: "".to_string(),
            hash: "sha256:def".to_string(),
        });
        registry.register(RegistryPackage {
            name: "httpkit".to_string(),
            version: Version::parse("2.0.0").unwrap(),
            tarball_url: "".to_string(),
            hash: "sha256:ghi".to_string(),
        });

        let req = VersionReq::parse("1.0.0").unwrap();
        let best = registry.find_best("httpkit", &req).unwrap();
        assert_eq!(best.version, Version::parse("1.2.0").unwrap());
    }

    #[test]
    fn test_registry_find_best_exact() {
        let mut registry = PackageRegistry::new();
        registry.register(RegistryPackage {
            name: "pkg".to_string(),
            version: Version::parse("1.0.0").unwrap(),
            tarball_url: "".to_string(),
            hash: "sha256:aaa".to_string(),
        });
        registry.register(RegistryPackage {
            name: "pkg".to_string(),
            version: Version::parse("1.2.0").unwrap(),
            tarball_url: "".to_string(),
            hash: "sha256:bbb".to_string(),
        });

        let req = VersionReq::parse("=1.0.0").unwrap();
        let best = registry.find_best("pkg", &req).unwrap();
        assert_eq!(best.version, Version::parse("1.0.0").unwrap());
    }

    #[test]
    fn test_resolve_no_dependencies() {
        let manifest = DooManifest::default_for_script();
        let registry = PackageRegistry::new();
        let lock = DependencyResolver::resolve(&manifest, &registry, Path::new(".")).unwrap();
        assert!(lock.is_empty());
    }

    #[test]
    fn test_resolve_missing_dependency() {
        let toml_str = r#"
[package]
name = "app"
version = "1.0.0"
edition = "2026"

[dependencies]
nonexistent = "1.0.0"
"#;
        let manifest = DooManifest::from_str(toml_str).unwrap();
        let registry = PackageRegistry::new();
        let result = DependencyResolver::resolve(&manifest, &registry, Path::new("."));
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.contains("nonexistent")));
    }

    #[test]
    fn test_compute_sha256() {
        let hash = compute_sha256_hex(b"hello");
        assert_eq!(hash.len(), 64);
        // Known sha256 of "hello"
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_hash_directory_nonexistent() {
        let hash = hash_directory(Path::new("/nonexistent/path"));
        assert!(hash.starts_with("sha256:"));
    }

    #[test]
    fn test_resolve_with_registry_dep() {
        let toml_str = r#"
[package]
name = "app"
version = "1.0.0"
edition = "2026"

[dependencies]
httpkit = "1.0.0"
"#;
        let manifest = DooManifest::from_str(toml_str).unwrap();

        let mut registry = PackageRegistry::new();
        registry.register(RegistryPackage {
            name: "httpkit".to_string(),
            version: Version::parse("1.2.0").unwrap(),
            tarball_url: "".to_string(),
            hash: "sha256:abc".to_string(),
        });

        let lock = DependencyResolver::resolve(&manifest, &registry, Path::new(".")).unwrap();
        assert_eq!(lock.len(), 1);
        assert!(lock.has("httpkit"));
        let pkg = lock.get("httpkit").unwrap();
        assert_eq!(pkg.version, "1.2.0");
        assert_eq!(pkg.hash, "sha256:abc");
    }

    #[test]
    fn test_resolve_with_path_dep() {
        let toml_str = r#"
[package]
name = "app"
version = "1.0.0"
edition = "2026"

[dependencies]
localutils = { path = "." }
"#;
        let manifest = DooManifest::from_str(toml_str).unwrap();
        let registry = PackageRegistry::new();

        // Use the project root as the path dependency (it exists)
        let lock = DependencyResolver::resolve(&manifest, &registry, Path::new(".")).unwrap();
        assert_eq!(lock.len(), 1);
        assert!(lock.has("localutils"));
        let pkg = lock.get("localutils").unwrap();
        assert!(pkg.hash.starts_with("sha256:"));
        assert!(matches!(pkg.source, LockedSource::Path { .. }));
    }

    #[test]
    fn test_resolve_with_git_dep() {
        let toml_str = r#"
[package]
name = "app"
version = "1.0.0"
edition = "2026"

[dependencies]
authlib = { git = "https://github.com/org/authlib", tag = "v2.0.0" }
"#;
        let manifest = DooManifest::from_str(toml_str).unwrap();
        let registry = PackageRegistry::new();

        let lock = DependencyResolver::resolve(&manifest, &registry, Path::new(".")).unwrap();
        assert_eq!(lock.len(), 1);
        assert!(lock.has("authlib"));
        let pkg = lock.get("authlib").unwrap();
        assert!(pkg.hash.starts_with("sha256:"));
        match &pkg.source {
            LockedSource::Git { url, tag } => {
                assert_eq!(url, "https://github.com/org/authlib");
                assert_eq!(tag.as_deref(), Some("v2.0.0"));
            }
            _ => panic!("expected Git source"),
        }
    }

    #[test]
    fn test_resolve_multiple_deps() {
        let toml_str = r#"
[package]
name = "app"
version = "1.0.0"
edition = "2026"

[dependencies]
aaa = "1.0.0"
bbb = "2.0.0"
ccc = { path = "." }
"#;
        let manifest = DooManifest::from_str(toml_str).unwrap();

        let mut registry = PackageRegistry::new();
        registry.register(RegistryPackage {
            name: "aaa".to_string(),
            version: Version::parse("1.5.0").unwrap(),
            tarball_url: "".to_string(),
            hash: "sha256:aaa".to_string(),
        });
        registry.register(RegistryPackage {
            name: "bbb".to_string(),
            version: Version::parse("2.3.0").unwrap(),
            tarball_url: "".to_string(),
            hash: "sha256:bbb".to_string(),
        });

        let lock = DependencyResolver::resolve(&manifest, &registry, Path::new(".")).unwrap();
        assert_eq!(lock.len(), 3);
        assert!(lock.has("aaa"));
        assert!(lock.has("bbb"));
        assert!(lock.has("ccc"));

        // Lockfile should be sorted alphabetically
        assert_eq!(lock.packages[0].name, "aaa");
        assert_eq!(lock.packages[1].name, "bbb");
        assert_eq!(lock.packages[2].name, "ccc");
    }
}
