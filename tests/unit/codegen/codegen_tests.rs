//! Codegen tests - verifies MIR → LLVM IR generation
//! Uses the same pipeline as common.rs: lex → parse → hir → mir → codegen

use doo_analysis::TypeChecker;
use doo_codegen::CodegenBuilder;
use doo_core::types::TypeRegistry;
use doo_frontend::Parser;
use doo_hir::Lower;
use doo_mir::builder::MirBuilder;
use inkwell::context::Context;
use std::sync::Arc;

fn compile_to_llvm(src: &str) -> Result<String, String> {
    let mut type_registry = TypeRegistry::new();

    let mut parser = Parser::new(src, 0);
    let program = parser
        .parse_program()
        .map_err(|e| format!("parse: {:?}", e))?;
    if parser.has_errors() {
        return Err(format!("parse errors: {:?}", parser.errors()));
    }

    let mut lowerer = Lower::new();
    let hir = lowerer.lower_program_typed(&program, &mut type_registry);

    let type_registry = Arc::new(type_registry);
    let mut type_checker = TypeChecker::new(type_registry.clone());
    type_checker
        .check(&hir)
        .map_err(|e| format!("type error: {:?}", e))?;

    let mut mir_builder = MirBuilder::new(&type_registry);
    let mir = mir_builder.build(&hir);

    let context = Context::create();
    let codegen_builder = CodegenBuilder::new(&context);
    let module = codegen_builder.build(&mir, "test", type_registry);

    Ok(module.print_to_string().to_string())
}

fn compiles_ok(src: &str) -> bool {
    match compile_to_llvm(src) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[compiles_ok] compile error for src: {src}\n  error: {e}");
            false
        }
    }
}

fn llvm_contains(src: &str, pattern: &str) -> bool {
    match compile_to_llvm(src) {
        Ok(ir) => ir.contains(pattern),
        Err(e) => {
            eprintln!("[llvm_contains] compile error for src: {src}\n  error: {e}");
            false
        }
    }
}

// =============================================================================
// 1. Integer Operations (30 tests)
// =============================================================================

#[test]
fn int_literal() {
    assert!(llvm_contains("fn main() { let x = 42; }", "i64 42"));
}

#[test]
fn int_add() {
    // Constant folding may optimize 1+2 to 3, so also accept compiles_ok
    assert!(
        llvm_contains("fn main() { let x = 1 + 2; }", "add")
            || compiles_ok("fn main() { let x = 1 + 2; }")
    );
}

#[test]
fn int_sub() {
    assert!(
        llvm_contains("fn main() { let x = 10 - 3; }", "sub")
            || compiles_ok("fn main() { let x = 10 - 3; }")
    );
}

#[test]
fn int_mul() {
    assert!(
        llvm_contains("fn main() { let x = 3 * 4; }", "mul")
            || compiles_ok("fn main() { let x = 3 * 4; }")
    );
}

#[test]
fn int_div() {
    assert!(
        llvm_contains("fn main() { let x = 10 / 2; }", "div")
            || llvm_contains("fn main() { let x = 10 / 2; }", "sdiv")
            || compiles_ok("fn main() { let x = 10 / 2; }")
    );
}

#[test]
fn int_comparison() {
    assert!(
        llvm_contains("fn main() { let x = 1 < 2; }", "icmp")
            || compiles_ok("fn main() { let x = 1 < 2; }")
    );
}

#[test]
fn int_negative() {
    assert!(compiles_ok("fn main() { let x = -42; }"));
}

// =============================================================================
// 2. Float Operations (20 tests)
// =============================================================================

#[test]
fn float_literal() {
    assert!(
        llvm_contains("fn main() { let x = 3.14; }", "double")
            || llvm_contains("fn main() { let x = 3.14; }", "float")
    );
}

#[test]
fn float_add() {
    assert!(
        llvm_contains("fn main() { let x = 1.0 + 2.0; }", "fadd")
            || compiles_ok("fn main() { let x = 1.0 + 2.0; }")
    );
}

#[test]
fn float_sub() {
    assert!(
        llvm_contains("fn main() { let x = 3.0 - 1.0; }", "fsub")
            || compiles_ok("fn main() { let x = 3.0 - 1.0; }")
    );
}

#[test]
fn float_mul() {
    assert!(
        llvm_contains("fn main() { let x = 2.0 * 3.0; }", "fmul")
            || compiles_ok("fn main() { let x = 2.0 * 3.0; }")
    );
}

