use std::env;
use std::fs;
use std::process::{exit, Command};

#[cfg(not(target_os = "windows"))]
use std::path::PathBuf;

#[cfg(not(target_os = "windows"))]
use std::io::Write;

#[cfg(not(target_os = "windows"))]
fn get_workspace_root() -> PathBuf {
    let mut current = env::current_dir().expect("Failed to get current directory");

    loop {
        if current.join("Cargo.toml").exists() {
            let cargo_toml = fs::read_to_string(current.join("Cargo.toml")).unwrap_or_default();
            // Check if this is a workspace or package Cargo.toml (and not xtask subfolder)
            let is_xtask = current
                .file_name()
                .map(|n| n.to_string_lossy() == "xtask")
                .unwrap_or(false);
            if (cargo_toml.contains("[workspace]") || cargo_toml.contains("[package]")) && !is_xtask
            {
                return current;
            }
        }

        if !current.pop() {
            return env::current_dir().expect("Failed to get current directory");
        }
    }
}

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

    let workspace_root = get_workspace_root();
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

    // Platform-specific installation
    #[cfg(target_os = "windows")]
    {
        install_windows(&target_dir);
    }

    #[cfg(not(target_os = "windows"))]
    {
        if release {
            install_unix(target_dir, &workspace_root);
        } else {
            println!("Add to PATH:");
            println!("  export PATH=\"{}:$PATH\"", target_dir.display());
        }
    }
}

#[cfg(target_os = "windows")]
fn install_windows(target_dir: &std::path::PathBuf) {
    println!("\n📦 Setting up Windows PATH...");
    println!("Add this to your PATH:");
    println!("  {}", target_dir.display());
    println!("\nPowerShell command (current session):");
    println!("  $env:PATH = \"{};${{env:PATH}}\"", target_dir.display());
    println!("\nPermanent PATH (run as Administrator):");
    println!(
        "  [Environment]::SetEnvironmentVariable('PATH', \"{};${{env:PATH}}\", 'User')",
        target_dir.display()
    );
}

#[cfg(not(target_os = "windows"))]
fn install_unix(target_dir: PathBuf, workspace_root: &PathBuf) {
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
    let lib_names = vec!["libdoo.so", "libdoo_file.so"];

    #[cfg(target_os = "macos")]
    let lib_names = vec!["libdoo.dylib", "libdoo_file.dylib"];

    for lib_name in lib_names {
        let lib_file = target_dir.join(lib_name);
        let dest_lib = doo_dir.join(lib_name);

        if lib_file.exists() {
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

    // Automatically update shell configuration
    println!("\n📝 Updating shell configuration...");
    let shell_configs = vec![
        PathBuf::from(&home).join(".bashrc"),
        PathBuf::from(&home).join(".zshrc"),
    ];

    for config_file in shell_configs {
        if config_file.exists() {
            let content = fs::read_to_string(&config_file).unwrap_or_default();
            if !content.contains(".local/bin/doo") {
                match fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&config_file)
                {
                    Ok(mut file) => {
                        // Add newline before export if file doesn't end with one
                        if !content.is_empty() && !content.ends_with('\n') {
                            let _ = writeln!(file);
                        }
                        if let Err(e) = writeln!(
                            file,
                            "# Added by doo installer\nexport PATH=\"$HOME/.local/bin/doo:$PATH\""
                        ) {
                            eprintln!("⚠️  Failed to update {}: {}", config_file.display(), e);
                        } else {
                            println!(
                                "  ✓ Updated {}",
                                config_file
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("⚠️  Failed to open {}: {}", config_file.display(), e);
                    }
                }
            } else {
                println!(
                    "  ✓ {} already has doo in PATH",
                    config_file
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                );
            }
        }
    }

    println!("\n✅ Installation complete!");
    println!("📁 Installed to: {}", doo_dir.display());

    println!("\n🔍 Verifying installation...");

    // Try to verify doo works with updated PATH
    let verify_script = format!(
        "source \"{}/.zshrc\" 2>/dev/null || source \"{}/.bashrc\" 2>/dev/null; \"{}/.local/bin/doo/doo\" --version 2>/dev/null",
        home, home, home
    );

    let mut verified = false;
    match Command::new("sh").arg("-c").arg(&verify_script).output() {
        Ok(output) => {
            if output.status.success() {
                let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!("  ✓ {} installed and verified!", version);
                verified = true;
            }
        }
        Err(_) => {}
    }

    if verified {
        println!("\n✨ Installation complete! You can now use `doo` in any new terminal.");
        println!("   For this terminal session, run one of these:");
        println!("   • zsh users: source ~/.zshrc");
        println!("   • bash users: source ~/.bashrc");
    } else {
        println!("  ⚠️  Installation completed");
        println!("\n✨ To use doo now:");
        println!("   • Open a new terminal, OR");
        println!("   • Run in current terminal:");

        let shell = env::var("SHELL").unwrap_or_default();
        if shell.contains("zsh") {
            println!("     source ~/.zshrc && doo --version");
        } else if shell.contains("bash") {
            println!("     source ~/.bashrc && doo --version");
        } else {
            println!("     source ~/.zshrc && doo --version");
        }
    }
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
