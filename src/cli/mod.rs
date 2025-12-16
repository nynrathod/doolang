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

            let opts = CompileOptions {
                input_path: path.clone(),
                output_name: temp_name.clone(),
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
                        let _ = std::fs::remove_file(&temp_name);
                        return 1;
                    }
                    copy_dlls_if_needed();
                }
                Err(e) => {
                    eprintln!("{} Failed to compile: {}", ERROR, e);
                    let _ = std::fs::remove_file(&temp_name);
                    return 1;
                }
            }

            let exe_name = if cfg!(windows) {
                format!("{}.exe", temp_name)
            } else {
                temp_name.clone()
            };
            let exe_path = match std::env::current_dir() {
                Ok(dir) => dir.join(&exe_name),
                Err(_) => {
                    eprintln!("{} Error: Could not determine current directory", ERROR);
                    return 1;
                }
            };

            let status = Command::new(&exe_path)
                .args(&args)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status();

            let code = match status {
                Ok(s) => {
                    let code = s.code().unwrap_or(1);
                    if !s.success() {
                        let _ = std::fs::remove_file(&exe_path);
                    }
                    code
                }
                Err(e) => {
                    eprintln!("{} Failed to start process: {}", ERROR, e);
                    let _ = std::fs::remove_file(&exe_path);
                    1
                }
            };
            let _ = std::fs::remove_file(&exe_path);
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
    }
}

// Helper to copy DLLs on Windows
fn copy_dlls_if_needed() {
    #[cfg(target_os = "windows")]
    {
        if let Ok(current_dir) = std::env::current_dir() {
            let target_release = current_dir.join("target").join("release");
            if target_release.exists() {
                let dll_names = [
                    "doo.dll",
                    "doo_http.dll",
                    "doo_runtime.dll",
                    "doo_auth.dll",
                    "doo_db.dll",
                    "doo_file.dll",
                ];
                for dll_name in &dll_names {
                    let dll_src = target_release.join(dll_name);
                    if dll_src.exists() {
                        let dll_dest = current_dir.join(dll_name);
                        let _ = std::fs::copy(&dll_src, &dll_dest);
                    }
                }
            }
        }
    }
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
