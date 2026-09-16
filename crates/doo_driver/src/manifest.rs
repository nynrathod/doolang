//! doo.toml Manifest Parser
//!
//! Parses the project manifest file (doo.toml) that declares
//! package metadata, dependencies, library kind, and workspace members.
//!
//! ## No-Config Mode
//!
//! A single `main.doo` script with no `doo.toml` is valid.
//! The compiler uses default package name "main", version "0.0.0".

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ============================================================================
// Version
// ============================================================================

/// Semantic version: MAJOR.MINOR.PATCH
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// Parse "1.2.3" into Version { major: 1, minor: 2, patch: 3 }.
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(format!(
                "invalid version '{}': expected MAJOR.MINOR.PATCH",
                s
            ));
        }
        let major = parts[0]
            .parse::<u32>()
            .map_err(|_| format!("invalid major version in '{}'", s))?;
        let minor = parts[1]
            .parse::<u32>()
            .map_err(|_| format!("invalid minor version in '{}'", s))?;
        let patch = parts[2]
            .parse::<u32>()
            .map_err(|_| format!("invalid patch version in '{}'", s))?;
        Ok(Self {
            major,
            minor,
            patch,
        })
    }

    /// Check if this version satisfies a version requirement.
    pub fn satisfies(&self, req: &VersionReq) -> bool {
        if req.exact {
            return self.major == req.major && self.minor == req.minor && self.patch == req.patch;
        }
        // Caret range: >= req and < (req.major + 1).0.0
        if self.major != req.major {
            return false;
        }
        if self.minor < req.minor {
            return false;
        }
        if self.minor == req.minor && self.patch < req.patch {
            return false;
        }
        true
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Default for Version {
    fn default() -> Self {
        Self {
            major: 0,
            minor: 0,
            patch: 0,
        }
    }
}

// ============================================================================
// VersionReq
// ============================================================================

/// Version requirement from doo.toml.
///
/// - `"1.2.0"` = caret range (>= 1.2.0, < 2.0.0)
/// - `"=1.2.0"` = exact pin (only 1.2.0)
#[derive(Debug, Clone)]
pub struct VersionReq {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub exact: bool,
}

impl VersionReq {
    /// Parse a version requirement string.
    ///
    /// - `"1.2.0"` → caret range
    /// - `"=1.2.0"` → exact pin
    pub fn parse(s: &str) -> Result<Self, String> {
        let (exact, version_str) = if let Some(stripped) = s.strip_prefix('=') {
            (true, stripped)
        } else {
            (false, s)
        };

        let version = Version::parse(version_str)?;
        Ok(Self {
            major: version.major,
            minor: version.minor,
            patch: version.patch,
            exact,
        })
    }

    /// Check if a version satisfies this requirement.
    pub fn matches(&self, version: &Version) -> bool {
        version.satisfies(self)
    }
}

impl std::fmt::Display for VersionReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.exact {
            write!(f, "={}.{}.{}", self.major, self.minor, self.patch)
        } else {
            write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
        }
    }
}

// ============================================================================
// Dependency Source
// ============================================================================

/// Where a dependency comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencySource {
    /// Default — from central package registry.
    Registry,
    /// From a Git repository.
    Git { url: String, tag: Option<String> },
    /// From a local filesystem path.
    Path { path: PathBuf },
}

impl std::fmt::Display for DependencySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registry => write!(f, "registry"),
            Self::Git { url, tag } => {
                write!(f, "git+{}", url)?;
                if let Some(t) = tag {
                    write!(f, "#{}", t)?;
                }
                Ok(())
            }
            Self::Path { path } => write!(f, "path:{}", path.display()),
        }
    }
}

// ============================================================================
// Dependency
// ============================================================================

/// A dependency declared in doo.toml.
#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub source: DependencySource,
    pub version_req: VersionReq,
}

// ============================================================================
// Library Kind
// ============================================================================

