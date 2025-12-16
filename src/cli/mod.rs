pub mod templates;

use clap::{Parser, Subcommand};
use dialoguer::{theme::ColorfulTheme, Password, Select};
use std::io::{self, Write};

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use templates::TEMPLATES;

/// CLI definition for the doo language tool.
#[derive(Parser)]
#[command(name = "doo")]
#[command(about = "doo language CLI")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Supported subcommands for the doo CLI.
#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new project from a template
    Init {
        /// Name of the project (optional, interactive if missing)
        name: Option<String>,

        /// Template to use (optional, interactive if missing)
        #[arg(long, short)]
        template: Option<String>,
    },

    /// Deploy the project to Fly.io
    Deploy,

    /// Build the project to a persistent binary
    Build {
        /// Path to the project directory or .doo file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Name of the output binary
        #[arg(short, long, default_value = "output")]
        output: String,

        /// Keep the generated LLVM IR (.ll) file
        #[arg(long)]
        keep_ll: bool,
    },

    /// Compile and run immediately (auto-cleanup)
    Run {
        /// Path to the project directory or main.doo file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Keep the generated LLVM IR (.ll) file
        #[arg(long)]
        keep_ll: bool,

        /// Arguments to pass to the program
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Check for errors without compiling
    Check {
        /// Path to the project directory or main.doo file
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Upgrade doo to the latest version
    Upgrade,
}

// Simple string constants for emojis since we removed console::Emoji
const SPARKLE: &str = "✨ ";
const TRUCK: &str = "🚚 ";
const ROCKET: &str = "🚀 ";
const ERROR: &str = "❌ ";
const CHECK: &str = "✅ ";
const PACKAGE: &str = "📦 ";
const KEY: &str = "🔑 ";
const CLOUD: &str = "☁️  ";
const HAMMER: &str = "🔨 ";
const ARROW_UP: &str = "⬆️  ";
const INFO: &str = "ℹ️  ";


/// Entrypoint for CLI logic.
/// Returns exit code (0 for success, nonzero for error).
pub fn run_cli(cli: Cli) -> i32 {
    use doo::compiler::{compile_project, CompileOptions};

    match cli.command {
        None => {
            println!("🎉 doo CLI - doo language tool");
            println!("Type 'doo --help' for usage");
            0
        }
        Some(Commands::Init { name, template }) => run_init(name, template),
        Some(Commands::Deploy) => run_deploy(),
        Some(Commands::Build {
            path,
            output,
            keep_ll,
        }) => {
            let opts = CompileOptions {
                input_path: path.clone(),
                output_name: output.clone(),
                dev_mode: false,
                print_ast: false,
                print_mir: false,
                keep_ll,
                keep_obj: false,
                check_only: false,
            };

            match compile_project(opts) {
                Ok(result) => {
                    if result.error_count > 0 {
                        eprintln!("{} Build failed with {} errors", ERROR, result.error_count);
                        return 1;
                    } else if result.success {
                        copy_dlls_if_needed();
                        println!("{} Build successful: {}", CHECK, output);
                        return 0;
                    } else {
                        eprintln!("{} Build failed", ERROR);
                        return 1;
                    }
                }
                Err(e) => {
                    eprintln!("{} Error: {}", ERROR, e);
                    return 1;
                }
            }
        }
        Some(Commands::Run {
            path,
            keep_ll,
            args,
        }) => {
            let temp_name = format!("temp_doo_{}", std::process::id());
            
            // Use target/release for temporary binary to find DLLs and keep root clean
            let target_dir = Path::new("target").join("release");
            if !target_dir.exists() {
                let _ = std::fs::create_dir_all(&target_dir);
            }
            
            // output_name should be the path without extension
            let output_path_buf = target_dir.join(&temp_name);
            let output_name = output_path_buf.to_string_lossy().to_string();

            let opts = CompileOptions {
                input_path: path.clone(),
                output_name: output_name.clone(),
                dev_mode: false,
                print_ast: false,
                print_mir: false,
                keep_ll,
                keep_obj: false,
                check_only: false,
            };

            match compile_project(opts) {
                Ok(result) => {
                    if result.error_count > 0 || !result.success {
                        eprintln!(
                            "{} Compilation failed with {} errors",
                            ERROR, result.error_count
                        );
                        // Try to cleanup if file was created
                        let _ = std::fs::remove_file(&output_name); 
                        if cfg!(windows) {
                            let _ = std::fs::remove_file(format!("{}.exe", output_name));
                        }
                        return 1;
                    }
                    // No need to copy DLLs anymore!
                }
                Err(e) => {
                    eprintln!("{} Failed to compile: {}", ERROR, e);
                    let _ = std::fs::remove_file(&output_name);
                    if cfg!(windows) {
                        let _ = std::fs::remove_file(format!("{}.exe", output_name));
                    }
                    return 1;
                }
            }

            let exe_name = if cfg!(windows) {
                format!("{}.exe", temp_name)
            } else {
                temp_name.clone()
            };
            
            // Full absolute path to executable
            let exe_full_path = match std::env::current_dir() {
                Ok(dir) => dir.join("target").join("release").join(&exe_name),
                Err(_) => {
                    eprintln!("{} Error: Could not determine current directory", ERROR);
                    return 1;
                }
            };

            let status = Command::new(&exe_full_path)
                .args(&args)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status();

            let code = match status {
                Ok(s) => {
                    let code = s.code().unwrap_or(1);
                    // Cleanup exe
                    let _ = std::fs::remove_file(&exe_full_path);
                    code
                }
                Err(e) => {
                    eprintln!("{} Failed to start process: {}", ERROR, e);
                    let _ = std::fs::remove_file(&exe_full_path);
                    1
                }
            };
            
            // Final attempt cleanup
            let _ = std::fs::remove_file(&exe_full_path);
            code
        }
        Some(Commands::Check { path }) => {
            let opts = CompileOptions {
                input_path: path.clone(),
                output_name: "output".to_string(),
                dev_mode: false,
                print_ast: false,
                print_mir: false,
                keep_ll: false,
                keep_obj: false,
                check_only: true,
            };

            match compile_project(opts) {
                Ok(result) => {
                    if result.error_count > 0 {
                        println!("Finding errors... Found {}", result.error_count);
                        return 1;
                    } else {
                        println!("{} No errors found", CHECK);
                        return 0;
                    }
                }
                Err(e) => {
                    eprintln!("{} Failed to check: {}", ERROR, e);
                    return 1;
                }
            }
        }
        Some(Commands::Upgrade) => run_upgrade(),
    }
}

// Helper to copy DLLs on Windows - REMOVED to avoid pollution
// Instead, we rely on the binary being in target/release or PATH being set correctly
fn copy_dlls_if_needed() {
    // No-op: User requested to stop copying DLLs to root
}

fn run_init(name_arg: Option<String>, template_arg: Option<String>) -> i32 {
    let name = match name_arg {
        Some(n) => n,
        None => {
            print!("📦 Project name: ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();

            let n = input.trim().to_string();
            if n.is_empty() {
                println!("{} Project name cannot be empty", ERROR);
                return 1;
            }
            n
        }
    };

    if name.is_empty() {
        println!("{} Project name cannot be empty", ERROR);
        return 1;
    }

    let template_idx = match template_arg {
        Some(t_name) => match TEMPLATES.iter().position(|t| t.name == t_name) {
            Some(idx) => idx,
            None => {
                println!("{} Unknown template: {}", ERROR, t_name);
                println!("Available templates: starter, todo, blog");
                return 1;
            }
        },
        None => {
            println!("\n🎨 Choose a template  (↑↓ navigate, Enter select)\n");

            // No prompt text → removes "? Templates ›"
            let selections: Vec<String> = TEMPLATES
                .iter()
                .map(|t| format!("{:<8}  {}", t.name, t.description))
                .collect();

            let idx = Select::with_theme(&ColorfulTheme::default())
                .items(&selections)
                .default(0)
                .interact()
                .unwrap();

            idx
        }
    };

    let template = &TEMPLATES[template_idx];
    let project_path = Path::new(&name);

    if project_path.exists() {
        println!("{} Directory '{}' already exists", ERROR, name);
        return 1;
    }

    println!(
        "\n{} Creating project '{}' using {} template...",
        HAMMER, name, template.name
    );

    if let Err(e) = fs::create_dir(project_path) {
        println!("{} Failed to create directory: {}", ERROR, e);
        return 1;
    }

    // Write template files
    for file in template.files {
        let file_path = project_path.join(file.path);
        if let Some(parent) = file_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = fs::write(&file_path, file.content) {
            println!("{} Failed to write {}: {}", ERROR, file.path, e);
            return 1;
        }
        println!("   {} Created {}", CHECK, file.path);
    }

    println!("\n{} Project created successfully!", SPARKLE);
    println!("👉 Run the project:");
    println!("   cd {}", name);
    println!("   doo run");
    0
}

fn check_flyctl_installed() -> bool {
    Command::new("flyctl")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn install_flyctl() -> bool {
    println!("{} flyctl not found. Installing...", PACKAGE);
    let result = if cfg!(target_os = "windows") {
        Command::new("powershell")
            .arg("-Command")
            .arg("iwr https://fly.io/install.ps1 -useb | iex")
            .status()
    } else {
        Command::new("sh")
            .arg("-c")
            .arg("curl -L https://fly.io/install.sh | sh")
            .status()
    };

    match result {
        Ok(s) if s.success() => {
            println!("{} flyctl installed successfully!", CHECK);
            let home = env::var("HOME")
                .or_else(|_| env::var("USERPROFILE"))
                .unwrap_or_default();
            let bin_path = Path::new(&home).join(".fly").join("bin");
            if bin_path.exists() {
                let path_env = env::var("PATH").unwrap_or_default();
                let new_path = format!(
                    "{}{}{}",
                    path_env,
                    if cfg!(windows) { ";" } else { ":" },
                    bin_path.display()
                );
                env::set_var("PATH", new_path);
            }
            true
        }
        _ => {
            println!("{} Failed to install flyctl.", ERROR);
            false
        }
    }
}

fn ensure_env_vars() {
    let env_path = Path::new(".env");
    let mut current_content = String::new();
    if env_path.exists() {
        current_content = fs::read_to_string(env_path).unwrap_or_default();
    }

    let mut needs_update = false;
    let mut new_content = current_content.clone();

    if !new_content.contains("DATABASE_URL=") {
        if !new_content.is_empty() && !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str("DATABASE_URL=postgres://postgres:postgres@localhost:5432/doo_app\n");
        needs_update = true;
    }
    if !new_content.contains("JWT_SECRET=") {
        if !new_content.is_empty() && !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str("JWT_SECRET=super_secret_key_change_me\n");
        needs_update = true;
    }
    if !new_content.contains("FLY_API_TOKEN=") {
        if !new_content.is_empty() && !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str("FLY_API_TOKEN=\n");
        needs_update = true;
    }

    if needs_update {
        if let Err(e) = fs::write(env_path, &new_content) {
            eprintln!("{} Failed to update .env: {}", ERROR, e);
        } else {
            println!("{} Updated .env with default variables", CHECK);
        }
    }
}

fn run_deploy() -> i32 {
    println!("\n{} {} {}", ROCKET, "Doo Deployment", "(Fly.io)");

    if !check_flyctl_installed() {
        if !install_flyctl() {
            return 1;
        }
    }

    ensure_env_vars();

    if let Ok(lines) = fs::read_to_string(".env") {
        for line in lines.lines() {
            if let Some((key, value)) = line.split_once('=') {
                env::set_var(key.trim(), value.trim());
            }
        }
    }

    let token = env::var("FLY_API_TOKEN").unwrap_or_default();
    if token.is_empty() {
        println!("\n{} Authentication required for Fly.io", KEY);
        println!("   Please enter your Fly.io API token (or press Enter to run 'fly auth login')");

        let input = match Password::with_theme(&ColorfulTheme::default())
            .with_prompt("Token")
            .allow_empty_password(true)
            .interact()
        {
            Ok(s) => s,
            Err(_) => String::new(),
        };

        if input.is_empty() {
            println!("{} Running 'fly auth login'...", SPARKLE);
            let status = Command::new("flyctl").arg("auth").arg("login").status();
            if status.is_err() || !status.unwrap().success() {
                println!("{} Login failed", ERROR);
                return 1;
            }
        } else {
            let env_path = Path::new(".env");
            if let Ok(content) = fs::read_to_string(env_path) {
                let new_content = if content.contains("FLY_API_TOKEN=") {
                    content
                        .lines()
                        .map(|line| {
                            if line.starts_with("FLY_API_TOKEN=") {
                                format!("FLY_API_TOKEN={}", input)
                            } else {
                                line.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    format!("{}\nFLY_API_TOKEN={}", content, input)
                };

                let _ = fs::write(env_path, new_content);
                env::set_var("FLY_API_TOKEN", input);
            }
        }
    }

    if !Path::new("fly.toml").exists() {
        println!("{} Initializing Fly app...", CLOUD);
        let status = Command::new("flyctl")
            .arg("launch")
            .arg("--no-deploy")
            .status();

        if status.is_err() || !status.unwrap().success() {
            println!("{} Fly launch failed", ERROR);
            return 1;
        }
    }

    println!("\n{} Deploying to Fly.io...", TRUCK);
    let status = Command::new("flyctl").arg("deploy").status();

    match status {
        Ok(s) if s.success() => {
            println!("\n{} Deployment successful!", SPARKLE);
            0
        }
        _ => {
            println!("\n{} Deployment failed", ERROR);
            1
        }
    }
}

/// Upgrade doo to the latest version
fn run_upgrade() -> i32 {
    println!("\n{} Doo Upgrade", ARROW_UP);

    // Get current version from Cargo.toml embedded version
    let current_version = env!("CARGO_PKG_VERSION");
    println!("{} Current version: v{}", INFO, current_version);

    // Detect platform
    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        "linux"
    };

    println!("{} Detected platform: {}", INFO, platform);

    // Fetch latest version from GitHub API
    println!("{} Checking for updates...", PACKAGE);

    let latest_version = match fetch_latest_version() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{} Failed to check for updates: {}", ERROR, e);
            return 1;
        }
    };

    let latest_version_num = latest_version.trim_start_matches('v');
    println!("{} Latest version: v{}", INFO, latest_version_num);

    // Compare versions
    if current_version == latest_version_num {
        println!("\n{} You're already on the latest version!", CHECK);
        return 0;
    }

    println!(
        "\n{} Upgrading v{} → v{}",
        ARROW_UP, current_version, latest_version_num
    );

    // Get the doo installation directory
    let install_dir = match get_doo_install_dir() {
        Some(dir) => dir,
        None => {
            eprintln!("{} Could not determine doo installation directory", ERROR);
            eprintln!("   Please reinstall doo using the install script.");
            return 1;
        }
    };

    println!("{} Installation directory: {}", INFO, install_dir.display());

    // Download and extract new version
    if let Err(e) = download_and_upgrade(&install_dir, platform, &latest_version, latest_version_num) {
        eprintln!("{} Upgrade failed: {}", ERROR, e);
        return 1;
    }

    println!("\n{} Upgrade complete! You now have doo v{}", SPARKLE, latest_version_num);
    println!("   Run 'doo --version' to verify.");

    0
}

/// Fetch the latest version tag from GitHub API
fn fetch_latest_version() -> Result<String, String> {
    let api_url = "https://api.github.com/repos/nynrathod/doolang/releases/latest";

    if cfg!(target_os = "windows") {
        // Use PowerShell on Windows
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "(Invoke-RestMethod -Uri '{}' -Headers @{{'User-Agent'='doo-upgrade'}}).tag_name",
                    api_url
                ),
            ])
            .output()
            .map_err(|e| format!("Failed to run PowerShell: {}", e))?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        // Use curl on Unix
        let output = Command::new("curl")
            .args(["-fsSL", "-H", "User-Agent: doo-upgrade", api_url])
            .output()
            .map_err(|e| format!("Failed to run curl: {}", e))?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }

        // Parse JSON to get tag_name
        let json_str = String::from_utf8_lossy(&output.stdout);
        
        // Simple JSON parsing for tag_name
        if let Some(start) = json_str.find("\"tag_name\":") {
            let rest = &json_str[start + 11..];
            if let Some(quote_start) = rest.find('"') {
                let rest = &rest[quote_start + 1..];
                if let Some(quote_end) = rest.find('"') {
                    return Ok(rest[..quote_end].to_string());
                }
            }
        }

        Err("Failed to parse version from GitHub API response".to_string())
    }
}

