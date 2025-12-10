// Hybrid linking: Embedded LLD for Windows, Clang for Unix

use crate::analyzer::types::SemanticError;
use crate::analyzer::SemanticAnalyzer;
use crate::codegen::core::CodeGen;
use crate::diagnostics::{print_grouped, DiagnosticRecord};
use crate::lexer::lexer::lex;
use crate::mir::builder::MirBuilder;
use crate::parser::{ParseError, Parser};
use bumpalo::Bump;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::OptimizationLevel;
use regex::Regex;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

// Embed linker for Windows only
#[cfg(target_os = "windows")]
const EMBEDDED_LINKER: &[u8] = include_bytes!("../linkers/lld-link.exe");

#[cfg(target_os = "windows")]
fn extract_embedded_linker() -> Result<PathBuf, String> {
    let temp_dir = env::temp_dir();
    let linker_path = temp_dir.join("doo_lld-link.exe");

    let should_write = if linker_path.exists() {
        fs::metadata(&linker_path)
            .map(|m| m.len() != EMBEDDED_LINKER.len() as u64)
            .unwrap_or(true)
    } else {
        true
    };

    if should_write {
        let mut file = fs::File::create(&linker_path)
            .map_err(|e| format!("Failed to create linker file: {}", e))?;
        file.write_all(EMBEDDED_LINKER)
            .map_err(|e| format!("Failed to link (Windows): {}", e))?;
    }

    Ok(linker_path)
}

pub struct CompileOptions {
    pub input_path: PathBuf,
    pub output_name: String,
    pub dev_mode: bool,
    pub print_ast: bool,
    pub print_mir: bool,
    pub keep_ll: bool,
    pub keep_obj: bool,
    pub check_only: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            input_path: PathBuf::from("."),
            output_name: "output".to_string(),
            dev_mode: cfg!(debug_assertions),
            print_ast: false,
            print_mir: false,
            keep_ll: false,
            keep_obj: false,
            check_only: false,
        }
    }
}

pub struct CompileResult {
    pub success: bool,
    pub error_count: usize,
    pub exe_path: Option<PathBuf>,
}

