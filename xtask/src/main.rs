use std::env;
use std::fs;
use std::process::{exit, Command};

fn main() {
    let task = env::args().nth(1);
    match task.as_deref() {
        Some("build") => build(false),
        Some("release") => build(true),
        Some("clean") => clean(),
        _ => {
            eprintln!("Usage: cargo xtask [build|release|clean]");
            eprintln!("");
            eprintln!("Commands:");
            eprintln!("  build    - Build in debug mode");
            eprintln!("  release  - Build in release mode (recommended)");
            eprintln!("  clean    - Clean all build artifacts");
            exit(1);
        }
    }
}

fn build(release: bool) {
    let mode = if release { "release" } else { "debug" };
    println!("🔨 Building doo compiler ({})...", mode);

    // Build the entire workspace (includes main doo + libdoo_file)
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("--workspace");

    if release {
        cmd.arg("--release");
    }

    let status = cmd.status().expect("Failed to run cargo build");
    if !status.success() {
        eprintln!("❌ Build failed!");
        exit(1);
    }

    // Copy FFI libraries from deps to target root for easier runtime discovery
    println!("📦 Copying FFI libraries...");

    let workspace_root = env::current_dir().expect("Failed to get current directory");
    let target_dir = workspace_root.join("target").join(mode);
    let deps_dir = target_dir.join("deps");

    // Platform-specific library extension
    #[cfg(target_os = "windows")]
    let lib_pattern = "doo_file.dll";

    #[cfg(target_os = "linux")]
    let lib_pattern = "libdoo_file.so";

    #[cfg(target_os = "macos")]
    let lib_pattern = "libdoo_file.dylib";

    // Find and copy the FFI library
    if let Ok(entries) = fs::read_dir(&deps_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(filename) = path.file_name() {
                let filename_str = filename.to_string_lossy();
                if filename_str.contains(lib_pattern) && !filename_str.ends_with(".d") {
                    let dest = target_dir.join(filename);
                    if let Err(e) = fs::copy(&path, &dest) {
                        eprintln!("Warning: Failed to copy {}: {}", filename_str, e);
                    }
                }
            }
        }
    }

    println!("✅ Build complete! Files in target/{}/", mode);
    println!("");
    println!("Add to PATH:");

    #[cfg(not(target_os = "windows"))]
    println!("  export PATH=\"{}:$PATH\"", target_dir.display());

    #[cfg(target_os = "windows")]
    println!("  $env:Path = \"{};$env:Path\"", target_dir.display());
}

fn clean() {
    println!("🧹 Cleaning build artifacts...");

    let status = Command::new("cargo")
        .arg("clean")
        .status()
        .expect("Failed to run cargo clean");

    if !status.success() {
        eprintln!("❌ Clean failed!");
        exit(1);
    }

    println!("✅ Clean complete!");
}
