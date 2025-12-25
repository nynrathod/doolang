pub mod analytics;
pub mod templates;

use clap::{Parser, Subcommand};
use dialoguer::{
    theme::{ColorfulTheme, SimpleTheme},
    Confirm, MultiSelect, Password, Select,
};
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

    /// Deploy the project to Fly.io or Railway
    Deploy {
        /// Show detailed build and deployment logs
        #[arg(long, short)]
        verbose: bool,
    },

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

// Simple string constants - minimal emoji usage for clean UX
const SPARKLE: &str = "✨";
const TRUCK: &str = "  ";
const ERROR: &str = "✗ ";
const CHECK: &str = "✓ ";
const PACKAGE: &str = "  ";
const KEY: &str = "  ";
const HAMMER: &str = "  ";
const ARROW_UP: &str = "  ";
const INFO: &str = "  ";

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
        Some(Commands::Deploy { verbose }) => run_deploy(verbose),
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

            use std::sync::{Arc, Mutex};

            let child = Command::new(&exe_full_path)
                .args(&args)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn();

            let code = match child {
                Ok(child) => {
                    // Handle Ctrl+C to kill the child process
                    let child_process = Arc::new(Mutex::new(Some(child)));
                    let child_process_clone = child_process.clone();

                    // Set Ctrl+C handler
                    let _ = ctrlc::set_handler(move || {
                        if let Ok(mut guard) = child_process_clone.lock() {
                            if let Some(child) = guard.as_mut() {
                                let _ = child.kill();
                            }
                        }
                        std::process::exit(130);
                    });

                    // Wait for the child to finish
                    // We need to release the lock while waiting to allow the signal handler to acquire it
                    // However, Child::wait() takes &mut self, so we'd need to keep the lock.
                    // Instead, we can poll or just block if we don't mind the signal handler potentially blocking on the lock.
                    // Loop with short sleep to allow signal handler to grab lock if needed
                    let result = loop {
                        // Scope to drop lock immediately
                        {
                            let mut guard = child_process.lock().unwrap();
                            if let Some(c) = guard.as_mut() {
                                if let Ok(Some(status)) = c.try_wait() {
                                    break status.code().unwrap_or(1);
                                }
                            } else {
                                // Child killed/gone
                                break 1;
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    };

                    let _ = std::fs::remove_file(&exe_full_path);
                    result
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

    // Track project creation (fire-and-forget, anonymous)
    analytics::track_project_created(template.name);
    analytics::flush(); // Wait for event to send

    0
}

fn run_deploy(verbose: bool) -> i32 {
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  🚀 Doo Deploy");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    // ================================================================
    // Step 1: Validate project structure
    // ================================================================
    if !validate_project_structure(verbose) {
        return 1;
    }

    // ================================================================
    // Step 2: Select deployment platform
    // ================================================================
    let platforms = vec![
        "Fly.io",
        // "Railway"
    ];

    let platform_idx = match Select::with_theme(&SimpleTheme)
        .with_prompt("Select platform")
        .items(&platforms)
        .default(0)
        .interact()
    {
        Ok(idx) => idx,
        Err(_) => {
            println!("{}Failed to select platform", ERROR);
            return 1;
        }
    };

    let platform_name = platforms[platform_idx];
    println!();
    println!("→ Platform: {}", platform_name);
    println!();

    // ================================================================
    // Step 3: Deploy to selected platform
    // ================================================================
    match platform_name {
        "Fly.io" => deploy_flyio(verbose),
        "Railway" => deploy_railway(verbose),
        _ => {
            println!("{} Unknown platform", ERROR);
            1
        }
    }
}

// ============================================================================
// PROJECT VALIDATION
// ============================================================================

fn validate_project_structure(_verbose: bool) -> bool {
    use templates::DOCKERFILE_CONTENT;

    // Check for main.doo
    let main_doo = Path::new("main.doo");
    if !main_doo.exists() {
        println!("{}main.doo not found", ERROR);
        println!("  Make sure you're in a doo project directory.");
        println!("  Run 'doo init <name>' to create a new project.\n");
        return false;
    }

    // Check for .env file (optional but create if missing)
    let env_file = Path::new(".env");
    let env_exists = env_file.exists();
    if !env_exists {
        let _ = fs::write(env_file, "# Doo environment variables\n");
    }

    // Check for Dockerfile (create if missing)
    let dockerfile = Path::new("Dockerfile");
    let dockerfile_exists = dockerfile.exists();
    if !dockerfile_exists {
        let _ = fs::write(dockerfile, DOCKERFILE_CONTENT);
    }

    // Print validation tree
    println!("{}Project validated", CHECK);
    println!("  ├─ main.doo");
    println!("  ├─ .env{}", if env_exists { "" } else { " (created)" });
    println!(
        "  └─ Dockerfile{}",
        if dockerfile_exists { "" } else { " (created)" }
    );
    println!();

    true
}

// ============================================================================
// FLY.IO DEPLOYMENT
// ============================================================================

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
    println!("{} flyctl not found.", PACKAGE);

    let install = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(" Would you like to install Fly.io CLI?")
        .default(true)
        .interact()
        .unwrap_or(false);

    if !install {
        println!("{} Installation cancelled", INFO);
        println!("   Install manually: https://fly.io/docs/hands-on/install-flyctl/");
        return false;
    }

    println!("{} Installing flyctl...", TRUCK);

    let result = if cfg!(target_os = "windows") {
        Command::new("powershell")
            .arg("-NoProfile")
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
            // Add to PATH for current session
            let home = env::var("HOME")
                .or_else(|_| env::var("USERPROFILE"))
                .unwrap_or_default();
            let bin_path = Path::new(&home).join(".fly").join("bin");
            if bin_path.exists() {
                let path_env = env::var("PATH").unwrap_or_default();
                let separator = if cfg!(windows) { ";" } else { ":" };
                let new_path = format!("{}{}{}", path_env, separator, bin_path.display());
                env::set_var("PATH", new_path);
            }
            true
        }
        Ok(_) => {
            println!("{} Installation failed", ERROR);
            println!("   Try installing manually: https://fly.io/docs/hands-on/install-flyctl/");
            false
        }
        Err(e) => {
            println!("{} Failed to run installer: {}", ERROR, e);
            println!("   Try installing manually: https://fly.io/docs/hands-on/install-flyctl/");
            false
        }
    }
}

fn read_env_vars() -> Vec<(String, String)> {
    let mut vars = Vec::new();
    let env_path = Path::new(".env");

    if let Ok(content) = fs::read_to_string(env_path) {
        for line in content.lines() {
            let line = line.trim();
            // Skip comments and empty lines
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim().to_string();
                let value = value.trim().to_string();
                if !key.is_empty() {
                    vars.push((key, value));
                }
            }
        }
    }

    vars
}