pub fn compile_project(opts: CompileOptions) -> Result<CompileResult, String> {
    let output_name = env::var("DOO_OUTPUT_NAME").unwrap_or(opts.output_name);
    let check_only = env::var("DOO_CHECK_ONLY").is_ok() || opts.check_only;

    let opts = CompileOptions {
        output_name,
        check_only,
        ..opts
    };

    let input_path = if opts.input_path.is_file() {
        opts.input_path.clone()
    } else {
        // Try main.doo in the specified directory
        let main_file = opts.input_path.join("main.doo");
        if main_file.exists() {
            main_file
        } else {
            // Try src/main.doo if not found in root
            let src_main_file = opts.input_path.join("src").join("main.doo");
            if src_main_file.exists() {
                src_main_file
            } else {
                return Err(format!(
                    "Error: main.doo not found in {} or {}/src",
                    opts.input_path.display(),
                    opts.input_path.display()
                ));
            }
        }
    };

    let input = fs::read_to_string(&input_path)
        .map_err(|e| format!("Failed to read {}: {}", input_path.display(), e))?;

    let project_root = input_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let arena = Bump::new();
    let tokens = lex(&input, &arena);
    let mut parser = Parser::new(&tokens);

    let mut diagnostics: Vec<DiagnosticRecord> = Vec::new();
    let mut error_count = 0;
    let mut sources = HashMap::new();

    let mut statements = Vec::new();
    while parser.current < parser.tokens.len() {
        match parser.parse_statement() {
            Ok(stmt) => statements.push(stmt),
            Err(e) => {
                let (line, col, msg) = match &e {
                    ParseError::UnexpectedTokenAt { msg, line, col } => {
                        (Some(*line), Some(*col), msg.clone())
                    }
                    _ => (None, None, e.to_string()),
                };
                diagnostics.push(DiagnosticRecord {
                    filename: input_path.display().to_string(),
                    message: msg,
                    line,
                    col,
                    is_parse: true,
                });
                skip_to_next_statement(&mut parser);
                error_count += 1;
            }
        }
    }

    let mut analyzer = SemanticAnalyzer::new(Some(project_root.clone()));

    let is_circular_import = if let Err(e) = analyzer.analyze_program(&mut statements) {
        match &e {
            SemanticError::ParseErrorInModule { file, error } => {
                let re = Regex::new(r"at (\d+):(\d+): (.+)").expect("Regex pattern is valid");
                let (line, col, msg) = if let Some(caps) = re.captures(error) {
                    (
                        caps.get(1).and_then(|m| m.as_str().parse().ok()),
                        caps.get(2).and_then(|m| m.as_str().parse().ok()),
                        caps.get(3)
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_else(|| error.clone()),
                    )
                } else {
                    (None, None, error.clone())
                };
                diagnostics.push(DiagnosticRecord {
                    filename: file.clone(),
                    message: msg,
                    line,
                    col,
                    is_parse: true,
                });
                if !sources.contains_key(file) {
                    if let Ok(src) = std::fs::read_to_string(file) {
                        sources.insert(file.clone(), src);
                    }
                }
                error_count += 1;
                false
            }
            SemanticError::CircularImport { .. } => {
                diagnostics.push(DiagnosticRecord {
                    filename: input_path.display().to_string(),
                    message: e.to_string(),
                    line: None,
                    col: None,
                    is_parse: false,
                });
                error_count += 1;
                true
            }
            _ => {
                diagnostics.push(DiagnosticRecord {
                    filename: input_path.display().to_string(),
                    message: e.to_string(),
                    line: None,
                    col: None,
                    is_parse: false,
                });
                error_count += 1;
                false
            }
        }
    } else {
        false
    };

    // Skip processing collected_errors if circular import was detected
    if !is_circular_import {
        for error in &analyzer.collected_errors {
            match error {
                SemanticError::ParseErrorInModule {
                    file,
                    error: err_msg,
                } => {
                    let re = Regex::new(r"at (\d+):(\d+): (.+)").expect("Regex pattern is valid");
                    let (line, col, msg) = if let Some(caps) = re.captures(err_msg) {
                        (
                            caps.get(1).and_then(|m| m.as_str().parse().ok()),
                            caps.get(2).and_then(|m| m.as_str().parse().ok()),
                            caps.get(3)
                                .map(|m| m.as_str().to_string())
                                .unwrap_or_else(|| err_msg.clone()),
                        )
                    } else {
                        (None, None, err_msg.clone())
                    };
                    diagnostics.push(DiagnosticRecord {
                        filename: file.clone(),
                        message: msg,
                        line,
                        col,
                        is_parse: true,
                    });
                    if !sources.contains_key(file) {
                        if let Ok(src) = std::fs::read_to_string(file) {
                            sources.insert(file.clone(), src);
                        }
                    }
                    error_count += 1;
                }
                _ => {
                    diagnostics.push(DiagnosticRecord {
                        filename: input_path.display().to_string(),
                        message: error.to_string(),
                        line: None,
                        col: None,
                        is_parse: false,
                    });
                    error_count += 1;
                }
            }
        }
    }

    if !diagnostics.is_empty() {
        sources.insert(input_path.display().to_string(), input.clone());
        for diag in &diagnostics {
            if !sources.contains_key(&diag.filename) {
                if let Ok(src) = std::fs::read_to_string(&diag.filename) {
                    sources.insert(diag.filename.clone(), src);
                }
            }
        }
        print_grouped(&diagnostics, &sources);
    }

    if error_count > 0 {
        if opts.dev_mode {}
        return Ok(CompileResult {
            success: false,
            error_count,
            exe_path: None,
        });
    }

    if opts.check_only {
        return Ok(CompileResult {
            success: error_count == 0,
            error_count,
            exe_path: None,
        });
    }

    // Merge imported structs and functions into the AST for MIR generation
    let mut all_nodes = analyzer.imported_structs.clone();
    all_nodes.extend(analyzer.imported_functions.clone());
    all_nodes.extend(statements);

    if opts.print_ast {}

    let mut mir_builder = MirBuilder::new();
    mir_builder.enum_table = std::sync::Arc::new(analyzer.enum_table.clone());
    mir_builder.enum_variant_order = std::sync::Arc::new(analyzer.enum_variant_order.clone());
    mir_builder.function_table = std::sync::Arc::new(analyzer.function_table.clone());
    mir_builder.method_table = std::sync::Arc::new(analyzer.method_table.clone());
    mir_builder.struct_table = std::sync::Arc::new(analyzer.struct_table.clone());
    mir_builder.set_is_main_entry(true); // Mark this as the main entry point
    mir_builder.build_program(&all_nodes);
    mir_builder.finalize();

    // Validate MIR before codegen
    if let Err(e) = mir_builder.program.validate() {
        return Err(format!("MIR validation failed: {}", e));
    }

    // Check that main() function exists before code generation
    let has_main = mir_builder
        .program
        .functions
        .iter()
        .any(|f| f.name == "main");
    if !has_main {
        return Err("Error: main() function not found. Every program must have a main() function as the entry point.".to_string());
    }

    if opts.print_mir || opts.dev_mode {}

    let context = inkwell::context::Context::create();
    let mut codegen = CodeGen::new("main_module", &context);
    codegen.function_aliases = analyzer.function_aliases.clone();
    codegen.struct_field_decorators = analyzer.struct_field_decorators.clone();

    // Detect if HTTP module is imported to enable HTTP-specific code generation
    codegen.uses_http = analyzer
        .imported_modules
        .keys()
        .any(|k| k.contains("std/Http") || k.contains("std\\Http") || k.contains("Http.doo"));

    codegen.generate_program(&mir_builder.program);

    if opts.dev_mode {
        codegen.dump();
    }

    if opts.keep_ll {
        let llvm_ir = codegen.module.print_to_string();
        let ll_file = format!("{}.ll", opts.output_name);
        fs::write(&ll_file, llvm_ir.to_string())
            .map_err(|e| format!("Failed to write LLVM IR: {}", e))?;
    }

    let current_dir =
        env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?;

    let exe_name = if cfg!(windows) {
        format!("{}.exe", opts.output_name)
    } else {
        opts.output_name.clone()
    };
    let exe_path = current_dir.join(&exe_name);

    compile_to_native(&codegen, &opts, &exe_path, &mir_builder.program)?;

    if !exe_path.exists() {
        return Ok(CompileResult {
            success: false,
            error_count: 0,
            exe_path: None,
        });
    } else {
    }

    Ok(CompileResult {
        success: true,
        error_count: 0,
        exe_path: Some(exe_path),
    })
}

