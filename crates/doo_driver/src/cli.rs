//! CLI structures for the driver.
//!
//! Single source of truth for CLI commands and flags.
//! Phase 10: Clean compilation orchestration with debug/explain support.

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

    /// Enable debug output
    #[arg(long, global = true)]
    pub debug: bool,

    /// Show compiler warnings (suppressed by default)
    #[arg(long, short = 'W', global = true)]
    pub warn: bool,

    /// Explain error codes in detail
    #[arg(long, global = true)]
    pub explain: Option<String>,
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

        /// Keep the object file (.o)
        #[arg(long)]
        keep_obj: bool,

        /// Print AST (debug)
        #[arg(long)]
        print_ast: bool,

        /// Print HIR (debug)
        #[arg(long)]
        print_hir: bool,

        /// Print MIR (debug)
        #[arg(long)]
        print_mir: bool,
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

    /// Run database migrations
    Migrate {
        /// Path to the project directory or main.doo file with models
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Show migration SQL without executing
        #[arg(long)]
        dry_run: bool,
    },

    /// Upgrade doo to the latest version
    Upgrade,
}