fn prompt_env_vars_for_production() -> Vec<(String, String)> {
    let env_vars = read_env_vars();

    if env_vars.is_empty() {
        println!("{} No environment variables found in .env", INFO);
        return Vec::new();
    }

    println!(
        "\n{} Select environment variables to set in production:",
        KEY
    );
    println!("   (Use Space to select, Enter to confirm)");
    println!(
        "   {} Never auto-upload secrets - you'll set values manually\n",
        INFO
    );

    let var_names: Vec<String> = env_vars.iter().map(|(k, _)| k.clone()).collect();

    // Pre-select DATABASE_URL and JWT_SECRET if they exist
    let defaults: Vec<bool> = var_names
        .iter()
        .map(|name| {
            name == "DATABASE_URL"
                || name == "JWT_SECRET"
                || name.contains("SECRET")
                || name.contains("KEY")
                || name.contains("TOKEN")
        })
        .collect();

    let selected_indices = match MultiSelect::with_theme(&ColorfulTheme::default())
        .items(&var_names)
        .defaults(&defaults)
        .interact()
    {
        Ok(indices) => indices,
        Err(_) => Vec::new(),
    };

    let mut result: Vec<(String, String)> = Vec::new();

    for idx in selected_indices {
        let var_name = &var_names[idx];
        let current_value = &env_vars[idx].1;

        // Prompt for production value
        println!("\n   {} Enter production value for {}:", KEY, var_name);
        println!(
            "   Current local value: {}",
            if current_value.len() > 20 {
                format!("{}...", &current_value[..20])
            } else {
                current_value.clone()
            }
        );

        let value = match Password::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("   {}", var_name))
            .allow_empty_password(true)
            .interact()
        {
            Ok(v) if !v.is_empty() => v,
            _ => current_value.clone(), // Use local value if empty
        };

        result.push((var_name.clone(), value));
    }

    result
}

