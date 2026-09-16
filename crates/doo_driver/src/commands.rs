//! Command implementations for the `doo` CLI.

use crate::cli::Cli;
use crate::compile::{compile_project, CompileOptions, CompileResult};
use doo_core::errors::codes::{CompilerError, ErrorCode};
use doo_core::Span;
use std::path::PathBuf;

fn make_opts(path: &str, output: Option<&str>, cli: &Cli, check_only: bool) -> CompileOptions {
    CompileOptions {
        input_path: PathBuf::from(path),
        output_name: output.unwrap_or("output").to_string(),
        dev_mode: cli.debug,
        print_ast: cli.print.as_deref() == Some("ast"),
        print_hir: cli.print.as_deref() == Some("hir"),
        print_mir: cli.print.as_deref() == Some("mir"),
        keep_ll: cli.emit.iter().any(|e| e == "llvm-ir"),
        keep_obj: cli.emit.iter().any(|e| e == "obj"),
        check_only,
        show_warnings: cli.show_warnings,
        timings: cli.time_passes,
    }
}

fn to_errors(msg: String) -> Vec<CompilerError> {
    vec![CompilerError::new(ErrorCode::LlvmError, msg, Span::dummy())]
}

pub fn build(path: &str, output: Option<&str>, cli: &Cli) -> Result<PathBuf, Vec<CompilerError>> {
    let opts = make_opts(path, output, cli, false);
    let result = compile_project(opts).map_err(|e| to_errors(e))?;

    if result.success {
        if let Some(exe) = result.exe_path {
            return Ok(exe);
        }
    }

    Err(to_errors(format!(
        "compilation failed: {} error(s)",
        result.error_count
    )))
}

pub fn run(path: &str, args: &[String], cli: &Cli) -> Result<i32, Vec<CompilerError>> {
    let opts = make_opts(path, None, cli, false);
    let result = compile_project(opts).map_err(|e| to_errors(e))?;

    if !result.success || result.exe_path.is_none() {
        return Err(to_errors(format!(
            "compilation failed: {} error(s)",
            result.error_count
        )));
    }

    let exe = result.exe_path.unwrap();
    let status = std::process::Command::new(&exe)
        .args(args)
        .status()
        .map_err(|e| to_errors(format!("failed to execute binary: {}", e)))?;

    Ok(status.code().unwrap_or(1))
}

pub fn check(path: &str, cli: &Cli) -> Result<(), Vec<CompilerError>> {
    let opts = make_opts(path, None, cli, true);
    let result = compile_project(opts).map_err(|e| to_errors(e))?;

    if result.success {
        Ok(())
    } else {
        Err(to_errors(format!(
            "type check failed: {} error(s)",
            result.error_count
        )))
    }
}

pub fn explain(error_code: &str) -> Result<(), Vec<CompilerError>> {
    let upper = error_code.to_uppercase();
    let desc = match upper.as_str() {
        "E0001" | "UNDEFINED_VARIABLE" => "A variable was referenced before declaration.",
        "E0002" | "TYPE_MISMATCH" => "Types do not match in an assignment, argument, or return.",
        "E0011" | "NAME_ALREADY_DEFINED" => "A name was declared more than once in the same scope.",
        "E0012" | "MODULE_NOT_FOUND" => "A module referenced by `use` was not found.",
        "E0900" | "LLVM_ERROR" => "An LLVM codegen or linking error occurred.",
        "E0901" | "CODEGEN_FAILED" => "Code generation failed for a MIR instruction.",
        _ => "No description available for this error code.",
    };
    println!("{} — {}", error_code, desc);
    Ok(())
}

pub fn clean(path: &str) -> Result<(), Vec<CompilerError>> {
    let root = PathBuf::from(path);
    let targets = [
        root.join("target"),
        root.join(".doo-cache"),
        root.join("doo-cache"),
    ];

    for target in &targets {
        if target.exists() {
            if target.is_dir() {
                std::fs::remove_dir_all(target).ok();
            } else {
                std::fs::remove_file(target).ok();
            }
            eprintln!("Removed: {}", target.display());
        }
    }

    if root.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(ext) = p.extension() {
                    if ext == "ll" || ext == "o" || ext == "bc" {
                        std::fs::remove_file(&p).ok();
                        eprintln!("Removed: {}", p.display());
                    }
                }
            }
        }
    }

    Ok(())
}