/// Get the directory where doo is installed
fn get_doo_install_dir() -> Option<PathBuf> {
    // Try to find where the current executable is
    if let Ok(exe_path) = env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            return Some(parent.to_path_buf());
        }
    }

    // Fallback: check standard installation directory
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .ok()?;
    
    let default_dir = Path::new(&home).join(".doo").join("bin");
    if default_dir.exists() {
        return Some(default_dir);
    }

    None
}

/// Download and upgrade doo
fn download_and_upgrade(
    install_dir: &Path,
    platform: &str,
    version_tag: &str,
    version_num: &str,
) -> Result<(), String> {
    let download_url = format!(
        "https://github.com/nynrathod/doolang/releases/download/{}/doo-{}-{}.zip",
        version_tag, platform, version_num
    );

    println!("{} Downloading from: {}", TRUCK, download_url);

    // Create temp directory
    let temp_dir = env::temp_dir().join(format!("doo-upgrade-{}", std::process::id()));
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Failed to create temp directory: {}", e))?;

    let zip_path = temp_dir.join("doo.zip");
    let extract_dir = temp_dir.join("extracted");

    // Download the zip file
    if cfg!(target_os = "windows") {
        let status = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; Invoke-WebRequest -Uri '{}' -OutFile '{}'",
                    download_url,
                    zip_path.display()
                ),
            ])
            .status()
            .map_err(|e| format!("Failed to download: {}", e))?;

        if !status.success() {
            return Err("Download failed".to_string());
        }
    } else {
        let status = Command::new("curl")
            .args(["-fsSL", &download_url, "-o", &zip_path.to_string_lossy()])
            .status()
            .map_err(|e| format!("Failed to download: {}", e))?;

        if !status.success() {
            return Err("Download failed".to_string());
        }
    }

    println!("{} Extracting...", PACKAGE);

    // Extract the zip file
    fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create extract directory: {}", e))?;

    if cfg!(target_os = "windows") {
        let status = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                    zip_path.display(),
                    extract_dir.display()
                ),
            ])
            .status()
            .map_err(|e| format!("Failed to extract: {}", e))?;

        if !status.success() {
            return Err("Extraction failed".to_string());
        }
    } else {
        let status = Command::new("unzip")
            .args(["-q", "-o", &zip_path.to_string_lossy(), "-d", &extract_dir.to_string_lossy()])
            .status()
            .map_err(|e| format!("Failed to extract: {}", e))?;

        if !status.success() {
            return Err("Extraction failed".to_string());
        }
    }

    // Find the extracted content (handle nested folder structure)
    let source_dir = find_doo_in_extracted(&extract_dir)?;

    println!("{} Backing up and replacing files...", HAMMER);

    // Create backup directory
    let backup_dir = temp_dir.join("backup");
    fs::create_dir_all(&backup_dir)
        .map_err(|e| format!("Failed to create backup directory: {}", e))?;

    // Known release files to update (only replace these, not user files)
    let release_files = get_release_file_patterns();

    // Backup and replace files
    for entry in fs::read_dir(&source_dir)
        .map_err(|e| format!("Failed to read source directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();
        let dest_path = install_dir.join(&file_name);

        // Only update release files, skip unknown files
        if !is_release_file(&file_name_str, &release_files) && !entry.path().is_dir() {
            continue;
        }

        // Backup existing file/dir if it exists
        if dest_path.exists() {
            let backup_path = backup_dir.join(&file_name);
            if entry.path().is_dir() {
                copy_dir_recursive(&dest_path, &backup_path)?;
                fs::remove_dir_all(&dest_path).ok();
            } else {
                fs::copy(&dest_path, &backup_path).ok();
                fs::remove_file(&dest_path).ok();
            }
        }

        // Copy new file/dir
        if entry.path().is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            fs::copy(&entry.path(), &dest_path)
                .map_err(|e| format!("Failed to copy {}: {}", file_name_str, e))?;
        }

        println!("   {} Updated {}", CHECK, file_name_str);
    }

    // Make executable on Unix
    if !cfg!(target_os = "windows") {
        let doo_path = install_dir.join("doo");
        let _ = Command::new("chmod").args(["+x", &doo_path.to_string_lossy()]).status();
    }

    // Cleanup temp directory
    let _ = fs::remove_dir_all(&temp_dir);

    Ok(())
}

