use doo::compiler::{compile_project, CompileOptions};
use std::path::PathBuf;

fn test_program_file(filename: &str) -> bool {
    let path = PathBuf::from(format!("tests/programs/valid/{}", filename));
    let opts = CompileOptions {
        input_path: path,
        output_name: "test_output".to_string(),
        check_only: true,
        ..Default::default()
    };

    match compile_project(opts) {
        Ok(result) => result.success,
        Err(_) => false,
    }
}

// =====================
// Basic & Classic Programs
// =====================

#[test]
fn test_hello_world() {
    assert!(test_program_file("hello_world.doo"));
}

#[test]
fn test_fibonacci() {
    assert!(test_program_file("fibonacci.doo"));
}

#[test]
fn test_sorting() {
    assert!(test_program_file("sorting.doo"));
}

#[test]
fn test_calculator() {
    assert!(test_program_file("calculator.doo"));
}

#[test]
fn test_nested_loops() {
    assert!(test_program_file("nested_loops.doo"));
}

#[test]
fn test_recursion() {
    assert!(test_program_file("recursion.doo"));
}

#[test]
fn test_arrays_and_maps() {
    assert!(test_program_file("arrays_maps.doo"));
}

#[test]
fn test_string_operations() {
    assert!(test_program_file("string_ops.doo"));
}

#[test]
fn test_type_inference() {
    assert!(test_program_file("type_inference.doo"));
}

#[test]
fn test_scoping() {
    assert!(test_program_file("scoping.doo"));
}

// =====================
// Functions
// =====================

#[test]
fn test_function_basic() {
    assert!(test_program_file("function_basic.doo"));
}

#[test]
fn test_function_multiple_params() {
    assert!(test_program_file("function_multiple_params.doo"));
}

#[test]
fn test_nested_function_calls() {
    assert!(test_program_file("nested_function_calls.doo"));
}

#[test]
fn test_function_return_array() {
    assert!(test_program_file("function_return_array.doo"));
}

#[test]
fn test_function_array_param() {
    assert!(test_program_file("function_array_param.doo"));
}

#[test]
fn test_function_composition() {
    assert!(test_program_file("function_composition.doo"));
}

#[test]
fn test_early_return() {
    assert!(test_program_file("early_return.doo"));
}

// =====================
// Arrays
// =====================

#[test]
fn test_array_iteration() {
    assert!(test_program_file("array_iteration.doo"));
}

#[test]
fn test_array_expressions() {
    assert!(test_program_file("array_expressions.doo"));
}

#[test]
fn test_array_loop_access() {
    assert!(test_program_file("array_loop_access.doo"));
}

// =====================
// Control Flow (if, for, loops)
// =====================

#[test]
fn test_if_else() {
    assert!(test_program_file("if_else.doo"));
}

#[test]
fn test_nested_loop() {
    assert!(test_program_file("nested_loop.doo"));
}

#[test]
fn test_loop_patterns() {
    assert!(test_program_file("loop_patterns.doo"));
}

#[test]
fn test_nested_control_flow() {
    assert!(test_program_file("nested_control_flow.doo"));
}

#[test]
fn test_large_loop() {
    assert!(test_program_file("large_loop.doo"));
}

#[test]
fn test_mixed_control() {
    assert!(test_program_file("mixed_control.doo"));
}

// =====================
// Variables & Scoping
// =====================
#[test]
fn test_mutable_vars() {
    assert!(test_program_file("mutable_vars.doo"));
}

#[test]
fn test_variable_scope() {
    assert!(test_program_file("variable_scope.doo"));
}

#[test]
fn test_multiple_mutable() {
    assert!(test_program_file("multiple_mutable.doo"));
}

// =====================
// Operators & Expressions
// =====================

#[test]
fn test_arithmetic() {
    assert!(test_program_file("arithmetic.doo"));
}

#[test]
fn test_boolean_logic() {
    assert!(test_program_file("boolean_logic.doo"));
}

#[test]
fn test_comparisons() {
    assert!(test_program_file("comparisons.doo"));
}

#[test]
fn test_compound_conditions() {
    assert!(test_program_file("compound_conditions.doo"));
}

// =====================
// String Operations
// =====================

#[test]
fn test_string_concat() {
    assert!(test_program_file("string_concat.doo"));
}

#[test]
fn test_type_operations() {
    assert!(test_program_file("type_operations.doo"));
}