fn compile_to_native(
    codegen: &CodeGen,
    opts: &CompileOptions,
    exe_path: &Path,
    mir_program: &crate::mir::mir::MirProgram,
) -> Result<(), String> {
    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("Failed to initialize target: {}", e))?;

    let triple = TargetMachine::get_default_triple();
    let cpu = TargetMachine::get_host_cpu_name().to_string();
    let features = TargetMachine::get_host_cpu_features().to_string();

    let target =
        Target::from_triple(&triple).map_err(|e| format!("Failed to create target: {}", e))?;

    let target_machine = target
        .create_target_machine(
            &triple,
            &cpu,
            &features,
            OptimizationLevel::Aggressive,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or("Failed to create target machine")?;

    let obj_file = format!("{}.o", opts.output_name);
    if let Err(e) = codegen.module.verify() {
        return Err(format!(
            "LLVM Module verification failed: {}",
            e.to_string()
        ));
    }
    target_machine
        .write_to_file(&codegen.module, FileType::Object, Path::new(&obj_file))
        .map_err(|e| format!("Failed to write object file: {}", e))?;

    let exe_path_str = exe_path
        .to_str()
        .ok_or_else(|| "Could not convert executable path to string".to_string())?;
    link_object_file(
        &obj_file,
        exe_path.to_str().unwrap(),
        opts.dev_mode,
        mir_program,
    )?;

    // Always remove .o file after linking unless keep_obj is true
    if !opts.keep_obj {
        if fs::remove_file(&obj_file).is_err() && opts.dev_mode {
            eprintln!("Warning: failed to remove object file {}", obj_file);
        }
    }

    Ok(())
}

