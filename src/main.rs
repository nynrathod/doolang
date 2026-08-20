use clap::Parser;
use doo_driver::{
    check_command, clean_command, compile_project, discover_main_doo_candidates, explain_error,
    initialize, run_command_with_compiler, Cli, Commands, CompileOptions,
};
use std::path::PathBuf;

fn main() {
    initialize();

    let cli = Cli::parse();

    // Debug system initialization
    // Debug builds: always enabled. Release: only with --debug flag.
    let debug_enabled = cli.debug;
    if debug_enabled {
        std::env::set_var("DOO_DEBUG", "1");
    }
    if cli.show_warnings {
        std::env::set_var("DOO_SHOW_WARNINGS", "1");
    }
    doo_core::debug::init(debug_enabled);

    // Extract global flags before matching (avoids borrow issues)
    let keep_ll = cli.emit.iter().any(|e| e == "llvm-ir");
    let keep_obj = cli.emit.iter().any(|e| e == "obj");
    let print_ast = cli.print.as_deref() == Some("ast");
    let print_hir = cli.print.as_deref() == Some("hir");
    let print_mir = cli.print.as_deref() == Some("mir");
    let show_warnings = cli.show_warnings;
    let timings = cli.time_passes;
    let verbose = cli.verbose;

    let exit_code = match cli.command {
        Commands::Build { path, output } => {
            let opts = CompileOptions {
                input_path: PathBuf::from(path),
                output_name: output.unwrap_or_else(|| "output".to_string()),
                dev_mode: false,
                print_ast,
                print_hir,
                print_mir,
                keep_ll,
                keep_obj,
                check_only: false,
                show_warnings,
                timings,
            };
            match compile_project(opts) {
                Ok(result) => {
                    if result.error_count > 0 {
                        eprintln!("Build failed with {} error(s)", result.error_count);
                        1
                    } else if result.success {
                        if let Some(exe) = result.exe_path {
                            println!("Build successful: {}", exe.display());
                        }
                        0
                    } else {
                        eprintln!("Build failed");
                        1
                    }
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    1
                }
            }
        }
        Commands::Run { path, args } => {
            if verbose {
                std::env::set_var("DOO_VERBOSE", "1");
            }
            run_command_with_compiler(
                PathBuf::from(path),
                keep_ll,
                debug_enabled,
                args,
                compile_project,
                discover_main_doo_candidates,
            )
        }
        Commands::Check { path } => check_command(PathBuf::from(path)),
        Commands::Explain { error } => {
            explain_error(&error);
            0
        }
        Commands::Clean { path } => clean_command(PathBuf::from(path)),
    };

    std::process::exit(exit_code);
}