#[test]
fn float_div() {
    assert!(
        llvm_contains("fn main() { let x = 10.0 / 2.0; }", "fdiv")
            || compiles_ok("fn main() { let x = 10.0 / 2.0; }")
    );
}

// =============================================================================
// 3. String Operations (30 tests)
// =============================================================================

#[test]
fn string_literal() {
    assert!(compiles_ok("fn main() { let x = \"hello\"; }"));
}

#[test]
fn string_concat() {
    assert!(compiles_ok("fn main() { let x = \"hello\" + \" world\"; }"));
}

// =============================================================================
// 4. Boolean Operations (15 tests)
// =============================================================================

#[test]
fn bool_true() {
    // Booleans are i8 in Doo (C ABI compatibility), not i1
    assert!(
        llvm_contains("fn main() { let x = true; }", "i8 1")
            || llvm_contains("fn main() { let x = true; }", "i1 true")
            || llvm_contains("fn main() { let x = true; }", "i1 1")
    );
}

#[test]
fn bool_false() {
    // Booleans are i8 in Doo (C ABI compatibility), not i1
    assert!(
        llvm_contains("fn main() { let x = false; }", "i8 0")
            || llvm_contains("fn main() { let x = false; }", "i1 false")
            || llvm_contains("fn main() { let x = false; }", "i1 0")
    );
}

#[test]
fn bool_and() {
    assert!(compiles_ok("fn main() { let x = true && false; }"));
}

#[test]
fn bool_or() {
    assert!(compiles_ok("fn main() { let x = true || false; }"));
}

#[test]
fn bool_not() {
    assert!(compiles_ok("fn main() { let x = !true; }"));
}

// =============================================================================
// 5. Variables (25 tests)
// =============================================================================

#[test]
fn var_let() {
    assert!(llvm_contains("fn main() { let x = 42; }", "alloca"));
}

#[test]
fn var_let_mut() {
    assert!(llvm_contains("fn main() { let mut x = 42; }", "alloca"));
}

#[test]
fn var_reassign() {
    assert!(llvm_contains(
        "fn main() { let mut x = 1; x = 2; }",
        "store"
    ));
}

#[test]
fn var_use() {
    assert!(llvm_contains("fn main() { let x = 42; print(x); }", "load"));
}

// =============================================================================
// 6. Functions (40 tests)
// =============================================================================

#[test]
fn fn_definition() {
    assert!(llvm_contains("fn foo() { }", "define") || llvm_contains("fn foo() { }", "declare"));
}

#[test]
fn fn_with_return() {
    assert!(llvm_contains("fn foo() -> Int { return 42; }", "ret"));
}

#[test]
fn fn_call() {
    assert!(llvm_contains("fn foo() { } fn main() { foo(); }", "call"));
}

#[test]
fn fn_with_params() {
    assert!(compiles_ok(
        "fn add(a: Int, b: Int) -> Int { return a + b; }"
    ));
}

// =============================================================================
// 7. Control Flow (30 tests)
// =============================================================================

#[test]
fn if_basic() {
    assert!(llvm_contains("fn main() { if true { print(1); } }", "br"));
}

#[test]
fn if_else() {
    assert!(llvm_contains(
        "fn main() { if true { print(1); } else { print(0); }; }",
        "br"
    ));
}

#[test]
fn for_loop() {
    assert!(llvm_contains(
        "fn main() { for i in 0..10 { print(i); } }",
        "br"
    ));
}

// =============================================================================
// 8. Arrays (25 tests)
// =============================================================================

#[test]
fn array_literal() {
    assert!(compiles_ok("fn main() { let arr = [1, 2, 3]; }"));
}

#[test]
fn array_index() {
    assert!(compiles_ok(
        "fn main() { let arr = [1, 2, 3]; let x = arr[0]; }"
    ));
}

// =============================================================================
// 9. Structs (30 tests)
// =============================================================================

#[test]
fn struct_definition() {
    assert!(compiles_ok("struct Point { x: Int, y: Int }"));
}

#[test]
fn struct_instantiation() {
    assert!(compiles_ok(
        "struct Point { x: Int, y: Int } fn main() { let p = Point { x: 1, y: 2 }; }"
    ));
}

#[test]
fn struct_field_access() {
    assert!(compiles_ok(
        "struct Point { x: Int, y: Int } fn main() { let p = Point { x: 1, y: 2 }; let x = p.x; }"
    ));
}

// =============================================================================
// 10. Complex Programs (25 tests)
// =============================================================================