fn link_object_file(
    obj_file: &str,
    output: &str,
    _dev_mode: bool,
    mir_program: &crate::mir::mir::MirProgram,
) -> Result<(), String> {
    // Collect all FFI libraries needed from the MIR
    let mut ffi_libs: std::collections::HashSet<String> = mir_program
        .functions
        .iter()
        .filter_map(|f| f.ffi_lib.clone())
        .collect();

    // Runtime functions (hash_string, json_*, file_*, panic_runtime) are now
    // compiled into libdoo.dylib (the main compiler library).
    // Always include it so compiled programs can access these runtime functions.
    ffi_libs.insert("doo".to_string());

    // Common: Build search paths for libraries (works on all platforms)
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let cwd = env::current_dir().ok();

    let mut search_paths: Vec<PathBuf> = Vec::new();

    // 1. Same directory as doo executable
    if let Some(ref dir) = exe_dir {
        search_paths.push(dir.clone());
    }

    // 2. target/release and target/debug (for development)
    if let Some(ref c) = cwd {
        search_paths.push(c.join("target").join("release"));
        search_paths.push(c.join("target").join("debug"));
    }

    // 3. System locations (Unix only, but harmless on Windows)
    search_paths.push(PathBuf::from("/usr/local/lib"));
    search_paths.push(PathBuf::from("/usr/lib"));

    // 4. User home lib directory and doo installation directory
    if let Ok(home) = env::var("HOME") {
        let home_path = PathBuf::from(&home);
        // Primary: ~/.local/bin/doo (where xtask installs on Linux/macOS)
        search_paths.push(home_path.join(".local").join("bin").join("doo"));
        // Fallback: ~/.local/lib
        search_paths.push(home_path.join(".local").join("lib"));
    }
    if let Ok(home) = env::var("USERPROFILE") {
        search_paths.push(PathBuf::from(home).join(".local").join("lib"));
    }

    // Helper: Find FFI library in search paths
    let find_ffi_lib = |lib_name: &str, paths: &[PathBuf]| -> Option<(PathBuf, PathBuf)> {
        for search_path in paths {
            // Windows: .dll.lib or .lib
            #[cfg(target_os = "windows")]
            {
                let dll_lib = search_path.join(format!("{}.dll.lib", lib_name));
                if dll_lib.exists() {
                    return Some((search_path.clone(), dll_lib));
                }
                let lib = search_path.join(format!("{}.lib", lib_name));
                if lib.exists() {
                    return Some((search_path.clone(), lib));
                }
            }
            // Unix: lib*.so, lib*.dylib, lib*.a
            #[cfg(not(target_os = "windows"))]
            {
                let so = search_path.join(format!("lib{}.so", lib_name));
                if so.exists() {
                    return Some((search_path.clone(), so));
                }
                let dylib = search_path.join(format!("lib{}.dylib", lib_name));
                if dylib.exists() {
                    return Some((search_path.clone(), dylib));
                }
                let a = search_path.join(format!("lib{}.a", lib_name));
                if a.exists() {
                    return Some((search_path.clone(), a));
                }
            }
        }
        None
    };

    // Also search in ffi_libs directory structure
    let mut ffi_search_paths = search_paths.clone();
    if let Some(ref c) = cwd {
        for ffi_lib in &ffi_libs {
            let ffi_dir = c
                .join("ffi_libs")
                .join(format!("lib{}", ffi_lib))
                .join("target")
                .join("release");
            if ffi_dir.exists() {
                ffi_search_paths.push(ffi_dir);
            }
        }
    }

    // ========== WINDOWS LINKING ==========
    #[cfg(target_os = "windows")]
    {
        let linker = extract_embedded_linker()?;
        let sdk_paths = find_windows_sdk_paths();

        let mut cmd = Command::new(&linker);
        cmd.arg(format!("/OUT:{}", output))
            .arg(obj_file)
            .arg("/SUBSYSTEM:CONSOLE")
            .arg("/ENTRY:main");

        // Add Windows SDK paths
        if let Some(paths) = sdk_paths {
            if let Some(ucrt) = paths.ucrt_lib {
                cmd.arg(format!("/LIBPATH:{}", ucrt));
            }
            if let Some(um) = paths.um_lib {
                cmd.arg(format!("/LIBPATH:{}", um));
            }
            if let Some(msvc) = paths.msvc_lib {
                cmd.arg(format!("/LIBPATH:{}", msvc));
            }
            cmd.arg("ucrt.lib")
                .arg("vcruntime.lib")
                .arg("legacy_stdio_definitions.lib")
                .arg("libcmt.lib");
        }

        // Link FFI libraries
        let mut added_paths = std::collections::HashSet::new();
        for ffi_lib in &ffi_libs {
            if let Some((lib_dir, lib_file)) = find_ffi_lib(ffi_lib, &ffi_search_paths) {
                // Add library path if not already added
                if added_paths.insert(lib_dir.clone()) {
                    cmd.arg(format!("/LIBPATH:{}", lib_dir.display()));
                }
                // Add Windows system libs needed by Rust FFI
                cmd.arg("ws2_32.lib")
                    .arg("userenv.lib")
                    .arg("bcrypt.lib")
                    .arg("kernel32.lib")
                    .arg("advapi32.lib");
                // Link the library
                cmd.arg(lib_file.to_str().unwrap());
            } else {
                // FFI library not found - provide helpful error
                return Err(format!(
                    "FFI library '{}.dll.lib' not found.\n\
                    Build it first:\n\
                      cd ffi_libs\\lib{} && cargo build --release\n\
                    Then rebuild doo:\n\
                      cargo build --release\n\
                    \n\
                    Searched in: {:?}",
                    ffi_lib, ffi_lib, ffi_search_paths
                ));
            }
        }

        let result = cmd.output();
        return match result {
            Ok(r) if r.status.success() => Ok(()),
            Ok(r) => Err(format!(
                "Linking failed:\nSTDOUT:\n{}\nSTDERR:\n{}",
                String::from_utf8_lossy(&r.stdout),
                String::from_utf8_lossy(&r.stderr)
            )),
            Err(e) => Err(format!("Linker error: {}", e)),
        };
    }

    // ========== UNIX LINKING (Linux & macOS) ==========
    #[cfg(not(target_os = "windows"))]
    {
        // Check for clang
        if Command::new("clang").arg("--version").output().is_err() {
            return Err("Clang not found. Install with:\n\
                - Ubuntu/Debian: sudo apt install clang\n\
                - Fedora: sudo dnf install clang\n\
                - macOS: xcode-select --install"
                .to_string());
        }

        let mut cmd = Command::new("clang");
        cmd.arg(obj_file).arg("-o").arg(output).arg("-lm");

        // Platform-specific system libraries
        #[cfg(target_os = "linux")]
        cmd.arg("-lpthread").arg("-ldl");

        #[cfg(target_os = "macos")]
        {
            cmd.arg("-lpthread");
            cmd.arg("-framework").arg("Security");
            cmd.arg("-framework").arg("CoreFoundation");
        }

        // Link FFI libraries
        let mut added_paths = std::collections::HashSet::new();
        for ffi_lib in &ffi_libs {
            if let Some((lib_dir, lib_file)) = find_ffi_lib(ffi_lib, &ffi_search_paths) {
                // For shared libraries, add -L and -l flags with rpath
                let is_shared = lib_file
                    .extension()
                    .map(|e| e == "so" || e == "dylib")
                    .unwrap_or(false);

                if is_shared {
                    if added_paths.insert(lib_dir.clone()) {
                        cmd.arg(format!("-L{}", lib_dir.display()));
                        cmd.arg(format!("-Wl,-rpath,{}", lib_dir.display()));
                    }
                    cmd.arg(format!("-l{}", ffi_lib));
                } else {
                    // Static library - link directly
                    cmd.arg(lib_file.to_str().unwrap());
                }
            } else {
                // FFI library not found - provide helpful error
                let lib_name = format!("lib{}.so", ffi_lib);
                #[cfg(target_os = "macos")]
                let lib_name = format!("lib{}.dylib", ffi_lib);

                return Err(format!(
                    "FFI library '{}' not found.\n\
                    Build it first:\n\
                      cd ffi_libs/lib{} && cargo build --release\n\
                    Then rebuild doo:\n\
                      cargo build --release\n\
                    \n\
                    Searched in: {:?}",
                    lib_name, ffi_lib, ffi_search_paths
                ));
            }
        }

        let result = cmd.output();
        return match result {
            Ok(r) if r.status.success() => Ok(()),
            Ok(r) => Err(format!(
                "Linking failed:\n{}",
                String::from_utf8_lossy(&r.stderr)
            )),
            Err(e) => Err(format!("Linker error: {}", e)),
        };
    }
}