fn deploy_flyio(verbose: bool) -> i32 {
    // Track deployment attempt (fire-and-forget, anonymous)
    analytics::track_deploy_attempt("flyio");

    // Check and install flyctl
    if !check_flyctl_installed() {
        println!("→ Installing Fly CLI...");

        if !install_flyctl() {
            println!("{}Fly CLI installation failed", ERROR);
            println!("  Install manually: https://fly.io/docs/flyctl/install/");
            analytics::track_deploy_error("flyio", "cli_install_failed");
            analytics::flush();
            return 1;
        }
        println!("{}Fly CLI installed", CHECK);
    }

    // Authenticate (silent check)
    let whoami = Command::new("flyctl")
        .args(["auth", "whoami"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if whoami.is_err() || !whoami.unwrap().success() {
        println!("→ Authenticating with Fly.io...");
        // Need to authenticate - this must be interactive (opens browser)
        let status = Command::new("flyctl")
            .args(["auth", "login"])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();

        if status.is_err() || !status.unwrap().success() {
            println!("{}Authentication failed", ERROR);
            analytics::track_deploy_error("flyio", "auth_failed");
            analytics::flush();
            return 1;
        }
    }
    println!("{}Authenticated with Fly.io", CHECK);

    // Check if this is first deploy (fly.toml doesn't exist)
    let is_first_deploy = !Path::new("fly.toml").exists();

    if is_first_deploy {
        println!();
        println!("→ Creating Fly app...");

        let status = Command::new("flyctl")
            .args(["launch", "--no-deploy", "--generate-name"])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();

        match status {
            Ok(s) if s.success() => {
                println!("{}Fly app created", CHECK);
            }
            _ => {
                println!("{}Failed to create Fly app", ERROR);
                println!("  Try: flyctl launch --no-deploy");
                analytics::track_deploy_error("flyio", "app_creation_failed");
                analytics::flush();
                return 1;
            }
        }

        // Ask for secrets on first deploy only
        let env_vars = prompt_env_vars_for_production();
        if !env_vars.is_empty() {
            print!("→ Setting secrets...");
            io::stdout().flush().ok();

            let secrets_args: Vec<String> = env_vars
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();

            let mut cmd = Command::new("flyctl");
            cmd.arg("secrets").arg("set").arg("--stage");
            for arg in &secrets_args {
                cmd.arg(arg);
            }

            let status = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status();

            match status {
                Ok(s) if s.success() => {
                    println!(" {}done", CHECK);
                }
                _ => {
                    println!(" {}warning: some secrets may not be set", INFO);
                }
            }
        }
    }

    // Deploy
    println!();
    println!("→ Deploying to Fly.io...");

    let start = std::time::Instant::now();

    let status = Command::new("flyctl")
        .args(["deploy"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    match status {
        Ok(s) if s.success() => {
            let duration = start.elapsed();
            println!(
                "\r{}Deployed successfully ({:.1}s)",
                CHECK,
                duration.as_secs_f64()
            );

            // Get deployment URL
            let output = Command::new("flyctl").args(["status", "--json"]).output();
            let mut hostname = String::new();

            if let Ok(out) = output {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if let Some(start) = stdout.find("\"Hostname\":") {
                    let rest = &stdout[start + 11..];
                    if let Some(quote_start) = rest.find('"') {
                        let rest = &rest[quote_start + 1..];
                        if let Some(quote_end) = rest.find('"') {
                            hostname = rest[..quote_end].to_string();
                        }
                    }
                }
            }

            println!();
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            if !hostname.is_empty() {
                println!("  ✨ Live at https://{}", hostname);
            } else {
                println!("  ✨ Deployed! Run 'flyctl open' to view");
            }
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!();

            // Track deploy success
            analytics::track_deploy_success("flyio", duration.as_millis() as u64);
            analytics::flush();

            0
        }
        Ok(_) => {
            println!("\r{}Deployment failed", ERROR);
            println!();
            println!("  Troubleshooting:");
            println!("  • Run with --verbose to see full logs");
            println!("  • Check logs: flyctl logs");
            println!("  • Build locally: docker build .");

            // Track deploy error
            analytics::track_deploy_error("flyio", "build_failed");
            analytics::flush();

            1
        }
        Err(e) => {
            println!("\r{}Deployment error: {}", ERROR, e);
            println!();
            println!("  Check your network connection and try again.");

            // Track deploy error
            analytics::track_deploy_error("flyio", "network_error");
            analytics::flush();

            1
        }
    }
}

// ============================================================================
// RAILWAY DEPLOYMENT
// ============================================================================

fn check_railway_installed() -> bool {
    // First try PATH
    let in_path = Command::new("railway")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if in_path {
        return true;
    }

    // Check custom install location (Windows)
    #[cfg(target_os = "windows")]
    {
        let local_bin = env::var("USERPROFILE")
            .map(|p| format!("{}\\AppData\\Local\\Programs\\railway", p))
            .unwrap_or_default();

        let railway_exe = format!("{}\\railway.exe", local_bin);

        if Path::new(&railway_exe).exists() {
            // Add to PATH for current process
            if let Ok(current_path) = env::var("PATH") {
                env::set_var("PATH", format!("{};{}", local_bin, current_path));
            }
            return true;
        }
    }

    false
}

fn install_railway() -> bool {
    println!("→ Installing Railway CLI...");

    // Platform-specific installation
    #[cfg(target_os = "windows")]
    {
        // Get paths
        let scoop_path = env::var("USERPROFILE")
            .map(|p| format!("{}\\scoop\\shims\\scoop.cmd", p))
            .unwrap_or_default();

        let railway_via_scoop = env::var("USERPROFILE")
            .map(|p| format!("{}\\scoop\\shims\\railway.exe", p))
            .unwrap_or_default();

        // Target path for direct download (user's local bin)
        let local_bin = env::var("USERPROFILE")
            .map(|p| format!("{}\\AppData\\Local\\Programs\\railway", p))
            .unwrap_or_default();

        let railway_exe = format!("{}\\railway.exe", local_bin);

        // Method 1: Check if scoop is already installed
        if Path::new(&scoop_path).exists() {
            println!("  Installing via scoop...");

            let scoop_install = Command::new(&scoop_path)
                .args(["install", "railway"])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status();

            if scoop_install.is_ok() && scoop_install.unwrap().success() {
                if Path::new(&railway_via_scoop).exists() {
                    println!("{}Railway CLI installed successfully!", CHECK);
                    return true;
                }
            }
            println!("  scoop install failed, trying npm...");
        }

        // Method 2: Try npm if available
        let npm_install = Command::new("npm")
            .args(["install", "-g", "@railway/cli"])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();

        if npm_install.is_ok() && npm_install.unwrap().success() {
            println!("{}Railway CLI installed successfully!", CHECK);
            return true;
        }

        // Method 3: Direct binary download using curl.exe (built into Windows 10/11)
        println!("  Downloading Railway CLI binary...");

        // Create directory
        let _ = fs::create_dir_all(&local_bin);

        // Get latest version from GitHub API
        let version_output = Command::new("curl.exe")
            .args([
                "-s",
                "https://api.github.com/repos/railwayapp/cli/releases/latest",
            ])
            .output();

        let version = if let Ok(output) = version_output {
            let json = String::from_utf8_lossy(&output.stdout);
            // Find "tag_name":"vX.X.X" pattern
            if let Some(start) = json.find("\"tag_name\":") {
                let after_tag = &json[start + 11..];
                if let Some(quote_start) = after_tag.find('"') {
                    let version_start = &after_tag[quote_start + 1..];
                    if let Some(quote_end) = version_start.find('"') {
                        version_start[..quote_end].to_string()
                    } else {
                        "v4.15.1".to_string()
                    }
                } else {
                    "v4.15.1".to_string()
                }
            } else {
                "v4.15.1".to_string()
            }
        } else {
            "v4.15.1".to_string()
        };

        let zip_url = format!(
            "https://github.com/railwayapp/cli/releases/download/{}/railway-{}-x86_64-pc-windows-msvc.zip",
            version, version
        );
        let zip_path = format!("{}\\railway.zip", local_bin);

        println!("  Downloading {}...", version);

        // Download zip file
        let curl_download = Command::new("curl.exe")
            .args(["-L", "-s", "-o", &zip_path, &zip_url])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();

        if curl_download.is_ok()
            && curl_download.unwrap().success()
            && Path::new(&zip_path).exists()
        {
            // Extract using tar (built into Windows 10+)
            println!("  Extracting...");
            let extract = Command::new("tar")
                .args(["-xf", &zip_path, "-C", &local_bin])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();

            // Clean up zip
            let _ = fs::remove_file(&zip_path);

            if extract.is_ok() && extract.unwrap().success() && Path::new(&railway_exe).exists() {
                // Add to PATH for current process
                if let Ok(current_path) = env::var("PATH") {
                    env::set_var("PATH", format!("{};{}", local_bin, current_path));
                }

                println!("{}Railway CLI installed successfully!", CHECK);
                return true;
            }
        }

        // Clean up any partial downloads
        let _ = fs::remove_file(&zip_path);

        // All methods failed
        println!("{}Could not auto-install Railway CLI", ERROR);
        println!();
        println!("  Please install manually:");
        println!("  1. Install Node.js from https://nodejs.org");
        println!("  2. Run: npm i -g @railway/cli");
        println!();
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        // macOS: Use Homebrew
        let brew_check = Command::new("brew")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        let brew_available = brew_check.is_ok() && brew_check.unwrap().success();

        if !brew_available {
            println!("  Homebrew not found, installing Homebrew first...");

            let brew_install = Command::new("bash")
                .args(["-c", "/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status();

            match brew_install {
                Ok(s) if s.success() => {
                    println!("{}Homebrew installed", CHECK);
                }
                _ => {
                    println!("{}Could not install Homebrew", ERROR);
                    println!("  Install manually: https://brew.sh/");
                    return false;
                }
            }
        }

        println!("  Installing Railway via Homebrew...");
        let install = Command::new("brew")
            .args(["install", "railway"])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();

        match install {
            Ok(s) if s.success() => {
                println!("{}Railway CLI installed successfully!", CHECK);
                return true;
            }
            _ => {
                println!("{}Homebrew install failed", ERROR);
                println!("  Try: brew install railway");
                return false;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Linux: Use shell script
        println!("  Installing Railway via shell script...");
        let install = Command::new("bash")
            .args(["-c", "curl -fsSL https://railway.app/install.sh | sh"])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();

        match install {
            Ok(s) if s.success() => {
                println!("{}Railway CLI installed successfully!", CHECK);
                return true;
            }
            _ => {
                println!("{}Shell script install failed", ERROR);
                println!("  Try: curl -fsSL https://railway.app/install.sh | sh");
                return false;
            }
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        println!("{}Platform not supported for auto-install", ERROR);
        println!("  Install manually: https://docs.railway.com/guides/cli");
        return false;
    }
}

fn deploy_railway(verbose: bool) -> i32 {
    // Track deployment attempt (fire-and-forget, anonymous)
    analytics::track_deploy_attempt("railway");

    // Check and install Railway CLI
    if !check_railway_installed() {
        println!("→ Installing Railway CLI...");

        if !install_railway() {
            println!("{}Railway CLI installation failed", ERROR);
            println!("  Install manually: npm i -g @railway/cli");
            analytics::track_deploy_error("railway", "cli_install_failed");
            analytics::flush();
            return 1;
        }
        println!("{}Railway CLI installed", CHECK);
    }

    // Authenticate (silent check)
    let whoami = Command::new("railway")
        .arg("whoami")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if whoami.is_err() || !whoami.unwrap().success() {
        println!("→ Authenticating with Railway...");
        println!("  Note: Allow local network access in your browser when prompted");

        // Try browser login first
        let status = Command::new("railway")
            .arg("login")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();

        let browser_failed = status.is_err() || !status.unwrap().success();

        // Verify login actually worked (browser login may return success even if cancelled)
        let auth_check = Command::new("railway")
            .arg("whoami")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        let auth_succeeded = auth_check.is_ok() && auth_check.unwrap().success();

        // If browser login failed or auth didn't work, try browserless
        if browser_failed || !auth_succeeded {
            println!("  Browser login incomplete, trying token-based login...");
            println!("  Get your token from: https://railway.app/account/tokens");

            let browserless_status = Command::new("railway")
                .args(["login", "--browserless"])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status();

            if browserless_status.is_err() || !browserless_status.unwrap().success() {
                println!("{}Authentication failed", ERROR);
                analytics::track_deploy_error("railway", "auth_failed");
                analytics::flush();
                return 1;
            }
        }
    }
    println!("{}Authenticated with Railway", CHECK);

    // Check if linked to a project (first deploy check)
    let status_check = Command::new("railway")
        .arg("status")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let is_first_deploy = match status_check {
        Ok(s) => !s.success(),
        Err(_) => true,
    };

    if is_first_deploy {
        // Get project name from directory
        let current_dir = env::current_dir().unwrap_or_default();
        let project_name = current_dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "doo-app".to_string());

        print!("→ Creating Railway project...");
        io::stdout().flush().ok();

        let status = Command::new("railway")
            .args(["init", "-n", &project_name])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();

        match status {
            Ok(s) if s.success() => {
                println!("{}Railway project created", CHECK);
            }
            _ => {
                println!("{}Failed to create Railway project", ERROR);
                println!("  Manage billing: https://railway.app/account/billing");
                analytics::track_deploy_error("railway", "project_creation_failed");
                analytics::flush();
                return 1;
            }
        }

        // Link the project
        println!("→ Linking project...");

        let link_status = Command::new("railway")
            .arg("link")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();

        if link_status.is_ok() && link_status.unwrap().success() {
            println!("{}Project linked", CHECK);
        } else {
            println!("{}Link manually if needed: railway link", INFO);
        }

        // Ask for secrets on first deploy only
        let env_vars = prompt_env_vars_for_production();
        if !env_vars.is_empty() {
            println!("→ Setting environment variables...");

            for (key, value) in &env_vars {
                let var_string = format!("{}={}", key, value);
                let status = Command::new("railway")
                    .args(["variables", "set", &var_string])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();

                match status {
                    Ok(s) if s.success() => {
                        println!("  {}{} set", CHECK, key);
                    }
                    _ => {
                        println!("  {}Failed to set {}", INFO, key);
                    }
                }
            }
        }
    }

    // Deploy
    println!();
    println!("→ Deploying to Railway...");

    let start = std::time::Instant::now();

    let status = Command::new("railway")
        .arg("up")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    let duration = start.elapsed();

    match status {
        Ok(s) if s.success() => {
            println!(
                "\r{}Deployed successfully ({:.1}s)",
                CHECK,
                duration.as_secs_f64()
            );

            println!();
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("  ✨ Deployed! Run 'railway open' to view");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!();

            // Track deploy success
            analytics::track_deploy_success("railway", duration.as_millis() as u64);
            analytics::flush();

            0
        }
        Ok(_) => {
            println!("\r{}Deployment failed", ERROR);
            println!();
            println!("  Troubleshooting:");
            println!("  • Run with --verbose to see full logs");
            println!("  • Check logs: railway logs");
            println!("  • Build locally: docker build .");

            // Track deploy error
            analytics::track_deploy_error("railway", "build_failed");
            analytics::flush();

            1
        }
        Err(e) => {
            println!("\r{}Deployment error: {}", ERROR, e);
            println!();
            println!("  Check your network connection and try again.");

            // Track deploy error
            analytics::track_deploy_error("railway", "network_error");
            analytics::flush();

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
    if let Err(e) =
        download_and_upgrade(&install_dir, platform, &latest_version, latest_version_num)
    {
        eprintln!("{} Upgrade failed: {}", ERROR, e);
        return 1;
    }

    println!(
        "\n{} Upgrade complete! You now have doo v{}",
        SPARKLE, latest_version_num
    );
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
    let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).ok()?;

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
    fs::create_dir_all(&temp_dir).map_err(|e| format!("Failed to create temp directory: {}", e))?;

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
            .args([
                "-q",
                "-o",
                &zip_path.to_string_lossy(),
                "-d",
                &extract_dir.to_string_lossy(),
            ])
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

    // Backup and replace files
    for entry in
        fs::read_dir(&source_dir).map_err(|e| format!("Failed to read source directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();
        let dest_path = install_dir.join(&file_name);

        // Only update release files, skip unknown files
        if !is_release_file(&file_name_str) && !entry.path().is_dir() {
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
        let _ = Command::new("chmod")
            .args(["+x", &doo_path.to_string_lossy()])
            .status();
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
    for entry in
        fs::read_dir(extract_dir).map_err(|e| format!("Failed to read extract directory: {}", e))?
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
    for entry in
        fs::read_dir(extract_dir).map_err(|e| format!("Failed to read extract directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        if entry.path().is_dir() {
            return Ok(entry.path());
        }
    }

    Err("Could not find extracted files".to_string())
}

/// Check if a file matches release file patterns (dynamic, no hardcoded list)
/// Matches: doo*, libdoo*, std folder
fn is_release_file(file_name: &str) -> bool {
    // Match any file starting with "doo" (doo.exe, doo.dll, doo_http.dll, doo_db.so, etc.)
    // Match any file starting with "libdoo" (libdoo.so, libdoo_http.dylib, etc.)
    // Match the "std" folder
    file_name.starts_with("doo")
        || file_name.starts_with("libdoo")
        || file_name == "std"
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