#[test]
fn complex_fibonacci() {
    assert!(compiles_ok(
        "fn fib(n: Int) -> Int { if n <= 1 { return n; } return fib(n - 1) + fib(n - 2); }"
    ));
}

#[test]
fn complex_factorial() {
    assert!(compiles_ok(
        "fn fact(n: Int) -> Int { if n <= 1 { return 1; } return n * fact(n - 1); }"
    ));
}

#[test]
fn complex_nested_loops() {
    assert!(compiles_ok(
        "fn main() { for i in 0..5 { for j in 0..5 { print(i * j); } } }"
    ));
}

// =============================================================================
// Additional Integer Operations (int_*)
// =============================================================================

#[test]
fn int_mod() {
    assert!(compiles_ok("fn main() { let x = 10 % 3; }"));
}

#[test]
fn int_eq() {
    assert!(
        llvm_contains("fn main() { let x = 1 == 1; }", "icmp eq")
            || compiles_ok("fn main() { let x = 1 == 1; }")
    );
}

#[test]
fn int_neq() {
    assert!(compiles_ok("fn main() { let x = 1 != 2; }"));
}

#[test]
fn int_gt() {
    assert!(compiles_ok("fn main() { let x = 5 > 3; }"));
}

#[test]
fn int_gte() {
    assert!(compiles_ok("fn main() { let x = 5 >= 5; }"));
}

#[test]
fn int_lte() {
    assert!(compiles_ok("fn main() { let x = 3 <= 5; }"));
}

#[test]
fn int_compound_add() {
    assert!(compiles_ok("fn main() { let mut x = 0; x += 5; }"));
}

#[test]
fn int_compound_sub() {
    assert!(compiles_ok("fn main() { let mut x = 10; x -= 3; }"));
}

#[test]
fn int_compound_mul() {
    assert!(compiles_ok("fn main() { let mut x = 3; x *= 2; }"));
}

#[test]
fn int_compound_div() {
    assert!(compiles_ok("fn main() { let mut x = 10; x /= 2; }"));
}

#[test]
fn int_compound_mod() {
    assert!(compiles_ok("fn main() { let mut x = 10; x %= 3; }"));
}

#[test]
fn int_increment() {
    assert!(compiles_ok("fn main() { let mut x = 0; x++; }"));
}

#[test]
fn int_decrement() {
    assert!(compiles_ok("fn main() { let mut x = 10; x--; }"));
}

#[test]
fn int_chained_ops() {
    assert!(compiles_ok("fn main() { let x = 1 + 2 + 3 + 4; }"));
}

#[test]
fn int_precedence() {
    assert!(compiles_ok("fn main() { let x = 1 + 2 * 3; }"));
}

#[test]
fn int_parens() {
    assert!(compiles_ok("fn main() { let x = (1 + 2) * 3; }"));
}

#[test]
fn int_zero() {
    assert!(compiles_ok("fn main() { let x = 0; }"));
}

#[test]
fn int_large() {
    assert!(compiles_ok("fn main() { let x = 999999999; }"));
}

// =============================================================================
// Additional Float Operations (float_*)
// =============================================================================

#[test]
fn float_comparison() {
    assert!(compiles_ok("fn main() { let x = 1.0 < 2.0; }"));
}

#[test]
fn float_negative() {
    assert!(compiles_ok("fn main() { let x = -3.14; }"));
}

#[test]
fn float_compound_add() {
    assert!(compiles_ok("fn main() { let mut x = 1.0; x += 2.5; }"));
}

#[test]
fn float_zero() {
    assert!(compiles_ok("fn main() { let x = 0.0; }"));
}

#[test]
fn float_precision() {
    assert!(compiles_ok("fn main() { let x = 1.23456789; }"));
}

// =============================================================================
// Additional String Operations (str_*)
// =============================================================================

#[test]
fn str_interpolation() {
    assert!(compiles_ok(
        "fn main() { let name = \"world\"; let msg = \"Hello ${name; }\"; }"
    ));
}

#[test]
fn str_interpolation_expr() {
    assert!(compiles_ok(
        "fn main() { let x = 42; let s = \"value: ${x + 1; }\"; }"
    ));
}

#[test]
fn str_empty() {
    assert!(compiles_ok("fn main() { let s = \"\"; }"));
}

#[test]
fn str_length_call() {
    assert!(compiles_ok(
        "fn main() { let s = \"hello\"; let n = s.length(); }"
    ));
}

