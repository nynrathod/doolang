pub mod analytics;
pub mod templates;

use clap::{Parser, Subcommand};
use console::Term;
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

        /// Enable debug output (temporary, session-only)
        #[arg(long)]
        debug: bool,

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
        None => run_cli(Cli {
            command: Some(Commands::Run {
                path: PathBuf::from("."),
                keep_ll: false,
                debug: false,
                args: Vec::new(),
            }),
        }),
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
            debug,
            args,
        }) => {
            let mut path = path;

            if env::var("DOO_ENTRY").is_err() && !path.is_file() {
                let candidates = doo::compiler::discover_main_doo_candidates(&path, 4, 25);
                if candidates.len() == 1 {
                    path = candidates[0].clone();
                } else if candidates.len() > 1 {
                    let is_interactive = Term::stdout().is_term();
                    if is_interactive {
                        let display_items: Vec<String> =
                            candidates.iter().map(|p| p.display().to_string()).collect();

                        let idx = match Select::with_theme(&ColorfulTheme::default())
                            .with_prompt("Multiple projects found. Select one to run")
                            .items(&display_items)
                            .default(0)
                            .interact()
                        {
                            Ok(i) => i,
                            Err(_) => {
                                eprintln!("{} Failed to select project", ERROR);
                                return 1;
                            }
                        };

                        if let Some(selected) = candidates.get(idx) {
                            path = selected.clone();
                        }
                    } else {
                        // Non-interactive (e.g. IDE task running `doo`): pick a deterministic default
                        // to avoid failing with "main.doo not found" when run from repo root.
                        let search_root = path.clone();
                        let mut best: Option<(usize, String, PathBuf)> = None;
                        for c in &candidates {
                            let rel_depth = c
                                .strip_prefix(&search_root)
                                .ok()
                                .map(|p| p.components().count())
                                .unwrap_or_else(|| c.components().count());
                            let key_str = c.display().to_string();
                            match &best {
                                None => best = Some((rel_depth, key_str, c.clone())),
                                Some((best_depth, best_str, _)) => {
                                    if rel_depth < *best_depth
                                        || (rel_depth == *best_depth && key_str < *best_str)
                                    {
                                        best = Some((rel_depth, key_str, c.clone()));
                                    }
                                }
                            }
                        }

                        if let Some((_, _, chosen)) = best {
                            path = chosen;
                        }
                    }
                }
            }

            // Set debug mode based on CLI flag or build type
            let debug_enabled = debug || cfg!(debug_assertions);

            // Determine the working directory for the user's project (for .env, etc.)
            let run_root: PathBuf = if path.is_file() {
                let parent = path.parent().unwrap_or_else(|| Path::new("."));
                let parent = if parent.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    parent
                };
                parent.to_path_buf()
            } else {
                path.clone()
            };

            let temp_name = format!("temp_doo_{}", std::process::id());

            // Find the compiler's build directory for target-linux/target-windows
            // Priority: DOO_BUILD_ROOT env var > walk up to find Cargo.toml > ~/.doo fallback
            let doo_compiler_root = if let Ok(build_root) = env::var("DOO_BUILD_ROOT") {
                // Use explicit build root if set
                PathBuf::from(build_root)
            } else if let Ok(exe_path) = env::current_exe() {
                let mut search_path = exe_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf();

                // Walk up to find Cargo.toml (for development builds)
                let mut found_cargo_root = None;
                loop {
                    if search_path.join("Cargo.toml").exists() {
                        found_cargo_root = Some(search_path.clone());
                        break;
                    }

                    match search_path.parent() {
                        Some(parent) if parent != search_path => {
                            search_path = parent.to_path_buf();
                        }
                        _ => break,
                    }
                }

                // If Cargo.toml found, use it; otherwise use ~/.doo for installed binaries
                if let Some(root) = found_cargo_root {
                    root
                } else {
                    // For installed binaries, use ~/.doo directory
                    let home_dir = env::var("HOME")
                        .or_else(|_| env::var("USERPROFILE"))
                        .unwrap_or_else(|_| String::from("."));
                    PathBuf::from(home_dir).join(".doo")
                }
            } else {
                // Last resort fallback
                let home_dir = env::var("HOME")
                    .or_else(|_| env::var("USERPROFILE"))
                    .unwrap_or_else(|_| String::from("."));
                PathBuf::from(home_dir).join(".doo")
            };

            // Use target/{platform}/release for temporary binaries
            // This is the standard layout used by cargo and other build systems
            let target_root = if cfg!(windows) {
                "target-windows"
            } else if cfg!(target_os = "macos") {
                "target"
            } else {
                "target-linux"
            };
            let base_is_already_target_root = doo_compiler_root
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == target_root)
                .unwrap_or(false);
            let target_dir = if base_is_already_target_root {
                doo_compiler_root.join("release")
            } else {
                doo_compiler_root.join(target_root).join("release")
            };
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

            let compile_start = std::time::Instant::now();
            // println!("{} Compiling...", INFO);

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
                    let compile_ms = compile_start.elapsed().as_millis();
                    // println!("{} Compiled in {} ms", CHECK, compile_ms);
                    // println!("{} Starting...", INFO);
                    let _ = std::io::stdout().flush();
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

            let exe_full_path = target_dir.join(&exe_name);

            use std::sync::{Arc, Mutex};

            // Build command with DOO_DEBUG env var if debug is enabled
            let mut cmd = Command::new(&exe_full_path);
            cmd.args(&args)
                .current_dir(&run_root)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());

            // Ensure the child process can find the correct FFI DLLs.
            // On Windows, DLL resolution depends heavily on PATH and the exe directory.
            // We prepend the compiler's release output directories so the temp exe always
            // loads the freshly built doo_*.dll without copying files around.
            #[cfg(windows)]
            {
                let mut dll_dirs: Vec<std::path::PathBuf> = Vec::new();

                // Main doo build output
                dll_dirs.push(doo_compiler_root.join("target-windows").join("release"));
                dll_dirs.push(
                    doo_compiler_root
                        .join("target-windows")
                        .join("release")
                        .join("deps"),
                );

                // Extra: any ffi_libs/*/target/release for local development
                let ffi_root = doo_compiler_root.join("ffi_libs");
                if let Ok(entries) = std::fs::read_dir(&ffi_root) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            let rel = path.join("target").join("release");
                            if rel.exists() {
                                dll_dirs.push(rel);
                            }
                        }
                    }
                }

                dll_dirs.retain(|p| p.exists());

                let path_sep = ';';
                let mut new_path = String::new();
                for d in dll_dirs {
                    if let Some(s) = d.to_str() {
                        new_path.push_str(s);
                        new_path.push(path_sep);
                    }
                }
                if let Ok(existing) = std::env::var("PATH") {
                    new_path.push_str(&existing);
                }
                if !new_path.is_empty() {
                    cmd.env("PATH", new_path);
                }
            }

            let env_file = run_root.join(".env");
            let env_vars = read_env_vars_from_file(&env_file);
            for (key, value) in env_vars {
                if env::var(&key).is_err() {
                    cmd.env(key, value);
                }
            }

            // Pass DOO_DEBUG to subprocess
            if debug_enabled {
                cmd.env("DOO_DEBUG", "1");
            }

            let child = cmd.spawn();

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
        "Render",
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
        "Render" => deploy_render(verbose),
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
    read_env_vars_from_file(Path::new(".env"))
}

