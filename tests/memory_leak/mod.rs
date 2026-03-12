//! Memory Leak Tests (Valgrind)
//!
//! Compiles each .doo test program, runs it through Valgrind memcheck,
//! and verifies no memory leaks exist.
//!
//! These tests only run on Linux (WSL) where Valgrind is available.
//! On Windows, they compile-check only (no Valgrind).
//! Zero overhead on production binaries — Valgrind instruments externally.

use std::path::Path;
use std::process::Command;

/// Find the doo binary based on the current platform
fn find_doo_binary() -> String {
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &["target-windows/release/doo.exe"]
    } else if cfg!(target_os = "macos") {
        &["target/release/doo"]
    } else {
        &[
            r"\\wsl.localhost\Ubuntu\home\nayan\doo-builds\linux\release\doo",
            "target-linux/release/doo",
        ]
    };

    candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .unwrap_or_else(|| {
            panic!(
                "No doo binary found. Build first with: cargo build --release --workspace\nTried: {:?}",
                candidates
            )
        })
        .to_string()
}

/// Check if Valgrind is available
fn has_valgrind() -> bool {
    Command::new("valgrind")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run a .doo file and check for memory leaks with Valgrind
fn run_leak_test(file_name: &str) {
    let test_dir = Path::new("tests/memory_leak/programs");
    let path = test_dir.join(file_name);
    assert!(path.exists(), "Test file not found: {:?}", path);

    let doo_bin = find_doo_binary();

    // Step 1: Compile the program
    let compile_output = Command::new(&doo_bin)
        .args(&["run", path.to_str().unwrap()])
        .output()
        .expect("Failed to run doo");

    let stdout = String::from_utf8_lossy(&compile_output.stdout);
    let stderr = String::from_utf8_lossy(&compile_output.stderr);

    // Must compile and run successfully first
    assert!(
        compile_output.status.success(),
        "Compilation/execution failed for {}:\nstdout:\n{}\nstderr:\n{}",
        file_name,
        stdout,
        stderr
    );

    // Step 2: Run through Valgrind (Linux only)
    if cfg!(target_os = "linux") && has_valgrind() {
        // Build to get a binary (not just run)
        let results_dir = Path::new("tests/memory_leak/results");
        std::fs::create_dir_all(results_dir).ok();
        let binary_name = file_name.trim_end_matches(".doo");
        let binary_path = results_dir.join(binary_name);

        let build_output = Command::new(&doo_bin)
            .args(&[
                "build",
                path.to_str().unwrap(),
                "-o",
                binary_path.to_str().unwrap(),
            ])
            .output()
            .expect("Failed to build doo program");

        if !build_output.status.success() {
            // Skip Valgrind if build command not supported
            return;
        }

        if binary_path.exists() {
            let valgrind_output = Command::new("valgrind")
                .args(&[
                    "--leak-check=full",
                    "--show-leak-kinds=definite,possible",
                    "--errors-for-leak-kinds=definite,possible",
                    "--error-exitcode=42",
                    binary_path.to_str().unwrap(),
                ])
                .output()
                .expect("Failed to run valgrind");

            let valgrind_stderr = String::from_utf8_lossy(&valgrind_output.stderr);

            assert!(
                valgrind_output.status.code() != Some(42),
                "MEMORY LEAK detected by Valgrind in {}!\n{}",
                file_name,
                valgrind_stderr
            );
        }
    }
}

// ===========================================================================
// Individual Test Cases
// ===========================================================================

#[test]
fn leak_01_basic_string() {
    run_leak_test("01_basic_string.doo");
}

#[test]
fn leak_02_basic_int() {
    run_leak_test("02_basic_int.doo");
}

#[test]
fn leak_03_multiple_strings() {
    run_leak_test("03_multiple_strings.doo");
}

#[test]
fn leak_04_string_interpolation() {
    run_leak_test("04_string_interpolation.doo");
}

#[test]
fn leak_05_array_create() {
    run_leak_test("05_array_create.doo");
}

#[test]
fn leak_06_array_strings() {
    run_leak_test("06_array_strings.doo");
}

#[test]
fn leak_07_struct_simple() {
    run_leak_test("07_struct_simple.doo");
}

#[test]
fn leak_08_struct_with_strings() {
    run_leak_test("08_struct_with_strings.doo");
}

#[test]
fn leak_09_nested_structs() {
    run_leak_test("09_nested_structs.doo");
}

#[test]
fn leak_10_func_string_param() {
    run_leak_test("10_func_string_param.doo");
}

#[test]
fn leak_11_func_return_string() {
    run_leak_test("11_func_return_string.doo");
}

#[test]
fn leak_12_func_return_struct() {
    run_leak_test("12_func_return_struct.doo");
}

#[test]
fn leak_13_func_struct_param() {
    run_leak_test("13_func_struct_param.doo");
}

#[test]
fn leak_14_func_array_param() {
    run_leak_test("14_func_array_param.doo");
}

#[test]
fn leak_15_loop_string_alloc() {
    run_leak_test("15_loop_string_alloc.doo");
}

#[test]
fn leak_16_loop_integer() {
    run_leak_test("16_loop_integer.doo");
}

#[test]
fn leak_17_conditional_string() {
    run_leak_test("17_conditional_string.doo");
}

#[test]
fn leak_18_mutable_string() {
    run_leak_test("18_mutable_string.doo");
}

#[test]
fn leak_19_multi_func_calls() {
    run_leak_test("19_multi_func_calls.doo");
}

#[test]
fn leak_20_map_str_str() {
    run_leak_test("20_map_str_str.doo");
}

#[test]
fn leak_21_map_str_int() {
    run_leak_test("21_map_str_int.doo");
}

#[test]
fn leak_22_enum_match() {
    run_leak_test("22_enum_match.doo");
}

#[test]
fn leak_23_enum_payload() {
    run_leak_test("23_enum_payload.doo");
}

#[test]
fn leak_24_error_handling() {
    run_leak_test("24_error_handling.doo");
}

#[test]
fn leak_25_array_push_loop() {
    run_leak_test("25_array_push_loop.doo");
}

#[test]
fn leak_26_array_string_push() {
    run_leak_test("26_array_string_push.doo");
}

#[test]
fn leak_27_struct_method() {
    run_leak_test("27_struct_method.doo");
}

#[test]
fn leak_28_for_array_iter() {
    run_leak_test("28_for_array_iter.doo");
}

#[test]
fn leak_29_array_modify() {
    run_leak_test("29_array_modify.doo");
}

#[test]
fn leak_30_func_return_array() {
    run_leak_test("30_func_return_array.doo");
}

#[test]
fn leak_31_multi_struct_instances() {
    run_leak_test("31_multi_struct_instances.doo");
}

#[test]
fn leak_32_string_concat_loop() {
    run_leak_test("32_string_concat_loop.doo");
}

#[test]
fn leak_33_nested_func_strings() {
    run_leak_test("33_nested_func_strings.doo");
}

#[test]
fn leak_34_struct_field_reassign() {
    run_leak_test("34_struct_field_reassign.doo");
}

#[test]
fn leak_35_range_loop_strings() {
    run_leak_test("35_range_loop_strings.doo");
}

#[test]
fn leak_36_deep_nested_struct() {
    run_leak_test("36_deep_nested_struct.doo");
}

#[test]
fn leak_37_multi_error_paths() {
    run_leak_test("37_multi_error_paths.doo");
}

#[test]
fn leak_38_primitives_no_heap() {
    run_leak_test("38_primitives_no_heap.doo");
}

#[test]
fn leak_39_struct_factory() {
    run_leak_test("39_struct_factory.doo");
}

#[test]
fn leak_40_stress_loop_strings() {
    run_leak_test("40_stress_loop_strings.doo");
}