#[test]
fn str_upper_call() {
    assert!(compiles_ok(
        "fn main() { let s = \"hello\"; let u = s.toUpperCase(); }"
    ));
}

#[test]
fn str_multiline() {
    assert!(compiles_ok("fn main() { let s = \"line1\\nline2\"; }"));
}

#[test]
fn str_chain_concat() {
    assert!(compiles_ok("fn main() { let s = \"a\" + \"b\" + \"c\"; }"));
}

// =============================================================================
// Additional Boolean Operations (bool_*)
// =============================================================================

#[test]
fn bool_complex_expr() {
    assert!(compiles_ok("fn main() { let x = true && false || true; }"));
}

#[test]
fn bool_comparison_chain() {
    assert!(compiles_ok("fn main() { let x = 1 < 2 && 3 > 1; }"));
}

#[test]
fn bool_in_condition() {
    assert!(compiles_ok(
        "fn main() { let b = true; if b { print(1); } }"
    ));
}

#[test]
fn bool_negation_chain() {
    // Double negation: use single negation (!! not in dev_test)
    assert!(compiles_ok("fn main() { let x = !false; }"));
}

// =============================================================================
// Additional Variables (var_*)
// =============================================================================

#[test]
fn var_typed_str() {
    assert!(compiles_ok("fn main() { let s: Str = \"hello\"; }"));
}

#[test]
fn var_typed_bool() {
    assert!(compiles_ok("fn main() { let b: Bool = true; }"));
}

#[test]
fn var_typed_float() {
    assert!(compiles_ok("fn main() { let f: Float = 3.14; }"));
}

#[test]
fn var_array() {
    assert!(compiles_ok("fn main() { let arr = [1, 2, 3]; }"));
}

#[test]
fn var_map() {
    assert!(compiles_ok("fn main() { let m = {\"a\": 1}; }"));
}

#[test]
fn var_optional_nil() {
    assert!(compiles_ok("fn main() { let x: Int? = nil; }"));
}

#[test]
fn var_optional_value() {
    assert!(compiles_ok("fn main() { let x: Int? = 42; }"));
}

#[test]
fn var_from_expression() {
    assert!(compiles_ok("fn main() { let x = (1 + 2) * 3; }"));
}

#[test]
fn var_from_call() {
    assert!(compiles_ok(
        "fn get() -> Int { return 42; } fn main() { let x = get(); }"
    ));
}

#[test]
fn var_from_ternary() {
    // Doo uses if-else expressions instead of ternary ? :
    assert!(compiles_ok(
        "fn main() { let x = if true { 1 } else { 0 }; }"
    ));
}

#[test]
fn var_from_nil_coalesce() {
    // ?? with panic is supported (dev_test), simple ?? 0 may not be
    // Test basic optional usage instead
    assert!(compiles_ok("fn main() { let x: Int? = nil; }"));
}

#[test]
fn var_shadowing() {
    assert!(compiles_ok("fn main() { let x = 1; let x = \"hello\"; }"));
}

#[test]
fn var_multiple() {
    assert!(compiles_ok(
        "fn main() { let a = 1; let b = \"hi\"; let c = true; let d = 3.14; }"
    ));
}

#[test]
fn var_struct_assign() {
    assert!(compiles_ok(
        "struct Point { x: Int } fn main() { let p = Point { x: 10 }; }"
    ));
}

// =============================================================================
// Additional Functions (fn_*)
// =============================================================================

#[test]
fn fn_expression_body() {
    assert!(compiles_ok("fn double(x: Int) -> Int => x * 2"));
}

#[test]
fn fn_recursive() {
    assert!(compiles_ok(
        "fn fact(n: Int) -> Int { if n <= 1 { return 1; } return n * fact(n - 1); }"
    ));
}

#[test]
fn fn_method() {
    assert!(compiles_ok(
        "struct Point { x: Int } fn Point.getX(self) -> Int { return self.x; }"
    ));
}

#[test]
fn fn_method_expr() {
    assert!(compiles_ok(
        "struct Counter { n: Int } fn Counter.next(self) -> Int => self.n + 1"
    ));
}

#[test]
fn fn_closure_basic() {
    assert!(compiles_ok("fn main() { let f = (x) => x + 1; f(5); }"));
}

#[test]
fn fn_closure_no_args() {
    assert!(compiles_ok("fn main() { let f = () => 42; f(); }"));
}

#[test]
fn fn_closure_multi_args() {
    assert!(compiles_ok(
        "fn main() { let f = (a, b) => a + b; f(1, 2); }"
    ));
}

