//! CLI argument parsing using clap.
//!
//! Pure compiler commands — no deploy, migrate, or init templates.

use clap::{Parser, Subcommand};

/// Doolang compiler.
#[derive(Parser)]
#[command(
    name = "doo",
    version,
    about = "Doolang compiler — compile and run Doolang programs"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Optimization level (0, 1, 2, 3, s, z).
    #[arg(short = 'O', long = "opt-level", default_value = "3", global = true)]
    pub opt_level: String,

    /// Generate DWARF debug information.
    #[arg(long = "debug", global = true)]
    pub debug: bool,

    /// Verbose output.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Additional library search paths.
    #[arg(short = 'L', long = "lib-path", global = true)]
    pub lib_paths: Vec<String>,

    /// Emit intermediate files (llvm-ir, obj, dep-info).
    #[arg(long = "emit", value_name = "FORMAT", global = true)]
    pub emit: Vec<String>,

    /// Print intermediate representation (ast, hir, mir).
    #[arg(long = "print", value_name = "KIND", global = true)]
    pub print: Option<String>,

    /// Print per-pass timing information.
    #[arg(long = "time-passes", global = true)]
    pub time_passes: bool,

    /// Show warnings.
    #[arg(long = "warnings", global = true)]
    pub show_warnings: bool,
}

/// Compiler subcommands.
#[derive(Subcommand)]
pub enum Commands {
    /// Compile to a native binary.
    Build {
        /// Source file or project directory.
        #[arg(default_value = ".")]
        path: String,

        /// Output file name.
        #[arg(short = 'o', long = "output")]
        output: Option<String>,
    },

    /// Compile and immediately execute.
    Run {
        /// Source file or project directory.
        #[arg(default_value = ".")]
        path: String,

        /// Arguments to pass to the compiled program.
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// Type-check only — no codegen or linking.
    Check {
        /// Source file or project directory.
        #[arg(default_value = ".")]
        path: String,
    },

    /// Explain an error code.
    Explain {
        /// Error code to explain (e.g. "E0001").
        error: String,
    },

    /// Remove build artifacts and incremental cache.
    Clean {
        /// Path to clean (defaults to current directory).
        #[arg(default_value = ".")]
        path: String,
    },
}