/// Find the directory containing doo binary in extracted content
fn find_doo_in_extracted(extract_dir: &Path) -> Result<PathBuf, String> {
    let doo_exe = if cfg!(target_os = "windows") {
        "doo.exe"
    } else {
        "doo"
    };

    // Check if doo is directly in extract_dir
    if extract_dir.join(doo_exe).exists() {
        return Ok(extract_dir.to_path_buf());
    }

    // Look in subdirectories
    for entry in fs::read_dir(extract_dir)
        .map_err(|e| format!("Failed to read extract directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        if entry.path().is_dir() {
            let possible_path = entry.path().join(doo_exe);
            if possible_path.exists() {
                return Ok(entry.path());
            }
        }
    }

    // Fallback: return first subdirectory
    for entry in fs::read_dir(extract_dir)
        .map_err(|e| format!("Failed to read extract directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        if entry.path().is_dir() {
            return Ok(entry.path());
        }
    }

    Err("Could not find extracted files".to_string())
}

/// Get list of known release file patterns
fn get_release_file_patterns() -> Vec<&'static str> {
    vec![
        "doo",
        "doo.exe",
        "doo.dll",
        "doo.dll.exp",
        "doo.dll.lib",
        "doo_file.dll",
        "doo_file.dll.exp",
        "doo_file.dll.lib",
        "libdoo.so",
        "libdoo.dylib",
        "libdoo_file.so",
        "libdoo_file.dylib",
        "std", // std library folder
    ]
}

/// Check if a file matches release file patterns
fn is_release_file(file_name: &str, patterns: &[&str]) -> bool {
    for pattern in patterns {
        if file_name == *pattern || file_name.starts_with("doo") || file_name.starts_with("libdoo") {
            return true;
        }
    }
    file_name == "std"
}

/// Recursively copy a directory
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst)
        .map_err(|e| format!("Failed to create directory {}: {}", dst.display(), e))?;

    for entry in fs::read_dir(src)
        .map_err(|e| format!("Failed to read directory {}: {}", src.display(), e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("Failed to copy {}: {}", src_path.display(), e))?;
        }
    }

    Ok(())
}

