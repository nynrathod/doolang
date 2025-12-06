use std::env;
use std::fs;
use std::path::PathBuf;
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

/// Check if running in WSL on Windows filesystem (/mnt)
#[cfg(target_os = "linux")]
fn check_wsl_on_windows_fs() {
    let cwd = env::current_dir().unwrap_or_default();
    let cwd_str = cwd.to_string_lossy();

    if cwd_str.starts_with("/mnt/") {
        // Double check it's WSL by looking for WSL-specific files
        let is_wsl = PathBuf::from("/proc/sys/fs/binfmt_misc/WSLInterop").exists()
            || PathBuf::from("/run/WSL").exists()
            || env::var("WSL_DISTRO_NAME").is_ok();

        if is_wsl {
            eprintln!("❌ Cannot build on Windows filesystem (/mnt) in WSL!");
            eprintln!("   This creates empty/corrupt .so files.\n");
            eprintln!("🔧 SOLUTION - Build on actual Linux or use Windows build:\n");
            eprintln!("Option 1 - Use Windows build (RECOMMENDED FOR DEVELOPMENT):");
            eprintln!("   Open PowerShell in Windows and run:");
            eprintln!("   cargo xtask release\n");
            eprintln!("Option 2 - Build in WSL native filesystem:");
            eprintln!("   cp -r /mnt/x/Projects/doo ~/doo");
            eprintln!("   cd ~/doo");
            eprintln!("   cargo xtask release\n");
            eprintln!("Option 3 - Build on actual Linux machine (for production)");
            exit(1);
        }
    }
}

fn build(release: bool) {
    // Check for WSL on Windows filesystem and exit with instructions
    #[cfg(target_os = "linux")]
    check_wsl_on_windows_fs();

    let mode = if release { "release" } else { "debug" };
    println!("🔨 Building doo compiler ({})...", mode);

    // Build the entire workspace
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

    let workspace_root = env::current_dir().expect("Failed to get current directory");
    let target_dir = workspace_root.join("target").join(mode);

    // Copy FFI libraries from deps to target root
    println!("📦 Copying FFI libraries...");
    copy_ffi_libraries(&target_dir);

    println!("✅ Build complete! Files in target/{}/", mode);
    println!("");

    // Platform-specific installation
    #[cfg(target_os = "windows")]
    {
        copy_windows_libs(&target_dir);
        install_windows(&target_dir);
    }

    #[cfg(target_os = "linux")]
    {
        if release {
            install_unix_from_path(&target_dir, &workspace_root);
        } else {
            println!("Add to PATH:");
            println!("  export PATH=\"{}:$PATH\"", target_dir.display());
        }
    }

    #[cfg(target_os = "macos")]
    {
        if release {
            install_unix_from_path(&target_dir, &workspace_root);
        } else {
            println!("Add to PATH:");
            println!("  export PATH=\"{}:$PATH\"", target_dir.display());
        }
    }
}

/// Copy FFI libraries from deps to target directory
fn copy_ffi_libraries(target_dir: &PathBuf) {
    let deps_dir = target_dir.join("deps");

    #[cfg(target_os = "windows")]
    let patterns: Vec<(&str, &str)> = vec![
        ("doo.dll", "doo.dll"),
        ("doo_file.dll", "doo_file.dll"),
        ("doo.dll.lib", "doo.dll.lib"),
        ("doo_file.dll.lib", "doo_file.dll.lib"),
    ];

    #[cfg(target_os = "linux")]
    let patterns: Vec<(&str, &str)> = vec![
        ("libdoo.so", "libdoo.so"),
        ("libdoo_file.so", "libdoo_file.so"),
    ];

    #[cfg(target_os = "macos")]
    let patterns: Vec<(&str, &str)> = vec![
        ("libdoo.dylib", "libdoo.dylib"),
        ("libdoo_file.dylib", "libdoo_file.dylib"),
    ];

    if let Ok(entries) = fs::read_dir(&deps_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(filename) = path.file_name() {
                let filename_str = filename.to_string_lossy();

                for (pattern, dest_name) in &patterns {
                    // Match exact name or name with hash (e.g., libdoo-abc123.so)
                    if filename_str == *pattern
                        || (filename_str.contains(pattern) && !filename_str.ends_with(".d"))
                    {
                        let dest = target_dir.join(dest_name);
                        // Only copy if source has content
                        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        if size > 0 {
                            if let Err(e) = fs::copy(&path, &dest) {
                                eprintln!("Warning: Failed to copy {}: {}", filename_str, e);
                            } else {
                                println!("  ✓ Copied {} ({} bytes)", dest_name, size);
                            }
                        }
                        break;
                    }
                }
            }
        }
    }
}

