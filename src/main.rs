use clap::Parser;
use doo_driver::{
    compile_project, discover_main_doo_candidates, initialize, Cli, Commands, CompileOptions,
    CompileResult,
};

fn main() {
    // Initialize driver
    initialize();

    let cli = Cli::parse();

    // Handle --explain flag
    if let Some(code) = &cli.explain {
        doo_driver::explain_error(code);
        std::process::exit(0);
    }

    // Initialize centralized debug system
    // Debug builds: always enabled. Release: only with --debug flag.
    let debug_enabled =
        cli.debug || matches!(&cli.command, Some(Commands::Run { debug: true, .. }));
    if debug_enabled {
        std::env::set_var("DOO_DEBUG", "1"); // Propagate to child processes (FFI runtime)
    }
    if cli.warn {
        std::env::set_var("DOO_SHOW_WARNINGS", "1");
    }
    doo_core::debug::init(debug_enabled);

    // Route commands
    let exit_code = match cli.command {
        None => run_command(std::path::PathBuf::from("."), false, false, Vec::new()),
        Some(Commands::Build {
            path,
            output,
            keep_ll,
            keep_obj,
            print_ast,
            print_hir,
            print_mir,
        }) => build_command(
            path, output, keep_ll, keep_obj, print_ast, print_hir, print_mir,
        ),
        Some(Commands::Run {
            path,
            keep_ll,
            debug,
            args,
        }) => run_command(path, keep_ll, debug, args),
        Some(Commands::Check { path }) => check_command(path),
        Some(Commands::Migrate { path, dry_run }) => migrate_command(path, dry_run),
        Some(Commands::Init { name, template }) => doo_driver::run_init(name, template),
        Some(Commands::Deploy { verbose }) => doo_driver::run_deploy(verbose),
        Some(Commands::Upgrade) => doo_driver::run_upgrade(),
    };

    std::process::exit(exit_code);
}

fn build_command(
    path: std::path::PathBuf,
    output: String,
    keep_ll: bool,
    keep_obj: bool,
    print_ast: bool,
    print_hir: bool,
    print_mir: bool,
) -> i32 {
    let opts = CompileOptions {
        input_path: path,
        output_name: output.clone(),
        dev_mode: false,
        print_ast,
        print_hir,
        print_mir,
        keep_ll,
        keep_obj,
        check_only: false,
        show_warnings: std::env::var("DOO_SHOW_WARNINGS").is_ok(),
    };
    match compile_project(opts) {
        Ok(result) => {
            if result.error_count > 0 {
                eprintln!("Build failed with {} error(s)", result.error_count);
                1
            } else if result.success {
                println!("Build successful: {}", output);
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

fn run_command(path: std::path::PathBuf, keep_ll: bool, debug: bool, args: Vec<String>) -> i32 {
    doo_driver::run_command_with_compiler(
        path,
        keep_ll,
        debug,
        args,
        compile_project,
        discover_main_doo_candidates,
    )
}

fn check_command(path: std::path::PathBuf) -> i32 {
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
            if result.error_count > 0 {
                eprintln!("Finding errors... Found {}", result.error_count);
                1
            } else {
                println!("✓ No errors found");
                0
            }
        }
        Err(e) => {
            eprintln!("✗ Failed to check: {}", e);
            1
        }
    }
}

fn migrate_command(_path: std::path::PathBuf, dry_run: bool) -> i32 {
    eprintln!("Migrate command not yet implemented");
    if dry_run {
        eprintln!("  Dry-run mode");
    }
    1
}
