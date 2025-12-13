use clap::{Parser, Subcommand};
use std::path::PathBuf;

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

/// Entrypoint for CLI logic.
/// Returns exit code (0 for success, nonzero for error).
pub fn run_cli(cli: Cli) -> i32 {
    use doo::compiler::{compile_project, CompileOptions};
    use std::process::Command;

    match cli.command {
        None => {
            println!("🎉 doo CLI - doo language tool");
            println!("Type `doo --help` for usage");
            0
        }
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
                        eprintln!("Build failed with {} errors", result.error_count);
                        return 1;
                    } else if result.success {
                        // Copy DLLs next to executable on Windows
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
                        println!("✓ Build successful: {}", output);
                        return 0;
                    } else {
                        eprintln!("Build failed");
                        return 1;
                    }
                }
                Err(e) => {
                    eprintln!("{}", e);
                    return 1;
                }
            }
        }
        Some(Commands::Run {
            path,
            keep_ll,
            args,
        }) => {
            // Generate unique temp binary name
            let temp_name = format!("temp_doo_{}", std::process::id());

            // Compile to temp binary, pass temp object name as env var
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

            // Actually compile
            match compile_project(opts) {
                Ok(result) => {
                    if result.error_count > 0 || !result.success {
                        eprintln!("Compilation failed with {} errors", result.error_count);
                        let _ = std::fs::remove_file(&temp_name);
                        return 1;
                    }

                    // Copy DLLs next to executable on Windows
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
                Err(e) => {
                    eprintln!("Failed to compile: {}", e);
                    let _ = std::fs::remove_file(&temp_name);
                    return 1;
                }
            }

            // Run the temp binary
            let exe_name = if cfg!(windows) {
                format!("{}.exe", temp_name)
            } else {
                temp_name.clone()
            };
            let exe_path = match std::env::current_dir() {
                Ok(dir) => dir.join(&exe_name),
                Err(_) => {
                    eprintln!("Error: Could not determine current directory");
                    return 1;
                }
            };

            // Run the temp binary and stream output directly to terminal
            use std::process::Stdio;
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
                    eprintln!("Failed to start process: {}", e);
                    let _ = std::fs::remove_file(&exe_path);
                    1
                }
            };
            // Always attempt to delete the temp binary after running, regardless of success/failure
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
                        println!("Found {} errors", result.error_count);
                        return 1;
                    } else {
                        println!("✓ No errors found");
                        return 0;
                    }
                }
                Err(e) => {
                    eprintln!("Failed to check: {}", e);
                    return 1;
                }
            }
        }
    }
}