/// Whether a crate is a normal library or a macro provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibKind {
    #[default]
    Lib,
    Macro,
}

impl LibKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "lib" => Some(Self::Lib),
            "macro" => Some(Self::Macro),
            _ => None,
        }
    }

    pub fn is_macro(self) -> bool {
        matches!(self, Self::Macro)
    }
}

// ============================================================================
// Manifest Structs
// ============================================================================

/// Package metadata from [package] in doo.toml.
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub name: String,
    pub version: Version,
    pub edition: String,
}

/// Library configuration from [lib] in doo.toml.
#[derive(Debug, Clone, Default)]
pub struct LibConfig {
    pub kind: LibKind,
}

/// Workspace configuration from [workspace] in doo.toml.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceConfig {
    pub members: Vec<PathBuf>,
}

/// The full doo.toml manifest.
#[derive(Debug, Clone)]
pub struct DooManifest {
    pub package: PackageInfo,
    pub dependencies: Vec<Dependency>,
    pub lib: Option<LibConfig>,
    pub workspace: Option<WorkspaceConfig>,
}

// ============================================================================
// TOML Deserialization Helpers
// ============================================================================

#[derive(Deserialize)]
struct TomlManifest {
    package: TomlPackage,
    #[serde(default)]
    dependencies: HashMap<String, TomlDep>,
    lib: Option<TomlLib>,
    workspace: Option<TomlWorkspace>,
}

#[derive(Deserialize)]
struct TomlPackage {
    name: String,
    version: String,
    edition: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TomlDep {
    Simple(String),
    Detailed(TomlDepDetail),
}

#[derive(Deserialize, Default)]
struct TomlDepDetail {
    version: Option<String>,
    git: Option<String>,
    tag: Option<String>,
    path: Option<String>,
}

#[derive(Deserialize)]
struct TomlLib {
    kind: Option<String>,
}

#[derive(Deserialize)]
struct TomlWorkspace {
    members: Vec<String>,
}

// ============================================================================
// Manifest Parsing
// ============================================================================

impl DooManifest {
    /// Parse doo.toml from a file path.
    ///
    /// Returns a default manifest if the file doesn't exist (no-config mode).
    pub fn from_file(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default_for_script());
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

        Self::from_str(&content)
    }

    /// Parse doo.toml from a string.
    pub fn from_str(content: &str) -> Result<Self, String> {
        let toml_manifest: TomlManifest =
            toml::from_str(content).map_err(|e| format!("Failed to parse doo.toml: {}", e))?;

        Self::from_toml(toml_manifest)
    }

    /// Convert the TOML deserialization struct to DooManifest.
    fn from_toml(toml: TomlManifest) -> Result<Self, String> {
        // Validate package name
        if toml.package.name.is_empty() {
            return Err("package name is required in doo.toml".to_string());
        }

        // Parse version
        let version = Version::parse(&toml.package.version)?;

        // Validate edition
        if toml.package.edition != "2026" {
            return Err(format!(
                "unsupported edition '{}' — only '2026' is supported",
                toml.package.edition
            ));
        }

        let package = PackageInfo {
            name: toml.package.name,
            version,
            edition: toml.package.edition,
        };

        // Parse dependencies
        let mut dependencies = Vec::new();
        for (name, dep) in &toml.dependencies {
            let dependency = match dep {
                TomlDep::Simple(version_str) => {
                    let version_req = VersionReq::parse(version_str)?;
                    Dependency {
                        name: name.clone(),
                        source: DependencySource::Registry,
                        version_req,
                    }
                }
                TomlDep::Detailed(detail) => {
                    let version_req = if let Some(v) = &detail.version {
                        VersionReq::parse(v)?
                    } else {
                        VersionReq::parse("0.0.0")?
                    };

                    let source = if let Some(git_url) = &detail.git {
                        DependencySource::Git {
                            url: git_url.clone(),
                            tag: detail.tag.clone(),
                        }
                    } else if let Some(path) = &detail.path {
                        DependencySource::Path {
                            path: PathBuf::from(path),
                        }
                    } else {
                        DependencySource::Registry
                    };

                    Dependency {
                        name: name.clone(),
                        source,
                        version_req,
                    }
                }
            };
            dependencies.push(dependency);
        }

        // Parse lib config
        let lib = toml.lib.map(|l| LibConfig {
            kind: l
                .kind
                .as_deref()
                .and_then(LibKind::from_str)
                .unwrap_or_default(),
        });

        // Parse workspace
        let workspace = toml.workspace.map(|w| WorkspaceConfig {
            members: w.members.into_iter().map(PathBuf::from).collect(),
        });

        Ok(Self {
            package,
            dependencies,
            lib,
            workspace,
        })
    }

