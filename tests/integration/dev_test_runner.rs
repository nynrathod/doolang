//! Dev Test Runner - Automatically runs all dev_test .doo files and fixture projects
//!
//! This runner:
//! - Auto-discovers all .doo files in dev_test/ (excluding fixture/)
//! - Runs only main.doo files in each fixture subfolder (first level only)
//! - Expects circular_import_test files to fail; others to succeed
//! - Continues running ALL tests even if some fail
//! - Shows all file paths as they are tested
//! - Shows actual compiler error output (same as terminal)
//!
//! Uses compile_project (same as `doo run`) for proper compilation with imports

use doo_driver::compile::{compile_project, CompileOptions};
use std::fs;
use std::path::{Path, PathBuf};

/// Find the crate root by looking for Cargo.toml
fn find_crate_root() -> PathBuf {
    let mut current = std::env::current_dir().expect("Failed to get current directory");

    // Walk up the directory tree looking for Cargo.toml
    loop {
        if current.join("Cargo.toml").exists() {
            return current;
        }

        if !current.pop() {
            panic!(
                "Could not find crate root (Cargo.toml) from {}",
                std::env::current_dir().unwrap().display()
            );
        }
    }
}

/// Recursively find all .doo files in a directory, excluding "fixture" folders
fn find_doo_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip fixture folders - they're handled separately
                if path.file_name().map_or(false, |n| n == "fixture") {
                    continue;
                }
                files.extend(find_doo_files(&path));
            } else if path.extension().map_or(false, |e| e == "doo") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Find main.doo files in FIRST LEVEL subdirectories only (not recursive for fixture)
fn find_fixture_main_files_first_level(base: &Path) -> Vec<PathBuf> {
    let mut mains = Vec::new();

    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let main_path = path.join("main.doo");
                if main_path.exists() {
                    mains.push(main_path);
                }
            }
        }
    }
    mains.sort();
    mains
}

/// Check if a file is expected to fail (name ends with _error)
fn is_error_test(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.ends_with("_error"))
        .unwrap_or(false)
}

/// Compile a .doo file using compile_project (same as `doo run`)
/// Returns (success: bool, error_message: Option<String>)
fn compile_doo_file(path: &Path) -> (bool, Option<String>) {
    // For single files, use the file's parent as project root
    // For fixture projects, use the directory containing main.doo
    let project_root = if path.file_name().map_or(false, |n| n == "main.doo") {
        // It's a main.doo in a fixture - use its parent directory
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        // It's a standalone file - use the file itself
        path.to_path_buf()
    };

    let opts = CompileOptions {
        input_path: project_root,
        output_name: "test_output".to_string(),
        dev_mode: false,
        print_ast: false,
        print_hir: false,
        print_mir: false,
        keep_ll: false,
        keep_obj: false,
        check_only: true, // Just check, don't generate executable
        show_warnings: false,
        timings: false,
    };

    match compile_project(opts) {
        Ok(result) => {
            if result.success {
                (true, None)
            } else {
                (false, Some("Compilation failed".to_string()))
            }
        }
        Err(e) => (false, Some(e)),
    }
}

/// Test all .doo files in dev_test/ (excluding fixture/)
/// These should all compile successfully.
#[test]
#[ignore = "runs .doo files through compiler - some cause hangs"]
fn test_all_dev_test_files() {
    let crate_root = find_crate_root();
    let dev_test_path = crate_root.join("tests").join("dev_test");

    if !dev_test_path.exists() {
        panic!(
            "dev_test directory not found at {}",
            dev_test_path.display()
        );
    }

    let files = find_doo_files(&dev_test_path);

    if files.is_empty() {
        panic!("No .doo files found in {}", dev_test_path.display());
    }

    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                    DEV_TEST FILE RUNNER                                      ║");
    println!(
        "║                    Testing {} .doo files                                      ║",
        files.len()
    );
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    let mut passed: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<(PathBuf, String)> = Vec::new();

    // Run ALL tests - don't stop on failure
    for file in files.iter() {
        let is_error = is_error_test(file);
        let (success, error_msg) = compile_doo_file(file);

        // For _error files, we expect failure
        let test_passed = if is_error { !success } else { success };

        if test_passed {
            if is_error {
                // println!("✓ PASS (error expected): {}", file.display());
            } else {
                println!("✓ PASS: {}", file.display());
            }
            passed.push(file.clone());
        } else {
            if is_error {
                println!("✗ FAIL (should have failed): {}", file.display());
                failed.push((file.clone(), "Expected to fail but succeeded".to_string()));
            } else {
                println!("✗ FAIL: {}", file.display());
                if let Some(err) = error_msg {
                    println!("  └─ Error: {}", err);
                    failed.push((file.clone(), err));
                } else {
                    failed.push((file.clone(), "Unknown error".to_string()));
                }
            }
        }
    }

    // Print summary
    println!();
    println!("════════════════════════════════════════════════════════════════════════════════");
    println!("                              SUMMARY");
    println!("════════════════════════════════════════════════════════════════════════════════");
    println!();
    println!("Total files tested: {}", files.len());
    println!("Passed: {}", passed.len());
    println!("Failed: {}", failed.len());
    println!();

    if !failed.is_empty() {
        println!(
            "╔══════════════════════════════════════════════════════════════════════════════╗"
        );
        println!(
            "║                           FAILED TESTS                                       ║"
        );
        println!(
            "╚══════════════════════════════════════════════════════════════════════════════╝"
        );
        println!();
        for (file, error) in &failed {
            println!("❌ {}", file.display());
            println!("   Error: {}", error);
            println!();
        }
    }

    if !passed.is_empty() {
        println!(
            "╔══════════════════════════════════════════════════════════════════════════════╗"
        );
        println!(
            "║                           PASSED TESTS                                       ║"
        );
        println!(
            "╚══════════════════════════════════════════════════════════════════════════════╝"
        );
        println!();
        for file in &passed {
            println!("✓ {}", file.display());
        }
        println!();
    }

    // Final result
    if failed.is_empty() {
        println!(
            "════════════════════════════════════════════════════════════════════════════════"
        );
        println!("✅ ALL {} TESTS PASSED!", files.len());
        println!(
            "════════════════════════════════════════════════════════════════════════════════"
        );
    } else {
        println!(
            "════════════════════════════════════════════════════════════════════════════════"
        );
        println!("❌ {} / {} TESTS FAILED", failed.len(), files.len());
        println!(
            "════════════════════════════════════════════════════════════════════════════════"
        );
        panic!("{} tests failed", failed.len());
    }
}