fn strip_surrounding_quotes(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let first = value.as_bytes()[0];
        let last = value.as_bytes()[value.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

fn read_env_vars_from_file(env_path: &Path) -> Vec<(String, String)> {
    let mut vars = Vec::new();

    if let Ok(content) = fs::read_to_string(env_path) {
        for line in content.lines() {
            let line = line.trim();
            // Skip comments and empty lines
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim().trim_start_matches("export ").trim().to_string();
                let value = strip_surrounding_quotes(value);
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

// ============================================================================
// RENDER DEPLOYMENT
// ============================================================================

fn deploy_render(_verbose: bool) -> i32 {
    // Track deployment attempt
    analytics::track_deploy_attempt("render");

    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  🚀 Render Deployment Prep");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    // 3. Git Status Check & Monorepo Support
    let mut git_root = None;
    if let Ok(current_dir) = env::current_dir() {
        let mut dir = current_dir.as_path();
        loop {
            if dir.join(".git").exists() {
                git_root = Some(dir.to_path_buf());
                break;
            }
            if let Some(parent) = dir.parent() {
                dir = parent;
            } else {
                break;
            }
        }
    }
    
    let is_git = git_root.is_some();
    
    if !is_git {
        println!();
        println!("{} Warning: Project is not a git repository", INFO);
        println!("   Render Blueprints require a git repository.");
    } else {
        println!("{}Git repository detected", CHECK);
    }

    // 1. Check/Create render.yaml (Moved after git check for monorepo support)
    if let Some(root) = &git_root {
        let render_yaml = root.join("render.yaml");
        if !render_yaml.exists() {
            println!("→ Creating render.yaml in git root...");
            
            // Calculate relative path for rootDir
            let current_dir = env::current_dir().unwrap_or_default();
            let relative_path = current_dir.strip_prefix(root).unwrap_or(Path::new(""));
            let relative_str = relative_path.to_string_lossy();
            
            // Prepare content with rootDir if needed
            let mut content = templates::RENDER_YAML_CONTENT.to_string();
            
            if !relative_str.is_empty() && relative_str != "." {
                // Determine indentation - RENDER_YAML_CONTENT uses 2 spaces
                // We need to insert `rootDir: relative_path` under the service definition
                // The template is:
                // services:
                //   - type: web
                //     name: doo-app
                // ...
                
                // Naive insertion: find "name: doo-app" and insert rootDir before it
                // Better: find "type: web" and insert after
                if let Some(pos) = content.find("type: web") {
                    // Find the end of the line
                    if let Some(newline_pos) = content[pos..].find('\n') {
                        let insert_pos = pos + newline_pos + 1;
                        // Assuming 4 space indent for properties under list item in current template
                        let root_dir_line = format!("    rootDir: {}\n", relative_str.replace('\\', "/"));
                        content.insert_str(insert_pos, &root_dir_line);
                        println!("   (Monorepo detected: set rootDir to '{}')", relative_str.replace('\\', "/"));
                    }
                }
            }

            if let Err(e) = fs::write(render_yaml, content) {
                println!("{}Failed to create render.yaml: {}", ERROR, e);
                return 1;
            }
            println!("{}render.yaml created", CHECK);
        } else {
            println!("{}render.yaml exists in git root", CHECK);
        }
    } else {
        // Fallback for non-git users (just current dir)
        let render_yaml = Path::new("render.yaml");
        if !render_yaml.exists() {
             println!("→ Creating render.yaml...");
             use templates::RENDER_YAML_CONTENT;
             if let Err(e) = fs::write(render_yaml, RENDER_YAML_CONTENT) {
                 println!("{}Failed to create render.yaml: {}", ERROR, e);
                 return 1;
             }
             println!("{}render.yaml created", CHECK);
        }
    }

    // 2. Check/Create Dockerfile (Always in current dir)
    use templates::DOCKERFILE_CONTENT;
    let dockerfile = Path::new("Dockerfile");
    if !dockerfile.exists() {
        println!("→ Creating Dockerfile...");
        if let Err(e) = fs::write(dockerfile, DOCKERFILE_CONTENT) {
            println!("{}Failed to create Dockerfile: {}", ERROR, e);
            return 1;
        }
        println!("{}Dockerfile created", CHECK);
    } else {
        println!("{}Dockerfile exists", CHECK);
    }

    println!();
    println!("✅ Configuration generated!");
    println!();
    println!("👉 Next Steps to Deploy:");
    println!("   1. Push code to GitHub/GitLab:");
    if !is_git {
        println!("      git init");
        println!("      git add .");
        println!("      git commit -m \"Initial commit\"");
        println!("      git remote add origin <your-repo-url>");
        println!("      git push -u origin main");
    } else {
        println!("      git add .");
        println!("      git commit -m \"Add Render config\"");
        println!("      git push");
    }
    println!();
    println!("   2. Connect to Render:");
    println!("      Open https://dashboard.render.com/blueprints/new");
    println!("      Select your repository and click 'Connect'.");
    println!();
    println!("   Render will automatically detect render.yaml and deploy.");
    println!();

    // Track success (prep phase)
    analytics::track_deploy_success("render", 0);
    analytics::flush();

    0
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
    file_name.starts_with("doo") || file_name.starts_with("libdoo") || file_name == "std"
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
