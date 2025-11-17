//! Program File Integration Tests
//! Tests complete .doo programs from the fixtures directory

use super::super::common::compile_snippet;
use std::fs;
use std::path::PathBuf;

/// Helper to read and compile a .doo program file
fn compile_program_file(filename: &str) -> Result<String, String> {
    let path = PathBuf::from("tests/fixtures/programs/valid").join(filename);
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read file '{}': {}", filename, e))?;
    compile_snippet(&content)
}

/// Helper to read and compile an invalid .doo program file (should fail)
fn compile_invalid_program_file(filename: &str) -> Result<String, String> {
    let path = PathBuf::from("tests/fixtures/programs/invalid").join(filename);
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read file '{}': {}", filename, e))?;
    compile_snippet(&content)
}

/// Helper to assert a program file compiles successfully
fn assert_program_compiles(filename: &str) {
    match compile_program_file(filename) {
        Ok(_) => (),
        Err(e) => panic!("Program '{}' failed to compile: {}", filename, e),
    }
}

/// Helper to assert an invalid program file fails to compile
fn assert_program_fails(filename: &str) {
    match compile_invalid_program_file(filename) {
        Ok(_) => panic!("Program '{}' should have failed to compile", filename),
        Err(_) => (),
    }
}

// ========================================
// BASIC PROGRAMS
// ========================================

#[test]
fn test_program_empty() {
    assert_program_compiles("empty_program.doo");
}

#[test]
fn test_program_hello_world() {
    assert_program_compiles("hello_world.doo");
}

// ========================================
// ARITHMETIC & OPERATIONS
// ========================================

#[test]
fn test_program_arithmetic() {
    assert_program_compiles("arithmetic.doo");
}

#[test]
fn test_program_comparisons() {
    assert_program_compiles("comparisons.doo");
}

#[test]
fn test_program_boolean_logic() {
    assert_program_compiles("boolean_logic.doo");
}

#[test]
fn test_program_type_operations() {
    assert_program_compiles("type_operations.doo");
}

// ========================================
// VARIABLES & SCOPING
// ========================================

#[test]
fn test_program_mutable_vars() {
    assert_program_compiles("mutable_vars.doo");
}

#[test]
fn test_program_multiple_mutable() {
    assert_program_compiles("multiple_mutable.doo");
}

#[test]
fn test_program_scoping() {
    assert_program_compiles("scoping.doo");
}

#[test]
fn test_program_variable_scope() {
    assert_program_compiles("variable_scope.doo");
}

#[test]
fn test_program_type_inference() {
    assert_program_compiles("type_inference.doo");
}

// ========================================
// FUNCTIONS
// ========================================

#[test]
fn test_program_function_basic() {
    assert_program_compiles("function_basic.doo");
}

#[test]
fn test_program_function_multiple_params() {
    assert_program_compiles("function_multiple_params.doo");
}

#[test]
fn test_program_nested_function_calls() {
    assert_program_compiles("nested_function_calls.doo");
}

#[test]
fn test_program_function_composition() {
    assert_program_compiles("function_composition.doo");
}

#[test]
fn test_program_function_array_param() {
    assert_program_compiles("function_array_param.doo");
}

#[test]
fn test_program_function_return_array() {
    assert_program_compiles("function_return_array.doo");
}

#[test]
fn test_program_early_return() {
    assert_program_compiles("early_return.doo");
}

// ========================================
// ARRAYS
// ========================================

#[test]
fn test_program_array_expressions() {
    assert_program_compiles("array_expressions.doo");
}

#[test]
fn test_program_array_iteration() {
    assert_program_compiles("array_iteration.doo");
}

#[test]
fn test_program_array_loop_access() {
    assert_program_compiles("array_loop_access.doo");
}

#[test]
fn test_program_arrays_maps() {
    assert_program_compiles("arrays_maps.doo");
}

// ========================================
// CONTROL FLOW
// ========================================

#[test]
fn test_program_if_else() {
    assert_program_compiles("if_else.doo");
}

#[test]
fn test_program_compound_conditions() {
    assert_program_compiles("compound_conditions.doo");
}

#[test]
fn test_program_loop_patterns() {
    assert_program_compiles("loop_patterns.doo");
}

#[test]
fn test_program_nested_loop() {
    assert_program_compiles("nested_loop.doo");
}

#[test]
fn test_program_nested_loops() {
    assert_program_compiles("nested_loops.doo");
}

#[test]
fn test_program_nested_control_flow() {
    assert_program_compiles("nested_control_flow.doo");
}

#[test]
fn test_program_mixed_control() {
    assert_program_compiles("mixed_control.doo");
}

// ========================================
// STRINGS
// ========================================

#[test]
fn test_program_string_concat() {
    assert_program_compiles("string_concat.doo");
}

#[test]
fn test_program_string_ops() {
    assert_program_compiles("string_ops.doo");
}

// ========================================
// COMPLEX PROGRAMS
// ========================================

#[test]
fn test_program_calculator() {
    assert_program_compiles("calculator.doo");
}

#[test]
fn test_program_fibonacci() {
    assert_program_compiles("fibonacci.doo");
}

#[test]
fn test_program_recursion() {
    assert_program_compiles("recursion.doo");
}

#[test]
fn test_program_sorting() {
    assert_program_compiles("sorting.doo");
}

// ========================================
// STRESS & LARGE PROGRAMS
// ========================================

#[test]
fn test_program_large_loop() {
    assert_program_compiles("large_loop.doo");
}

#[test]
fn test_program_large_program() {
    assert_program_compiles("large_program.doo");
}

// ========================================
// ERROR CASES - PROGRAMS THAT SHOULD FAIL
// ========================================

#[test]
fn test_error_type_mismatch() {
    assert_program_fails("type_error.doo");
}

#[test]
fn test_error_wrong_arg_count() {
    assert_program_fails("wrong_arg_count.doo");
}

#[test]
fn test_error_undefined_variable() {
    assert_program_fails("undefined_variable.doo");
}

#[test]
fn test_error_undefined_function() {
    assert_program_fails("undefined_function.doo");
}

#[test]
fn test_error_return_type_mismatch() {
    assert_program_fails("return_type_mismatch.doo");
}

#[test]
fn test_error_break_outside_loop() {
    assert_program_fails("break_outside_loop.doo");
}

#[test]
fn test_error_continue_outside_loop() {
    assert_program_fails("continue_outside_loop.doo");
}

#[test]
fn test_error_duplicate_variable() {
    assert_program_fails("duplicate_variable.doo");
}

#[test]
fn test_error_immutable_reassignment() {
    assert_program_fails("immutable_reassignment.doo");
}

#[test]
fn test_error_missing_return() {
    assert_program_fails("missing_return.doo");
}

#[test]
fn test_error_array_type_mismatch() {
    assert_program_fails("array_type_mismatch.doo");
}

#[test]
fn test_error_map_key_type_mismatch() {
    assert_program_fails("map_key_type_mismatch.doo");
}

#[test]
fn test_error_map_value_type_mismatch() {
    assert_program_fails("map_value_type_mismatch.doo");
}

#[test]
fn test_error_invalid_arithmetic() {
    assert_program_fails("invalid_arithmetic.doo");
}

#[test]
fn test_error_duplicate_function() {
    assert_program_fails("duplicate_function.doo");
}

#[test]
fn test_error_if_condition_type() {
    assert_program_fails("if_condition_type_error.doo");
}

#[test]
fn test_error_invalid_loop_range() {
    assert_program_fails("invalid_loop_range.doo");
}

#[test]
fn test_error_duplicate_parameter() {
    assert_program_fails("duplicate_parameter.doo");
}
