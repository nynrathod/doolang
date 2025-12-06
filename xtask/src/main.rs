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
    let lib_patterns = vec!["libdoo.rlib", "libdoo_file.rlib"];

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
                        // Check if file has actual size (not empty placeholder)
                        if let Ok(metadata) = fs::metadata(&path) {
                            if metadata.len() > 0 {
                                let dest = target_dir.join(filename);
                                if let Err(e) = fs::copy(&path, &dest) {
                                    eprintln!("Warning: Failed to copy {}: {}", filename_str, e);
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
    }

    println!("✅ Build complete! Files in target/{}/", mode);
    println!("");

    // Platform-specific installation
    #[cfg(target_os = "windows")]
    {
        println!("📦 Windows Setup:");
        println!("Add target/release to PATH:");
        println!("  $env:Path = \"{};$env:Path\"", target_dir.display());
    }

    #[cfg(not(target_os = "windows"))]
    {
        if release {
            install_unix(&target_dir, &workspace_root);
        } else {
            println!("Add to PATH:");
            println!("  export PATH=\"{}:$PATH\"", target_dir.display());
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn install_unix(target_dir: &PathBuf, workspace_root: &PathBuf) {
    println!("\n📦 Installing to ~/.local/bin/doo/...");

    let home = env::var("HOME").expect("HOME environment variable not set");
    let local_bin = PathBuf::from(&home).join(".local").join("bin");
    let doo_dir = local_bin.join("doo");

    // Create doo directory
    if let Err(e) = fs::create_dir_all(&doo_dir) {
        eprintln!("❌ Failed to create {}: {}", doo_dir.display(), e);
        exit(1);
    }

    // Copy doo binary
    let doo_binary = target_dir.join("doo");
    let dest_binary = doo_dir.join("doo");

    if !doo_binary.exists() {
        eprintln!("❌ doo binary not found at {}", doo_binary.display());
        exit(1);
    }

    match fs::copy(&doo_binary, &dest_binary) {
        Ok(_) => {
            println!("  ✓ Installed doo binary");

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
        Err(e) => {
            eprintln!("❌ Failed to copy doo binary: {}", e);
            exit(1);
        }
    }

    // Copy FFI libraries
    #[cfg(target_os = "linux")]
    let lib_names = vec!["libdoo.rlib", "libdoo_file.rlib"];

    #[cfg(target_os = "macos")]
    let lib_names = vec!["libdoo.dylib", "libdoo_file.dylib"];

    for lib_name in lib_names {
        let lib_file = target_dir.join(lib_name);
        let dest_lib = doo_dir.join(lib_name);

        if lib_file.exists() {
            // Check file has actual size before copying
            if let Ok(metadata) = fs::metadata(&lib_file) {
                if metadata.len() == 0 {
                    eprintln!(
                        "❌ {} is empty (0 bytes). Build may have failed on Windows filesystem.",
                        lib_name
                    );
                    eprintln!("💡 Try building inside WSL native filesystem instead of /mnt");
                    exit(1);
                }
            }

            match fs::copy(&lib_file, &dest_lib) {
                Ok(_) => println!("  ✓ Installed {}", lib_name),
                Err(e) => {
                    eprintln!("❌ Failed to copy {}: {}", lib_name, e);
                    exit(1);
                }
            }
        } else {
            eprintln!("❌ {} not found in target directory", lib_name);
            exit(1);
        }
    }

    // Copy std directory
    let std_dir = workspace_root.join("std");
    let dest_std = doo_dir.join("std");

    if !std_dir.exists() {
        eprintln!("❌ std directory not found at {}", std_dir.display());
        exit(1);
    }

    // Remove old std directory if exists
    let _ = fs::remove_dir_all(&dest_std);

    match copy_dir_recursive(&std_dir, &dest_std) {
        Ok(_) => println!("  ✓ Installed std library"),
        Err(e) => {
            eprintln!("❌ Failed to copy std directory: {}", e);
            exit(1);
        }
    }

    println!("\n✅ Installation complete!");
    println!("📁 Installed to: {}", doo_dir.display());

    // Check and guide PATH setup
    let path_var = env::var("PATH").unwrap_or_default();
    let doo_dir_str = doo_dir.to_string_lossy();

    if !path_var.contains(doo_dir_str.as_ref()) {
        println!("\n⚠️  Add doo to your PATH:");
        println!("  echo 'export PATH=\"$HOME/.local/bin/doo:$PATH\"' >> ~/.bashrc");
        println!("  echo 'export PATH=\"$HOME/.local/bin/doo:$PATH\"' >> ~/.zshrc");
        println!("\nOr for current session:");
        println!("  export PATH=\"$HOME/.local/bin/doo:$PATH\"");
    } else {
        println!("\n✓ doo is already in your PATH");
    }

    println!("\n💡 Verify installation:");
    println!("  doo --version");
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