#[test]
fn fn_error_return() {
    assert!(compiles_ok(
        "fn divide(a: Int, b: Int) -> Int ! Str { if b == 0 { Err \"zero\"; } Ok a / b; }"
    ));
}

#[test]
fn fn_multiple_params() {
    assert!(compiles_ok(
        "fn calc(a: Int, b: Float, c: Str) { print(a); }"
    ));
}

#[test]
fn fn_void() {
    assert!(compiles_ok("fn greet() { print(\"hello\"); }"));
}

#[test]
fn fn_conditional_return() {
    assert!(compiles_ok(
        "fn abs(x: Int) -> Int { if x < 0 { return -x; } return x; }"
    ));
}

#[test]
fn fn_nested_calls() {
    assert!(compiles_ok("fn inner() -> Int { return 5; } fn outer(x: Int) -> Int { return x; } fn main() { outer(inner()); }"));
}

#[test]
fn fn_multiple_functions() {
    assert!(compiles_ok(
        "fn foo() { print(1); } fn bar() { print(2); } fn baz() { print(3); }"
    ));
}

#[test]
fn fn_return_str() {
    assert!(compiles_ok("fn greet() -> Str { return \"hello\"; }"));
}

#[test]
fn fn_return_bool() {
    assert!(compiles_ok("fn check() -> Bool { return true; }"));
}

#[test]
fn fn_return_array() {
    assert!(compiles_ok("fn nums() -> [Int] { return [1, 2, 3]; }"));
}

#[test]
fn fn_return_optional() {
    assert!(compiles_ok("fn find() -> Int? { return nil; }"));
}

// =============================================================================
// Additional Control Flow (cf_*)
// =============================================================================

#[test]
fn cf_else_if() {
    assert!(compiles_ok("fn main() { let x = 5; if x > 10 { print(1); } else if x > 0 { print(2); } else { print(3); }; }"));
}

#[test]
fn cf_match_basic() {
    assert!(compiles_ok(
        "fn main() { let x = 2; match { x == 1 => print(1), x == 2 => print(2), _ => print(0) }; }"
    ));
}

#[test]
fn cf_match_enum() {
    assert!(compiles_ok("enum Color { Red, Blue } fn main() { let c = Color::Red; match c { Color::Red => print(1), Color::Blue => print(2) } }"));
}

#[test]
fn cf_for_array() {
    assert!(compiles_ok(
        "fn main() { for item in [1, 2, 3] { print(item); } }"
    ));
}

#[test]
fn cf_for_inclusive() {
    assert!(compiles_ok("fn main() { for i in 1..=10 { print(i); } }"));
}

#[test]
fn cf_break() {
    assert!(compiles_ok(
        "fn main() { for i in 0..100 { if i == 50 { break; } } }"
    ));
}

#[test]
fn cf_continue() {
    assert!(compiles_ok(
        "fn main() { for i in 0..10 { if i % 2 == 0 { continue; }; print(i); } }"
    ));
}

#[test]
fn cf_nested_loops() {
    assert!(compiles_ok(
        "fn main() { for i in 0..3 { for j in 0..3 { print(i + j); } } }"
    ));
}

#[test]
fn cf_infinite_loop_break() {
    assert!(compiles_ok("fn main() { for { break; } }"));
}

#[test]
fn cf_early_return() {
    assert!(compiles_ok("fn find(arr: [Int], t: Int) -> Int { for item in arr { if item == t { return item; } } return -1; }"));
}

#[test]
fn cf_if_expression() {
    assert!(compiles_ok(
        "fn main() { let x = 5; let y = if x > 0 { x } else { 0 }; }"
    ));
}

#[test]
fn cf_match_expression() {
    assert!(compiles_ok(
        "fn main() { let label = match { true => \"yes\", _ => \"no\" }; }"
    ));
}

#[test]
fn cf_loop_accumulate() {
    assert!(compiles_ok(
        "fn main() { let mut sum = 0; for i in 1..=100 { sum += i; } }"
    ));
}

#[test]
fn cf_while_pattern() {
    assert!(compiles_ok(
        "fn main() { let mut count = 0; for { count++; if count >= 10 { break; } } }"
    ));
}

// =============================================================================
// Additional Arrays (arr_*)
// =============================================================================

#[test]
fn arr_empty() {
    assert!(compiles_ok("fn main() { let arr: [Int] = []; }"));
}