/// Windows-specific: copy additional .lib files needed for linking
#[cfg(target_os = "windows")]
fn copy_windows_libs(target_dir: &PathBuf) {
    let deps_dir = target_dir.join("deps");

    // Also look for .dll.lib files which are import libraries
    let lib_patterns = vec!["doo.dll.lib", "doo_file.dll.lib"];

    if let Ok(entries) = fs::read_dir(&deps_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(filename) = path.file_name() {
                let filename_str = filename.to_string_lossy();

                for pattern in &lib_patterns {
                    if filename_str.ends_with(pattern) || filename_str == *pattern {
                        let dest = target_dir.join(pattern);
                        if !dest.exists() {
                            if let Err(e) = fs::copy(&path, &dest) {
                                eprintln!("Warning: Failed to copy {}: {}", pattern, e);
                            }
                        }
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn install_windows(target_dir: &PathBuf) {
    println!("\n📦 Windows Installation");
    println!("========================");

    // Verify required files exist
    let required_files = vec!["doo.exe", "doo.dll", "doo_file.dll"];
    let mut missing = Vec::new();

    for file in &required_files {
        if !target_dir.join(file).exists() {
            missing.push(*file);
        }
    }

    if !missing.is_empty() {
        eprintln!("\n⚠️  Warning: Some files are missing: {:?}", missing);
    }

    println!("\nFiles in {}:", target_dir.display());
    println!("  - doo.exe (compiler binary)");
    println!("  - doo.dll (runtime library)");
    println!("  - doo_file.dll (file operations library)");

    println!("\n📋 Add to PATH (choose one method):");
    println!("\nMethod 1 - Current PowerShell session:");
    println!("  $env:PATH = \"{};$env:PATH\"", target_dir.display());

    println!("\nMethod 2 - Permanent (run PowerShell as Administrator):");
    println!(
        "  [Environment]::SetEnvironmentVariable('PATH', \"{};\" + [Environment]::GetEnvironmentVariable('PATH', 'User'), 'User')",
        target_dir.display()
    );

    println!("\nMethod 3 - Via System Settings:");
    println!("  1. Press Win+R, type 'sysdm.cpl', press Enter");
    println!("  2. Advanced tab → Environment Variables");
    println!("  3. Under User variables, edit PATH");
    println!("  4. Add: {}", target_dir.display());

    println!("\n💡 Verify installation:");
    println!("  doo --version");
}

#[cfg(not(target_os = "windows"))]
fn install_unix_from_path(target_dir: &PathBuf, workspace_root: &PathBuf) {
    let home = env::var("HOME").expect("HOME environment variable not set");
    let doo_dir = PathBuf::from(&home).join(".local").join("bin").join("doo");

    println!("📦 Installing to {}...", doo_dir.display());

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
        Ok(size) => {
            println!("  ✓ Installed doo binary ({} bytes)", size);
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
    let lib_names = vec!["libdoo.so", "libdoo_file.so"];

    #[cfg(target_os = "macos")]
    let lib_names = vec!["libdoo.dylib", "libdoo_file.dylib"];

    let mut libs_installed = 0;
    for lib_name in &lib_names {
        let lib_file = target_dir.join(lib_name);
        let dest_lib = doo_dir.join(lib_name);

        if lib_file.exists() {
            let size = fs::metadata(&lib_file).map(|m| m.len()).unwrap_or(0);
            if size == 0 {
                eprintln!("  ⚠️  {} is empty (0 bytes), skipping", lib_name);
                continue;
            }

            match fs::copy(&lib_file, &dest_lib) {
                Ok(_) => {
                    println!("  ✓ Installed {} ({} bytes)", lib_name, size);
                    libs_installed += 1;
                }
                Err(e) => {
                    eprintln!("  ⚠️  Failed to copy {}: {}", lib_name, e);
                }
            }
        } else {
            eprintln!("  ⚠️  {} not found in target directory", lib_name);
        }
    }

    if libs_installed == 0 {
        eprintln!("\n❌ No libraries were installed!");
        eprintln!("   This usually happens when building on Windows filesystem in WSL.");
        eprintln!("   The build should have been done in native filesystem.");
        exit(1);
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

    // List installed files
    println!("\nInstalled files:");
    if let Ok(entries) = fs::read_dir(&doo_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if path.is_file() {
                let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                println!("  - {} ({} bytes)", name, size);
            } else if path.is_dir() {
                println!("  - {}/ (directory)", name);
            }
        }
    }

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