    /// Create a default manifest for single-file scripts (no doo.toml).
    ///
    /// Package name is "main", version is "0.0.0", edition is "2026".
    pub fn default_for_script() -> Self {
        Self {
            package: PackageInfo {
                name: "main".to_string(),
                version: Version::default(),
                edition: "2026".to_string(),
            },
            dependencies: Vec::new(),
            lib: None,
            workspace: None,
        }
    }

    /// Check if this manifest has any dependencies.
    pub fn has_dependencies(&self) -> bool {
        !self.dependencies.is_empty()
    }

    /// Check if this package is a macro provider.
    pub fn is_macro_crate(&self) -> bool {
        self.lib
            .as_ref()
            .map(|l| l.kind.is_macro())
            .unwrap_or(false)
    }

    /// Get all dependency names.
    pub fn dependency_names(&self) -> Vec<&str> {
        self.dependencies.iter().map(|d| d.name.as_str()).collect()
    }

    /// Find a dependency by name.
    pub fn get_dependency(&self, name: &str) -> Option<&Dependency> {
        self.dependencies.iter().find(|d| d.name == name)
    }
}

impl Default for DooManifest {
    fn default() -> Self {
        Self::default_for_script()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parse() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn test_version_parse_invalid() {
        assert!(Version::parse("1.2").is_err());
        assert!(Version::parse("1.2.3.4").is_err());
        assert!(Version::parse("abc").is_err());
    }

    #[test]
    fn test_version_display() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(format!("{}", v), "1.2.3");
    }

    #[test]
    fn test_version_satisfies_caret() {
        let req = VersionReq::parse("1.2.0").unwrap();
        assert!(!req.exact);
        assert!(Version::parse("1.2.0").unwrap().satisfies(&req));
        assert!(Version::parse("1.2.5").unwrap().satisfies(&req));
        assert!(Version::parse("1.3.0").unwrap().satisfies(&req));
        assert!(!Version::parse("1.1.0").unwrap().satisfies(&req));
        assert!(!Version::parse("2.0.0").unwrap().satisfies(&req));
    }

    #[test]
    fn test_version_satisfies_exact() {
        let req = VersionReq::parse("=1.2.3").unwrap();
        assert!(req.exact);
        assert!(Version::parse("1.2.3").unwrap().satisfies(&req));
        assert!(!Version::parse("1.2.4").unwrap().satisfies(&req));
        assert!(!Version::parse("1.3.0").unwrap().satisfies(&req));
    }

