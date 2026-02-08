//! # Doo Driver
//!
//! Compiler driver - Phase 10: Clean compilation orchestration.
//! Single source of truth for all compilation commands.

pub mod analytics;
pub mod cli;
pub mod commands;
pub mod compile;
pub mod loader;
pub mod templates;

use console::Term;
use dialoguer::{theme::ColorfulTheme, Select};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

pub use cli::{Cli, Commands};
pub use commands::{run_deploy, run_init, run_upgrade};
pub use compile::{compile_project, discover_main_doo_candidates, CompileOptions, CompileResult};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const ERROR: &str = "✗ ";
const CHECK: &str = "✓ ";

// Type aliases for backward compatibility with callback-based API
pub type CompileFn = fn(CompileOptions) -> Result<CompileResult, String>;
pub type DiscoverFn = fn(&Path, usize, usize) -> Vec<PathBuf>;

pub fn initialize() {
    #[cfg(target_os = "windows")]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn SetConsoleOutputCP(code_page: u32) -> i32;
        }
        unsafe {
            SetConsoleOutputCP(65001);
        }
    }
}

pub fn run_command_with_compiler(
    mut path: PathBuf,
    keep_ll: bool,
    debug: bool,
    args: Vec<String>,
    compile_fn: CompileFn,
    discover_fn: DiscoverFn,
) -> i32 {
    // Multiple project detection
    if env::var("DOO_ENTRY").is_err() && !path.is_file() {
        let candidates = discover_fn(&path, 4, 25);
        if candidates.len() == 1 {
            path = candidates[0].clone();
        } else if candidates.len() > 1 {
            let is_interactive = Term::stdout().is_term();
            if is_interactive {
                let display_items: Vec<String> = candidates
                    .iter()
                    .map(|p: &PathBuf| p.display().to_string())
                    .collect();

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
                // Non-interactive: pick deterministic default
                let search_root = path.clone();
                let mut best: Option<(usize, String, PathBuf)> = None;
                for c in &candidates {
                    let rel_depth = c
                        .strip_prefix(&search_root)
                        .ok()
                        .map(|p: &Path| p.components().count())
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

    let debug_enabled = debug || cfg!(debug_assertions);

    // Determine working directory for .env loading
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

    // Find compiler build directory: DOO_BUILD_ROOT > Cargo.toml walk-up > ~/.doo
    let doo_compiler_root = if let Ok(build_root) = env::var("DOO_BUILD_ROOT") {
        PathBuf::from(build_root)
    } else if let Ok(exe_path) = env::current_exe() {
        let mut search_path = exe_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

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

        if let Some(root) = found_cargo_root {
            root
        } else {
            let home_dir = env::var("HOME")
                .or_else(|_| env::var("USERPROFILE"))
                .unwrap_or_else(|_| String::from("."));
            PathBuf::from(home_dir).join(".doo")
        }
    } else {
        let home_dir = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .unwrap_or_else(|_| String::from("."));
        PathBuf::from(home_dir).join(".doo")
    };

    // Platform-specific target directory
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
        let _ = fs::create_dir_all(&target_dir);
    }

    let output_path_buf = target_dir.join(&temp_name);
    let output_name = output_path_buf.to_string_lossy().to_string();

    let opts = CompileOptions {
        input_path: path.clone(),
        output_name: output_name.clone(),
        dev_mode: false,
        print_ast: false,
        print_hir: false,
        print_mir: false,
        keep_ll,
        keep_obj: false,
        check_only: false,
        show_warnings: std::env::var("DOO_SHOW_WARNINGS").is_ok(),
    };

    let compile_start = std::time::Instant::now();

    match compile_fn(opts) {
        Ok(result) => {
            if result.error_count > 0 || !result.success {
                eprintln!(
                    "{} Compilation failed with {} errors",
                    ERROR, result.error_count
                );
                cleanup_temp(&output_name);
                return 1;
            }
            let compile_ms = compile_start.elapsed().as_millis();
            if debug_enabled {
                println!("{} Compiled in {} ms", CHECK, compile_ms);
            }
            let _ = std::io::stdout().flush();
        }
        Err(e) => {
            eprintln!("{} Failed to compile: {}", ERROR, e);
            cleanup_temp(&output_name);
            return 1;
        }
    }

    let exe_name = if cfg!(windows) {
        format!("{}.exe", temp_name)
    } else {
        temp_name.clone()
    };

    let exe_full_path = target_dir.join(&exe_name);

    // Build command with full environment setup
    let mut cmd = Command::new(&exe_full_path);
    cmd.args(&args)
        .current_dir(&run_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    // Windows DLL path management
    #[cfg(windows)]
    {
        let mut dll_dirs: Vec<PathBuf> = Vec::new();

        dll_dirs.push(doo_compiler_root.join("target-windows").join("release"));
        dll_dirs.push(
            doo_compiler_root
                .join("target-windows")
                .join("release")
                .join("deps"),
        );

        let ffi_root = doo_compiler_root.join("ffi_libs");
        if let Ok(entries) = fs::read_dir(&ffi_root) {
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
        if let Ok(existing) = env::var("PATH") {
            new_path.push_str(&existing);
        }
        if !new_path.is_empty() {
            cmd.env("PATH", new_path);
        }
    }

    // Load .env file
    let env_file = run_root.join(".env");
    let env_vars = read_env_vars_from_file(&env_file);
    for (key, value) in env_vars {
        if env::var(&key).is_err() {
            cmd.env(key, value);
        }
    }

    if debug_enabled {
        cmd.env("DOO_DEBUG", "1");
    }

    let child = cmd.spawn();

    let code = match child {
        Ok(child) => {
            // Ctrl+C handler
            let child_process = Arc::new(Mutex::new(Some(child)));
            let child_process_clone = child_process.clone();

            let _ = ctrlc::set_handler(move || {
                if let Ok(mut guard) = child_process_clone.lock() {
                    if let Some(child) = guard.as_mut() {
                        let _ = child.kill();
                    }
                }
                std::process::exit(130);
            });

            // Poll-based wait to allow Ctrl+C handler to work
            let result = loop {
                {
                    let mut guard = child_process.lock().unwrap();
                    if let Some(c) = guard.as_mut() {
                        if let Ok(Some(status)) = c.try_wait() {
                            break status.code().unwrap_or(1);
                        }
                    } else {
                        break 1;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            };

            let _ = fs::remove_file(&exe_full_path);
            result
        }
        Err(e) => {
            eprintln!("{} Failed to start process: {}", ERROR, e);
            let _ = fs::remove_file(&exe_full_path);
            1
        }
    };

    let _ = fs::remove_file(&exe_full_path);
    code
}

/// Check a Doo project for errors without compiling.
pub fn check_command(path: PathBuf) -> i32 {
    let opts = CompileOptions {
        input_path: path,
        output_name: "output".to_string(),
        dev_mode: false,
        print_ast: false,
        print_hir: false,
        print_mir: false,
        keep_ll: false,
        keep_obj: false,
        check_only: true,
        show_warnings: std::env::var("DOO_SHOW_WARNINGS").is_ok(),
    };

    match compile_project(opts) {
        Ok(result) => {
            if result.error_count > 0 || !result.success {
                eprintln!("{} Check failed with {} errors", ERROR, result.error_count);
                1
            } else {
                println!("{} No errors found", CHECK);
                0
            }
        }
        Err(e) => {
            eprintln!("{} Check failed: {}", ERROR, e);
            1
        }
    }
}

pub fn migrate_command(_path: PathBuf, dry_run: bool) -> i32 {
    eprintln!("Migrate command not yet implemented");
    if dry_run {
        eprintln!("  Dry-run mode");
    }
    1
}

pub fn explain_error(code: &str) {
    use doo_core::errors::codes::ErrorCode;

    if let Some(ec) = ErrorCode::from_code(code) {
        let mut emitter = doo_diagnostics::DiagnosticEmitter::new(true);
        let _ = emitter.explain_code(ec);
    } else {
        eprintln!("error: unknown error code `{}`", code);
        eprintln!("  Use `doo --explain E0100` with a valid code");
    }
}

fn read_env_vars_from_file(env_path: &Path) -> Vec<(String, String)> {
    let mut vars = Vec::new();

    if let Ok(content) = fs::read_to_string(env_path) {
        for line in content.lines() {
            let line = line.trim();
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

fn cleanup_temp(output_name: &str) {
    let _ = fs::remove_file(output_name);
    if cfg!(windows) {
        let _ = fs::remove_file(format!("{}.exe", output_name));
    }
}