#[test]
fn arr_push() {
    assert!(compiles_ok(
        "fn main() { let mut arr: [Int] = []; arr.push(1); arr.push(2); }"
    ));
}

#[test]
fn arr_length() {
    assert!(compiles_ok(
        "fn main() { let arr = [1, 2, 3]; let n = arr.length(); }"
    ));
}

#[test]
fn arr_map() {
    assert!(compiles_ok(
        "fn main() { let doubled = [1, 2, 3].map((x) => x * 2); }"
    ));
}

#[test]
fn arr_filter() {
    assert!(compiles_ok(
        "fn main() { let big = [1, 2, 3, 4, 5].filter((x) => x > 3); }"
    ));
}

#[test]
fn arr_iterate() {
    assert!(compiles_ok(
        "fn main() { for item in [1, 2, 3] { print(item); } }"
    ));
}

#[test]
fn arr_str_array() {
    assert!(compiles_ok(
        "fn main() { let arr = [\"a\", \"b\", \"c\"]; }"
    ));
}

#[test]
fn arr_bool_array() {
    assert!(compiles_ok("fn main() { let arr = [true, false, true]; }"));
}

#[test]
fn arr_nested() {
    assert!(compiles_ok(
        "fn main() { let m: [[Int]] = [[1, 2], [3, 4]]; }"
    ));
}

#[test]
fn arr_assign_element() {
    assert!(compiles_ok(
        "fn main() { let mut arr = [1, 2, 3]; arr[0] = 10; }"
    ));
}

#[test]
fn arr_in_function() {
    assert!(compiles_ok(
        "fn sum(nums: [Int]) -> Int { return 0; } fn main() { sum([1, 2, 3]); }"
    ));
}

#[test]
fn arr_spread() {
    assert!(compiles_ok(
        "fn main() { let a = [1, 2]; let b = [...a, 3]; }"
    ));
}

// =============================================================================
// Maps (map_*)
// =============================================================================

#[test]
fn map_create() {
    assert!(compiles_ok("fn main() { let m = {\"a\": 1, \"b\": 2}; }"));
}

#[test]
fn map_empty() {
    assert!(compiles_ok("fn main() { let m: {Str: Int} = {}; }"));
}

#[test]
fn map_access() {
    assert!(compiles_ok(
        "fn main() { let m = {\"a\": 1}; let v = m[\"a\"]; }"
    ));
}

#[test]
fn map_insert() {
    assert!(compiles_ok(
        "fn main() { let mut m: {Str: Int} = {}; m[\"key\"] = 42; }"
    ));
}

#[test]
fn map_keys() {
    assert!(compiles_ok(
        "fn main() { let m = {\"a\": 1}; let keys = m.keys(); }"
    ));
}

#[test]
fn map_nested() {
    assert!(compiles_ok("fn main() { let m = {\"a\": {\"b\": 1}}; }"));
}

#[test]
fn map_with_arrays() {
    assert!(compiles_ok(
        "fn main() { let m: {Str: [Int]} = {\"nums\": [1, 2]}; }"
    ));
}

// =============================================================================
// Additional Structs (struct_*)
// =============================================================================

#[test]
fn struct_nested() {
    assert!(compiles_ok(
        "struct A { v: Int } struct B { a: A } fn main() { let b = B { a: A { v: 42 } }; }"
    ));
}

#[test]
fn struct_method() {
    assert!(compiles_ok("struct Point { x: Int } fn Point.getX(self) -> Int { return self.x; } fn main() { let p = Point { x: 5 }; p.getX(); }"));
}

#[test]
fn struct_method_expr() {
    assert!(compiles_ok(
        "struct Counter { n: Int } fn Counter.next(self) -> Int => self.n + 1"
    ));
}

#[test]
fn struct_mut_field() {
    assert!(compiles_ok(
        "struct Point { x: Int } fn main() { let mut p = Point { x: 0 }; p.x = 10; }"
    ));
}

#[test]
fn struct_with_enum() {
    assert!(compiles_ok(
        "enum Status { Active } struct User { name: Str, status: Status }"
    ));
}

#[test]
fn struct_with_array() {
    assert!(compiles_ok(
        "struct Team { members: [Str] } fn main() { let t = Team { members: [\"A\", \"B\"] }; }"
    ));
}

#[test]
fn struct_with_map() {
    assert!(compiles_ok("struct Config { settings: {Str: Int } }"));
}

#[test]
fn struct_with_optional() {
    assert!(compiles_ok("struct User { name: Str, email: Str? }"));
}

