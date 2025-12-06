use std::env;
use std::fs;
use std::process::{exit, Command};

#[cfg(not(target_os = "windows"))]
use std::path::PathBuf;

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

    // Platform-specific library patterns (both libdoo and libdoo_file)
    #[cfg(target_os = "windows")]
    let lib_patterns = vec!["doo.dll", "doo_file.dll"];

    #[cfg(target_os = "linux")]
    let lib_patterns = vec!["libdoo.so", "libdoo_file.so"];

    #[cfg(target_os = "macos")]
    let lib_patterns = vec!["libdoo.dylib", "libdoo_file.dylib"];

    // Find and copy the FFI libraries
    if let Ok(entries) = fs::read_dir(&deps_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(filename) = path.file_name() {
                let filename_str = filename.to_string_lossy();
                for lib_pattern in &lib_patterns {
                    if filename_str.contains(lib_pattern) && !filename_str.ends_with(".d") {
                        let dest = target_dir.join(filename);
                        if let Err(e) = fs::copy(&path, &dest) {
                            eprintln!("Warning: Failed to copy {}: {}", filename_str, e);
                        }
                        break;
                    }
                }
            }
        }
    }

    println!("✅ Build complete! Files in target/{}/", mode);
    println!("");

    // Install to system path on Linux/macOS
    #[cfg(not(target_os = "windows"))]
    {
        if release {
            install_to_system(target_dir.clone(), &workspace_root);
        }
    }

    // Show PATH instructions
    #[cfg(target_os = "windows")]
    {
        println!("Add to PATH:");
        println!("  $env:Path = \"{};$env:Path\"", target_dir.display());
    }

    #[cfg(not(target_os = "windows"))]
    {
        if !release {
            println!("Add to PATH:");
            println!("  export PATH=\"{}:$PATH\"", target_dir.display());
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn install_to_system(target_dir: PathBuf, workspace_root: &PathBuf) {
    println!("\n📦 Installing to ~/.local/bin...");

    let home = env::var("HOME").expect("HOME environment variable not set");
    let local_bin = PathBuf::from(&home).join(".local").join("bin");

    // Create directory
    if let Err(e) = fs::create_dir_all(&local_bin) {
        eprintln!("Warning: Failed to create {}: {}", local_bin.display(), e);
        return;
    }

    // Copy doo binary
    let doo_binary = target_dir.join("doo");
    let dest_binary = local_bin.join("doo");

    if doo_binary.exists() {
        match fs::copy(&doo_binary, &dest_binary) {
            Ok(_) => {
                println!("  ✓ Installed doo binary to {}", dest_binary.display());

                // Make executable
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(metadata) = fs::metadata(&dest_binary) {
                        let mut perms = metadata.permissions();
                        perms.set_mode(0o755);
                        let _ = fs::set_permissions(&dest_binary, perms);
                    }
                }
            }
            Err(e) => eprintln!("Warning: Failed to copy doo binary: {}", e),
        }
    }

    // Copy FFI libraries (both libdoo and libdoo_file)
    #[cfg(target_os = "linux")]
    let lib_names = vec!["libdoo.so", "libdoo_file.so"];

    #[cfg(target_os = "macos")]
    let lib_names = vec!["libdoo.dylib", "libdoo_file.dylib"];

    for lib_name in lib_names {
        let lib_file = target_dir.join(lib_name);
        let dest_lib = local_bin.join(lib_name);

        if lib_file.exists() {
            match fs::copy(&lib_file, &dest_lib) {
                Ok(_) => println!("  ✓ Installed {} to {}", lib_name, dest_lib.display()),
                Err(e) => eprintln!("Warning: Failed to copy {}: {}", lib_name, e),
            }
        } else {
            eprintln!("Warning: {} not found in target directory", lib_name);
        }
    }

    // Copy std directory
    let std_dir = workspace_root.join("std");
    let dest_std = local_bin.join("std");

    if std_dir.exists() {
        // Remove old std directory if exists
        let _ = fs::remove_dir_all(&dest_std);

        match copy_dir_recursive(&std_dir, &dest_std) {
            Ok(_) => println!("  ✓ Installed std library to {}", dest_std.display()),
            Err(e) => eprintln!("Warning: Failed to copy std directory: {}", e),
        }
    }

    // Check if ~/.local/bin is in PATH
    let path_var = env::var("PATH").unwrap_or_default();
    let local_bin_str = local_bin.to_string_lossy();

    if !path_var.contains(local_bin_str.as_ref()) {
        println!("\n⚠️  Add ~/.local/bin to your PATH:");
        println!("  echo 'export PATH=\"$HOME/.local/bin:$PATH\"' >> ~/.bashrc");
        println!("  echo 'export PATH=\"$HOME/.local/bin:$PATH\"' >> ~/.zshrc");
        println!("\nOr for current session:");
        println!("  export PATH=\"$HOME/.local/bin:$PATH\"");
    }

    // Set DOO_STD_PATH environment variable hint
    println!("\n💡 The compiler will look for std library at:");
    println!("  1. $DOO_STD_PATH (if set)");
    println!("  2. ~/.local/bin/std (installed location)");
    println!("  3. ./std (relative to binary)");

    println!("\n✅ Installation complete!");
}

#[cfg(not(target_os = "windows"))]
fn copy_dir_recursive(src: &PathBuf, dest: &PathBuf) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            fs::copy(&src_path, &dest_path)?;
        }
    }

    Ok(())
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