#[cfg(target_os = "windows")]
struct WindowsSdkPaths {
    ucrt_lib: Option<String>,
    um_lib: Option<String>,
    msvc_lib: Option<String>,
}

#[cfg(target_os = "windows")]
fn find_windows_sdk_paths() -> Option<WindowsSdkPaths> {
    let program_files_x86 = env::var("ProgramFiles(x86)").ok()?;
    let kits_base = format!("{}\\Windows Kits\\10\\Lib", program_files_x86);
    let kits_path = Path::new(&kits_base);

    let ucrt_lib = if kits_path.exists() {
        if let Ok(entries) = fs::read_dir(kits_path) {
            let mut versions: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            versions.sort();
            versions
                .last()
                .map(|v| format!("{}\\{}\\ucrt\\x64", kits_base, v))
        } else {
            None
        }
    } else {
        None
    };

    let um_lib = ucrt_lib.as_ref().map(|u| u.replace("ucrt", "um"));

    let msvc_base = format!("{}\\Microsoft Visual Studio", program_files_x86);
    let msvc_lib = find_msvc_lib_path(&msvc_base);

    Some(WindowsSdkPaths {
        ucrt_lib,
        um_lib,
        msvc_lib,
    })
}

#[cfg(target_os = "windows")]
fn find_msvc_lib_path(base: &str) -> Option<String> {
    let base_path = Path::new(base);
    for year in &["2022", "2019", "2017"] {
        for edition in &["BuildTools", "Community", "Professional", "Enterprise"] {
            let vc_path = base_path.join(year).join(edition).join("VC\\Tools\\MSVC");
            if let Ok(entries) = fs::read_dir(&vc_path) {
                if let Some(version) = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .max()
                {
                    return Some(format!("{}\\{}\\lib\\x64", vc_path.display(), version));
                }
            }
        }
    }
    None
}

fn skip_to_next_statement(parser: &mut Parser) {
    while parser.current < parser.tokens.len() {
        if let Some(tok) = parser.peek() {
            if matches!(tok.kind, crate::lexer::token::TokenType::Semi) {
                parser.advance();
                break;
            }
        }
        parser.advance();
    }
}