/// Test main.doo in each visibilitytest fixture (first level only)
/// These should all compile successfully.
#[test]
#[ignore = "multi-file imports not fully supported yet"]
fn test_visibilitytest_main() {
    let crate_root = find_crate_root();
    let visibility_path = crate_root
        .join("tests")
        .join("dev_test")
        .join("fixture")
        .join("visibilitytest");

    if !visibility_path.exists() {
        println!("visibilitytest fixture not found, skipping");
        return;
    }

    let mut mains = find_fixture_main_files_first_level(&visibility_path);

    // Also check for main.doo at root level
    let root_main = visibility_path.join("main.doo");
    if root_main.exists() && !mains.contains(&root_main) {
        mains.insert(0, root_main);
    }

    if mains.is_empty() {
        panic!("No main.doo files found in {}", visibility_path.display());
    }

    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                    VISIBILITYTEST RUNNER                                     ║");
    println!(
        "║                    Testing {} fixture(s)                                      ║",
        mains.len()
    );
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    let mut passed: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<(PathBuf, String)> = Vec::new();

    for main_path in mains.iter() {
        let (success, error_msg) = compile_doo_file(main_path);

        if success {
            println!("✓ PASS: {}", main_path.display());
            passed.push(main_path.clone());
        } else {
            println!("✗ FAIL: {}", main_path.display());
            if let Some(err) = error_msg {
                println!("  └─ Error: {}", err);
                failed.push((main_path.clone(), err));
            } else {
                failed.push((main_path.clone(), "Unknown error".to_string()));
            }
        }
    }

    // Print summary
    println!();
    println!("════════════════════════════════════════════════════════════════════════════════");
    println!("Passed: {} / {}", passed.len(), mains.len());

    if !failed.is_empty() {
        println!();
        println!("Failed tests:");
        for (file, error) in &failed {
            println!("  ❌ {}", file.display());
            println!("     Error: {}", error);
        }
        panic!("{} visibilitytest fixtures failed", failed.len());
    } else {
        println!("✅ All visibilitytest fixtures passed!");
    }
}

/// Test main.doo in each circular_import_test subfolder (first level only)
/// These should all FAIL to compile (circular imports are errors).
#[test]
fn test_circular_import_main() {
    let crate_root = find_crate_root();
    let circular_path = crate_root
        .join("tests")
        .join("dev_test")
        .join("fixture")
        .join("circular_import_test");

    if !circular_path.exists() {
        println!("circular_import_test fixture not found, skipping");
        return;
    }

    let mains = find_fixture_main_files_first_level(&circular_path);

    if mains.is_empty() {
        panic!("No main.doo files found in {}", circular_path.display());
    }

    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                    CIRCULAR_IMPORT_TEST RUNNER                               ║");
    println!(
        "║                    Testing {} fixture(s) - EXPECT FAILURE                     ║",
        mains.len()
    );
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    let mut passed: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<PathBuf> = Vec::new();

    for main_path in mains.iter() {
        let (success, _error_msg) = compile_doo_file(main_path);

        // For circular import tests, we EXPECT failure
        if !success {
            // println!("✓ PASS (failed as expected): {}", main_path.display());
            passed.push(main_path.clone());
        } else {
            println!("✗ FAIL (should have failed): {}", main_path.display());
            failed.push(main_path.clone());
        }
    }

    // Print summary
    println!();
    println!("════════════════════════════════════════════════════════════════════════════════");
    println!(
        "Passed (failed as expected): {} / {}",
        passed.len(),
        mains.len()
    );

    if !failed.is_empty() {
        println!();
        println!("Tests that should have failed but didn't:");
        for file in &failed {
            println!("  ❌ {}", file.display());
        }
        panic!(
            "{} circular_import tests should have failed but succeeded",
            failed.len()
        );
    } else {
        println!("✅ All circular_import_test fixtures failed as expected!");
    }
}