    #[test]
    fn test_manifest_parse_basic() {
        let toml_str = r#"
[package]
name = "testPackage"
version = "0.1.0"
edition = "2026"
"#;
        let manifest = DooManifest::from_str(toml_str).unwrap();
        assert_eq!(manifest.package.name, "testPackage");
        assert_eq!(manifest.package.version.major, 0);
        assert_eq!(manifest.package.version.minor, 1);
        assert_eq!(manifest.package.edition, "2026");
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn test_manifest_parse_with_dependencies() {
        let toml_str = r#"
[package]
name = "myApp"
version = "1.0.0"
edition = "2026"

[dependencies]
httpkit = "1.2.0"
localutils = { path = "../utils" }
"#;
        let manifest = DooManifest::from_str(toml_str).unwrap();
        assert_eq!(manifest.dependencies.len(), 2);
        assert!(manifest.has_dependencies());

        let dep = manifest.get_dependency("httpkit").unwrap();
        assert_eq!(dep.name, "httpkit");
        assert!(matches!(dep.source, DependencySource::Registry));

        let dep = manifest.get_dependency("localutils").unwrap();
        assert_eq!(dep.name, "localutils");
        assert!(matches!(dep.source, DependencySource::Path { .. }));
    }

    #[test]
    fn test_manifest_parse_with_git_dep() {
        let toml_str = r#"
[package]
name = "myApp"
version = "1.0.0"
edition = "2026"

[dependencies]
authlib = { git = "https://github.com/org/authlib", tag = "v2.0.0" }
"#;
        let manifest = DooManifest::from_str(toml_str).unwrap();
        let dep = manifest.get_dependency("authlib").unwrap();
        match &dep.source {
            DependencySource::Git { url, tag } => {
                assert_eq!(url, "https://github.com/org/authlib");
                assert_eq!(tag.as_deref(), Some("v2.0.0"));
            }
            _ => panic!("expected Git source"),
        }
    }

    #[test]
    fn test_manifest_parse_macro_lib() {
        let toml_str = r#"
[package]
name = "doo-derive-json"
version = "0.1.0"
edition = "2026"

[lib]
kind = "macro"
"#;
        let manifest = DooManifest::from_str(toml_str).unwrap();
        assert!(manifest.is_macro_crate());
    }

    #[test]
    fn test_manifest_parse_workspace() {
        let toml_str = r#"
[workspace]
members = ["services/userApi", "services/orderApi"]
"#;
        // Workspace-only manifest needs a [package] section
        let toml_str = format!(
            r#"{}
[package]
name = "workspace-root"
version = "0.0.0"
edition = "2026"
"#,
            toml_str
        );
        let manifest = DooManifest::from_str(&toml_str).unwrap();
        assert!(manifest.workspace.is_some());
        let ws = manifest.workspace.unwrap();
        assert_eq!(ws.members.len(), 2);
    }

    #[test]
    fn test_manifest_no_config_mode() {
        let manifest = DooManifest::default_for_script();
        assert_eq!(manifest.package.name, "main");
        assert_eq!(manifest.package.version.major, 0);
        assert!(!manifest.has_dependencies());
    }

    #[test]
    fn test_manifest_missing_name() {
        let toml_str = r#"
[package]
version = "0.1.0"
edition = "2026"
"#;
        // This will fail because name is empty (toml will set it to "")
        let result = DooManifest::from_str(toml_str);
        // toml::from_str will fail because name is required
        assert!(result.is_err());
    }

    #[test]
    fn test_manifest_invalid_edition() {
        let toml_str = r#"
[package]
name = "test"
version = "0.1.0"
edition = "2025"
"#;
        let result = DooManifest::from_str(toml_str);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("2026"));
    }

    #[test]
    fn test_manifest_exact_version() {
        let toml_str = r#"
[package]
name = "app"
version = "1.0.0"
edition = "2026"

[dependencies]
pinned = "=2.3.4"
"#;
        let manifest = DooManifest::from_str(toml_str).unwrap();
        let dep = manifest.get_dependency("pinned").unwrap();
        assert!(dep.version_req.exact);
    }

    #[test]
    fn test_dependency_names() {
        let toml_str = r#"
[package]
name = "app"
version = "1.0.0"
edition = "2026"

[dependencies]
aaa = "1.0.0"
bbb = { path = "../bbb" }
"#;
        let manifest = DooManifest::from_str(toml_str).unwrap();
        let names = manifest.dependency_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"aaa"));
        assert!(names.contains(&"bbb"));
    }
}
