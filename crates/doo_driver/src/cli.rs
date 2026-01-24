//! CLI Module
//!
//! Command line interface for the Doo compiler.

use std::path::PathBuf;
use clap::{Parser, Subcommand};
use anyhow::Result;

use crate::compile::{compile_file, CompileOptions};

/// Doo programming language compiler
#[derive(Parser, Debug)]
#[command(name = "doo")]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Available commands
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run a Doo file
    Run {
        /// Source file to run
        file: PathBuf,
        
        /// Keep LLVM IR file
        #[arg(long)]
        keep_ll: bool,
        
        /// Run database migrations
        #[arg(long)]
        migrate: bool,
    },
    
    /// Build a Doo file to executable
    Build {
        /// Source file to build
        file: PathBuf,
        
        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,
        
        /// Keep intermediate files
        #[arg(long)]
        keep_intermediates: bool,
    },
    
    /// Run database migrations only
    Migrate {
        /// Source file with models
        file: PathBuf,
    },
    
    /// Check a Doo file for errors
    Check {
        /// Source file to check
        file: PathBuf,
    },
    
    /// Format a Doo file
    Fmt {
        /// Source file to format
        file: PathBuf,
    },
    
    /// Show version information
    Version,
}

/// Run the CLI
pub fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Run { file, keep_ll, migrate } => {
            let options = CompileOptions {
                keep_intermediates: keep_ll,
                migrate,
                ..Default::default()
            };
            let result = compile_file(&file, &options)?;
            
            if result.success {
                println!("✓ Compiled successfully");
                // Would execute the binary here
            } else {
                for err in &result.errors {
                    eprintln!("❌ {}", err);
                }
                std::process::exit(1);
            }
        }
        
        Commands::Build { file, output, keep_intermediates } => {
            let options = CompileOptions {
                keep_intermediates,
                output: output.map(|p| p.to_string_lossy().to_string()),
                ..Default::default()
            };
            let result = compile_file(&file, &options)?;
            
            if result.success {
                if let Some(out) = result.output_path {
                    println!("✓ Built: {}", out);
                }
            } else {
                for err in &result.errors {
                    eprintln!("❌ {}", err);
                }
                std::process::exit(1);
            }
        }
        
        Commands::Migrate { file } => {
            let options = CompileOptions {
                migrate: true,
                ..Default::default()
            };
            let result = compile_file(&file, &options)?;
            
            if result.success {
                println!("✓ Migrations complete");
            } else {
                for err in &result.errors {
                    eprintln!("❌ {}", err);
                }
                std::process::exit(1);
            }
        }
        
        Commands::Check { file } => {
            let options = CompileOptions::default();
            let result = compile_file(&file, &options)?;
            
            if result.success {
                println!("✓ No errors found");
            } else {
                for err in &result.errors {
                    eprintln!("❌ {}", err);
                }
                std::process::exit(1);
            }
        }
        
        Commands::Fmt { file: _ } => {
            println!("Format command not yet implemented");
        }
        
        Commands::Version => {
            println!("doo {}", env!("CARGO_PKG_VERSION"));
        }
    }
    
    Ok(())
}