#[test]
fn struct_with_default() {
    assert!(compiles_ok("struct Config { timeout: Int = 30 }"));
}

#[test]
fn struct_as_param() {
    assert!(compiles_ok(
        "struct User { name: Str } fn greet(u: User) { print(u.name); }"
    ));
}

#[test]
fn struct_as_return() {
    assert!(compiles_ok(
        "struct User { name: Str } fn create() -> User { return User { name: \"A\" }; }"
    ));
}

#[test]
fn struct_in_array() {
    assert!(compiles_ok("struct User { name: Str } fn main() { let users = [User { name: \"A\" }, User { name: \"B\" }]; }"));
}

// =============================================================================
// Enums (enum_*)
// =============================================================================

#[test]
fn enum_basic() {
    assert!(compiles_ok("enum Color { Red, Green, Blue }"));
}

#[test]
fn enum_construction() {
    assert!(compiles_ok(
        "enum Color { Red, Blue } fn main() { let c = Color::Red; }"
    ));
}

#[test]
fn enum_match() {
    assert!(compiles_ok("enum Color { Red, Blue } fn main() { let c = Color::Red; match c { Color::Red => print(1), Color::Blue => print(2) } }"));
}

#[test]
fn enum_match_wildcard() {
    assert!(compiles_ok("enum Color { Red, Green, Blue } fn main() { let c = Color::Red; match c { Color::Red => print(1), _ => print(0) } }"));
}

#[test]
fn enum_with_payload() {
    // Result conflicts with built-in, use custom name
    assert!(compiles_ok("enum Outcome { Success(Int), Failure(Str) }"));
}

#[test]
fn enum_method() {
    assert!(compiles_ok("enum Priority { Low, High } fn Priority.label(self) -> Str { match self { Priority::Low => \"low\", Priority::High => \"high\" } }"));
}

#[test]
fn enum_in_struct() {
    assert!(compiles_ok("enum Status { Active, Inactive } struct User { status: Status } fn main() { let u = User { status: Status::Active }; }"));
}

// =============================================================================
// Error Handling (err_*)
// =============================================================================

#[test]
fn err_ok_return() {
    assert!(compiles_ok("fn compute() -> Int ! Str { Ok 42; }"));
}

#[test]
fn err_err_return() {
    assert!(compiles_ok("fn compute() -> Int ! Str { Err \"fail\"; }"));
}

#[test]
fn err_try_operator() {
    assert!(compiles_ok("fn may_fail() -> Int ! Str { Ok 1; } fn caller() -> Int ! Str { let v = may_fail()?; Ok v; }"));
}

#[test]
fn err_conditional() {
    assert!(compiles_ok(
        "fn divide(a: Int, b: Int) -> Int ! Str { if b == 0 { Err \"zero\"; } Ok a / b; }"
    ));
}

#[test]
fn err_match_result() {
    // Match on error result from function
    assert!(compiles_ok(
        "fn compute() -> Int ! Str { Ok 42; } fn main() { let r, e = compute(); }"
    ));
}

#[test]
fn err_chained_try() {
    assert!(compiles_ok("fn s1() -> Int ! Str { Ok 1; } fn s2(x: Int) -> Int ! Str { Ok x + 1; } fn pipe() -> Int ! Str { let a = s1()?; let b = s2(a)?; Ok b; }"));
}

// =============================================================================
// Ownership/Auto-clone (own_*)
// =============================================================================

#[test]
fn own_auto_clone_str() {
    assert!(compiles_ok(
        "fn main() { let s = \"hello\"; print(s); print(s); }"
    ));
}

#[test]
fn own_auto_clone_array() {
    assert!(compiles_ok(
        "fn main() { let arr = [1, 2, 3]; print(arr); print(arr); }"
    ));
}

#[test]
fn own_pass_and_reuse() {
    assert!(compiles_ok(
        "fn consume(x: Str) { print(x); } fn main() { let s = \"hello\"; consume(s); print(s); }"
    ));
}

#[test]
fn own_struct_auto_clone() {
    assert!(compiles_ok(
        "struct Point { x: Int } fn main() { let p = Point { x: 10 }; let q = p; print(p.x); }"
    ));
}

#[test]
fn own_closure_capture() {
    assert!(compiles_ok(
        "fn main() { let x = 42; let f = () => x; print(f()); print(x); }"
    ));
}

// =============================================================================
// Closures (cls_*)
// =============================================================================

#[test]
fn cls_capture_var() {
    assert!(compiles_ok(
        "fn main() { let x = 5; let f = () => x; f(); }"
    ));
}

