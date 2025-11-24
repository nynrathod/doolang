mod cli;

use clap::Parser;
use cli::{run_cli, Cli};

use doo::compiler::{compile_project, CompileOptions};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn main() {
    // Enable UTF-8 console output on Windows
    #[cfg(target_os = "windows")]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn SetConsoleOutputCP(code_page: u32) -> i32;
        }
        unsafe {
            SetConsoleOutputCP(65001); // 65001 is UTF-8
        }
    }
    // If no subcommand is provided, default to dev-mode compilation and run (for cargo run)
    let cli = Cli::parse();

    if cli.command.is_none() {
        // Dev mode: compile and run the project as in the old workflow
        let opts = CompileOptions {
            input_path: PathBuf::from("."),
            output_name: "output".to_string(),
            dev_mode: true,
            print_ast: true,
            print_mir: true,
            keep_ll: true,
            keep_obj: false,
            check_only: false,
        };

        match compile_project(opts) {
            Ok(result) => {
                if result.success {
                    if let Some(exe_path) = result.exe_path {
                        let status = Command::new(&exe_path)
                            .stdin(Stdio::inherit())
                            .stdout(Stdio::inherit())
                            .stderr(Stdio::inherit())
                            .status();
                        let _ = std::fs::remove_file(&exe_path);
                        std::process::exit(status.map(|s| s.code().unwrap_or(0)).unwrap_or(1));
                    }
                    std::process::exit(0);
                } else {
                    eprintln!("✗ Compilation failed with {} error(s)", result.error_count);
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        // Use CLI logic for subcommands
        let exit_code = run_cli(cli);
        std::process::exit(exit_code);
    }
}