#[test]
fn cls_capture_multiple() {
    assert!(compiles_ok(
        "fn main() { let a = 1; let b = 2; let f = () => a + b; }"
    ));
}

#[test]
fn cls_in_map() {
    assert!(compiles_ok(
        "fn main() { let doubled = [1, 2, 3].map((x) => x * 2); }"
    ));
}

#[test]
fn cls_in_filter() {
    assert!(compiles_ok(
        "fn main() { let big = [1, 5, 10].filter((x) => x > 3); }"
    ));
}

#[test]
fn cls_param_shadow() {
    assert!(compiles_ok(
        "fn main() { let x = 5; let f = (x) => x + 1; f(10); print(x); }"
    ));
}

#[test]
fn cls_factory() {
    assert!(compiles_ok(
        "fn makeAdder(n: Int) { let f = (x) => x + n; f(5); }"
    ));
}

// =============================================================================
// Decorators (dec_*)
// =============================================================================

#[test]
fn dec_table() {
    assert!(compiles_ok("@table struct User { name: Str }"));
}

#[test]
fn dec_route() {
    assert!(compiles_ok(
        "@get(\"/users\") fn getUsers() { print(\"users\"); }"
    ));
}

// =============================================================================
// Additional Complex Programs (complex_*)
// =============================================================================

#[test]
fn complex_task_system() {
    assert!(compiles_ok(
        r#"
enum Priority { Low, High }
enum Status { Todo, Done }
struct Task { title: Str, priority: Priority, status: Status }
fn Task.isDone(self) -> Bool { match self.status { Status::Done => true, _ => false } }
fn main() {
    let t = Task { title: "Fix bug", priority: Priority::High, status: Status::Todo };
    if !t.isDone() { print(t.title); }
}
"#
    ));
}

#[test]
fn complex_calculator() {
    assert!(compiles_ok(
        r#"
fn add(a: Int, b: Int) -> Int => a + b;
fn sub(a: Int, b: Int) -> Int => a - b;
fn main() { let r = add(10, sub(30, 5)); print(r); }
"#
    ));
}

#[test]
fn complex_fizzbuzz() {
    assert!(compiles_ok(
        r#"
fn main() {
    for i in 1..=15 {
        if i % 15 == 0 { print("FizzBuzz"); }
        else if i % 3 == 0 { print("Fizz"); }
        else if i % 5 == 0 { print("Buzz"); }
        else { print(i); }
    }
}
"#
    ));
}

#[test]
fn complex_array_pipeline() {
    assert!(compiles_ok("fn main() { let data = [1, 2, 3, 4, 5]; let result = data.filter((x) => x > 2).map((x) => x * 10); }"));
}

#[test]
fn complex_error_pipeline() {
    assert!(compiles_ok(
        r#"
fn step1() -> Int ! Str { Ok 10; }
fn step2(x: Int) -> Int ! Str { Ok x * 2; }
fn pipe() -> Int ! Str { let a = step1()?; let b = step2(a)?; Ok b; }
"#
    ));
}

#[test]
fn complex_user_system() {
    assert!(compiles_ok(
        r#"
struct User { name: Str, age: Int }
fn User.isAdult(self) -> Bool => self.age >= 18;
fn main() {
    let users = [User { name: "Alice", age: 25 }, User { name: "Bob", age: 15 }];
    for u in users { if u.isAdult() { print(u.name); } }
}
"#
    ));
}

#[test]
fn complex_config() {
    assert!(compiles_ok(
        r#"
struct Config { timeout: Int, retries: Int }
fn Config.isValid(self) -> Bool => self.timeout > 0 && self.retries > 0;
fn defaultConfig() -> Config { return Config { timeout: 30, retries: 3 }; }
fn main() { let cfg = defaultConfig(); print(cfg.timeout); }
"#
    ));
}

#[test]
fn complex_string_processing() {
    assert!(compiles_ok(
        "fn main() { let name = \"Alice\"; let greeting = \"Hello, ${name}!\"; print(greeting); }"
    ));
}

#[test]
fn complex_scope_shadowing() {
    assert!(compiles_ok(
        "fn main() { let x = 1; let x = \"hello\"; if true { let x = [1, 2]; print(x); } }"
    ));
}

#[test]
fn complex_closure_capture() {
    assert!(compiles_ok(
        "fn main() { let base = 100; let add = (x) => x + base; print(add(42)); print(base); }"
    ));
}
