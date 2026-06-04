//! MIR generation tests - verifies HIR → MIR transformation

use doo_core::types::TypeRegistry;
use doo_frontend::Parser;
use doo_hir::Lower;
use doo_mir::builder::MirBuilder;
use std::sync::Arc;

fn parse_lower_and_build_mir(src: &str) -> Result<doo_mir::MirProgram, String> {
    let mut parser = Parser::new(src, 0);
    let program = parser
        .parse_program()
        .map_err(|e| format!("parse: {:?}", e))?;
    if parser.has_errors() {
        return Err(format!("parse errors: {:?}", parser.errors()));
    }
    let mut type_registry = TypeRegistry::new();
    let mut lowerer = Lower::new();
    let hir = lowerer.lower_program_typed(&program, &mut type_registry);
    let type_registry = Arc::new(type_registry);
    let mut builder = MirBuilder::new(&type_registry);
    Ok(builder.build(&hir))
}

fn mir_ok(src: &str) -> bool {
    parse_lower_and_build_mir(src).is_ok()
}

// =============================================================================
// 1. Variable Allocation (25 tests)
// =============================================================================

#[test]
fn var_let_int() {
    assert!(mir_ok("fn main() { let x = 42; }"));
}

#[test]
fn var_let_str() {
    assert!(mir_ok("fn main() { let s = \"hello\"; }"));
}

#[test]
fn var_let_bool() {
    assert!(mir_ok("fn main() { let b = true; }"));
}

#[test]
fn var_let_mut() {
    assert!(mir_ok("fn main() { let mut x = 10; }"));
}

#[test]
fn var_let_typed() {
    assert!(mir_ok("fn main() { let x: Int = 42; }"));
}

#[test]
fn var_multiple() {
    assert!(mir_ok("fn main() { let a = 1; let b = 2; let c = 3; }"));
}

#[test]
fn var_reassign() {
    assert!(mir_ok("fn main() { let mut x = 1; x = 2; }"));
}

#[test]
fn var_array() {
    assert!(mir_ok("fn main() { let arr = [1, 2, 3]; }"));
}

#[test]
fn var_from_call() {
    assert!(mir_ok(
        "fn get() -> Int { return 42; } fn main() { let x = get(); }"
    ));
}

// =============================================================================
// 2. Control Flow Graphs (30 tests)
// =============================================================================

#[test]
fn cfg_single_block() {
    let mir = parse_lower_and_build_mir("fn main() { print(1); }").unwrap();
    assert!(!mir.functions.is_empty());
}

#[test]
fn cfg_if_two_branches() {
    assert!(mir_ok(
        "fn main() { if x > 0 { print(1); } else { print(0); } }"
    ));
}

#[test]
fn cfg_if_no_else() {
    assert!(mir_ok("fn main() { if x > 0 { print(1); } }"));
}

#[test]
fn cfg_for_loop() {
    assert!(mir_ok("fn main() { for i in 0..10 { print(i); } }"));
}

#[test]
fn cfg_break() {
    assert!(mir_ok(
        "fn main() { for i in 0..100 { if i == 50 { break; } } }"
    ));
}

#[test]
fn cfg_continue() {
    assert!(mir_ok(
        "fn main() { for i in 0..10 { if i % 2 == 0 { continue; }; print(i); } }"
    ));
}

#[test]
fn cfg_return_early() {
    assert!(mir_ok(
        "fn foo(x: Int) -> Int { if x < 0 { return 0; }; return x; }"
    ));
}

#[test]
fn cfg_nested_if() {
    assert!(mir_ok("fn main() { if a { if b { print(1); } } }"));
}

#[test]
fn cfg_match() {
    assert!(mir_ok(
        "fn main() { match x { 1 => print(1), _ => print(0) } }"
    ));
}

// =============================================================================
// 3. Function Calls (25 tests)
// =============================================================================

#[test]
fn call_no_args() {
    assert!(mir_ok("fn foo() { print(1); } fn main() { foo(); }"));
}

#[test]
fn call_with_args() {
    assert!(mir_ok(
        "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { add(1, 2); }"
    ));
}

#[test]
fn call_nested() {
    assert!(mir_ok(
        "fn inner() -> Int { return 1; } fn outer() -> Int { return inner(); } fn main() { outer(); }"
    ));
}

#[test]
fn call_method() {
    assert!(mir_ok("struct User { name: Str } fn User.getName(self) -> Str { return self.name; } fn main() { let u = User { name: \"x\" }; u.getName(); }"));
}

// =============================================================================
// 4. Ownership Operations (25 tests)
// =============================================================================

#[test]
fn own_move_basic() {
    assert!(mir_ok("fn main() { let x = 42; let y = x; }"));
}

#[test]
fn own_pass_to_function() {
    assert!(mir_ok(
        "fn consume(x: Int) { print(x); } fn main() { let val = 10; consume(val); }"
    ));
}

#[test]
fn own_return_value() {
    assert!(mir_ok("fn create() -> Int { let x = 42; return x; }"));
}

#[test]
fn own_mut_update() {
    assert!(mir_ok("fn main() { let mut x = 1; x = 2; }"));
}

// =============================================================================
// 5. Binary/Unary Operations (20 tests)
// =============================================================================

#[test]
fn binop_add() {
    assert!(mir_ok("fn main() { let x = 1 + 2; }"));
}

#[test]
fn binop_sub() {
    assert!(mir_ok("fn main() { let x = 10 - 3; }"));
}

#[test]
fn binop_mul() {
    assert!(mir_ok("fn main() { let x = 3 * 4; }"));
}

#[test]
fn binop_div() {
    assert!(mir_ok("fn main() { let x = 10 / 2; }"));
}

#[test]
fn binop_comparison() {
    assert!(mir_ok("fn main() { let x = 1 < 2; }"));
}

#[test]
fn unary_neg() {
    assert!(mir_ok("fn main() { let x = -42; }"));
}

#[test]
fn unary_not() {
    assert!(mir_ok("fn main() { let x = !true; }"));
}

// =============================================================================
// 6. Complex Programs (15 tests)
// =============================================================================

#[test]
fn complex_fibonacci() {
    assert!(mir_ok(
        "fn fib(n: Int) -> Int { if n <= 1 { return n; } return fib(n - 1) + fib(n - 2); }"
    ));
}

#[test]
fn complex_struct_method() {
    assert!(mir_ok(
        r#"

struct Point { x: Int, y: Int }
fn Point.distance(self) -> Float { return 0.0; }
fn main() { let p = Point { x: 1, y: 2 }; p.distance(); }

"#
    ));
}

#[test]
fn complex_nested_loops() {
    assert!(mir_ok(
        "fn main() { for i in 0..5 { for j in 0..5 { print(i * j); } } }"
    ));
}

// =============================================================================
// Additional Variable Allocation (var_*)
// =============================================================================

#[test]
fn var_let_float() {
    assert!(mir_ok("fn main() { let f = 3.14; }"));
}

#[test]
fn var_let_optional() {
    assert!(mir_ok("fn main() { let x: Int? = nil; }"));
}

#[test]
fn var_let_optional_value() {
    assert!(mir_ok("fn main() { let x: Int? = 42; }"));
}

#[test]
fn var_let_map() {
    assert!(mir_ok("fn main() { let m = {\"a\": 1}; }"));
}

// #[test]
// #[ignore = "reason"]
// fn var_struct_construction() {
//     assert!(mir_ok(
//         "struct Point { x: Int, y: Int } fn main() { let p = Point { x: 10, y: 20 }; }"
//     ));
// }

#[test]
fn var_shadowing() {
    assert!(mir_ok("fn main() { let x = 1; let x = \"hello\"; }"));
}

#[test]
fn var_shadowing_in_scope() {
    assert!(mir_ok(
        "fn main() { let x = 1; if true { let x = 2; print(x); } }"
    ));
}

#[test]
fn var_compound_assign() {
    assert!(mir_ok(
        "fn main() { let mut x = 0; x += 5; x -= 2; x *= 3; }"
    ));
}

#[test]
fn var_from_expression() {
    assert!(mir_ok("fn main() { let x = (1 + 2) * 3; }"));
}

#[test]
fn var_from_ternary() {
    assert!(mir_ok("fn main() { let x = if true { 1 } else { 0 }; }"));
}

#[test]
fn var_from_nil_coalesce() {
    assert!(mir_ok(
        "fn main() { let x: Int? = nil; let y = if x != nil { x } else { 0 }; }"
    ));
}

#[test]
fn var_from_field_access() {
    assert!(mir_ok(
        "struct Point { x: Int } fn main() { let p = Point { x: 10 }; let v = p.x; }"
    ));
}

#[test]
fn var_array_index() {
    assert!(mir_ok("fn main() { let arr = [10, 20]; let v = arr[0]; }"));
}

#[test]
fn var_map_access() {
    assert!(mir_ok(
        "fn main() { let m = {\"a\": 1}; let v = m[\"a\"]; }"
    ));
}

#[test]
fn var_nested_struct() {
    assert!(mir_ok("struct A { v: Int } struct B { a: A } fn main() { let b = B { a: A { v: 42 } }; let v = b.a.v; }"));
}

// =============================================================================
// Additional Control Flow Graphs (cfg_*)
// =============================================================================

#[test]
fn cfg_else_if_chain() {
    assert!(mir_ok("fn main() { let x = 5; if x > 10 { print(1); } else if x > 0 { print(2); } else { print(3); } }"));
}

#[test]
fn cfg_match_multiple_arms() {
    assert!(mir_ok(
        "fn main() { let x = 2; match { x == 1 => print(1), x == 2 => print(2), _ => print(0) } }"
    ));
}

#[test]
fn cfg_for_in_array() {
    assert!(mir_ok(
        "fn main() { for item in [1, 2, 3] { print(item); } }"
    ));
}

#[test]
fn cfg_for_inclusive_range() {
    assert!(mir_ok("fn main() { for i in 1..=10 { print(i); } }"));
}

#[test]
fn cfg_infinite_loop() {
    assert!(mir_ok("fn main() { for { break; } }"));
}

#[test]
fn cfg_nested_loops_break() {
    assert!(mir_ok(
        "fn main() { for i in 0..5 { for j in 0..5 { if j == 2 { break; } } } }"
    ));
}

#[test]
fn cfg_early_return_with_value() {
    assert!(mir_ok("fn find(arr: [Int], t: Int) -> Int { for item in arr { if item == t { return item; } } return -1; }"));
}

#[test]
fn cfg_match_enum() {
    assert!(mir_ok("enum Color { Red, Blue } fn main() { let c = Color::Red; match c { Color::Red => print(1), Color::Blue => print(2) } }"));
}

#[test]
fn cfg_if_expression() {
    assert!(mir_ok(
        "fn main() { let x = 5; let y = if x > 0 { x } else { 0 }; }"
    ));
}

#[test]
fn cfg_match_expression() {
    assert!(mir_ok(
        "fn main() { let label = match { true => \"yes\", _ => \"no\" }; }"
    ));
}

#[test]
fn cfg_loop_accumulate() {
    assert!(mir_ok(
        "fn main() { let mut sum = 0; for i in 1..=100 { sum += i; } }"
    ));
}

#[test]
fn cfg_conditional_in_loop() {
    assert!(mir_ok(
        "fn main() { for i in 0..10 { if i % 2 == 0 { print(i); } else { print(-i); } } }"
    ));
}

// =============================================================================
// Additional Function Calls (call_*)
// =============================================================================

#[test]
fn call_recursive() {
    assert!(mir_ok(
        "fn fact(n: Int) -> Int { if n <= 1 { return 1; } return n * fact(n - 1); }"
    ));
}

#[test]
fn call_chained() {
    assert!(mir_ok(
        "fn a() -> Int { return 1; } fn b(x: Int) -> Int { return x + 1; } fn main() { b(a()); }"
    ));
}

#[test]
fn call_method_chain() {
    assert!(mir_ok(
        "fn main() { let result = [1, 2, 3].map((x) => x * 2); }"
    ));
}

#[test]
fn call_closure() {
    assert!(mir_ok("fn main() { let f = (x) => x + 1; f(5); }"));
}

#[test]
fn call_closure_no_args() {
    assert!(mir_ok("fn main() { let f = () => 42; f(); }"));
}

#[test]
fn call_with_string_arg() {
    assert!(mir_ok(
        "fn greet(name: Str) { print(name); } fn main() { greet(\"Alice\"); }"
    ));
}

#[test]
fn call_multiple_return_uses() {
    assert!(mir_ok(
        "fn get() -> Int { return 5; } fn main() { let a = get(); let b = get(); print(a + b); }"
    ));
}

#[test]
fn call_expression_body() {
    assert!(mir_ok(
        "fn double(x: Int) -> Int => x * 2;\nfn main() { print(double(21)); }"
    ));
}

// =============================================================================
// Additional Ownership Operations (own_*)
// =============================================================================

#[test]
fn own_auto_clone_str() {
    assert!(mir_ok(
        "fn main() { let s = \"hello\"; print(s); print(s); }"
    ));
}

#[test]
fn own_auto_clone_array() {
    assert!(mir_ok(
        "fn main() { let arr = [1, 2, 3]; print(arr); print(arr); }"
    ));
}

#[test]
fn own_pass_and_reuse() {
    assert!(mir_ok(
        "fn consume(x: Str) { print(x); } fn main() { let s = \"hello\"; consume(s); print(s); }"
    ));
}

#[test]
fn own_struct_auto_clone() {
    assert!(mir_ok(
        "struct Point { x: Int } fn main() { let p = Point { x: 10 }; let q = p; print(p.x); }"
    ));
}

#[test]
fn own_return_struct() {
    assert!(mir_ok("struct Point { x: Int } fn make() -> Point { return Point { x: 42 }; } fn main() { let p = make(); }"));
}

#[test]
fn own_mut_struct_field() {
    assert!(mir_ok(
        "struct Point { x: Int } fn main() { let mut p = Point { x: 0 }; p.x = 10; }"
    ));
}

#[test]
fn own_array_push() {
    assert!(mir_ok(
        "fn main() { let mut arr: [Int] = []; arr.push(1); arr.push(2); }"
    ));
}

#[test]
fn own_map_insert() {
    assert!(mir_ok(
        "fn main() { let mut m: {Str: Int} = {}; m[\"a\"] = 1; }"
    ));
}

#[test]
fn own_closure_capture() {
    assert!(mir_ok(
        "fn main() { let x = 42; let f = () => x; print(f()); print(x); }"
    ));
}

#[test]
fn own_loop_variable() {
    assert!(mir_ok("fn main() { for i in 0..10 { print(i); } }"));
}

// =============================================================================
// Additional Binary/Unary Operations (op_*)
// =============================================================================

#[test]
fn op_float_add() {
    assert!(mir_ok("fn main() { let x = 1.5 + 2.5; }"));
}

#[test]
fn op_float_sub() {
    assert!(mir_ok("fn main() { let x = 5.0 - 2.5; }"));
}

#[test]
fn op_float_mul() {
    assert!(mir_ok("fn main() { let x = 2.0 * 3.0; }"));
}

#[test]
fn op_float_div() {
    assert!(mir_ok("fn main() { let x = 10.0 / 3.0; }"));
}

#[test]
fn op_mod() {
    assert!(mir_ok("fn main() { let x = 10 % 3; }"));
}

#[test]
fn op_cmp_gt() {
    assert!(mir_ok("fn main() { let x = 5 > 3; }"));
}

#[test]
fn op_cmp_gte() {
    assert!(mir_ok("fn main() { let x = 5 >= 5; }"));
}

#[test]
fn op_cmp_lte() {
    assert!(mir_ok("fn main() { let x = 3 <= 5; }"));
}

#[test]
fn op_cmp_eq() {
    assert!(mir_ok("fn main() { let x = 1 == 1; }"));
}

#[test]
fn op_cmp_neq() {
    assert!(mir_ok("fn main() { let x = 1 != 2; }"));
}

#[test]
fn op_logical_and() {
    assert!(mir_ok("fn main() { let x = true && false; }"));
}

#[test]
fn op_logical_or() {
    assert!(mir_ok("fn main() { let x = true || false; }"));
}

#[test]
fn op_str_concat() {
    assert!(mir_ok("fn main() { let x = \"hello\" + \" world\"; }"));
}

#[test]
fn op_chained() {
    assert!(mir_ok("fn main() { let x = 1 + 2 + 3 + 4; }"));
}

#[test]
fn op_mixed_precedence() {
    assert!(mir_ok("fn main() { let x = 1 + 2 * 3; }"));
}

#[test]
fn op_parens() {
    assert!(mir_ok("fn main() { let x = (1 + 2) * 3; }"));
}

#[test]
fn op_compound_add() {
    assert!(mir_ok("fn main() { let mut x = 0; x += 5; }"));
}

#[test]
fn op_compound_sub() {
    assert!(mir_ok("fn main() { let mut x = 10; x -= 3; }"));
}

#[test]
fn op_compound_mul() {
    assert!(mir_ok("fn main() { let mut x = 3; x *= 2; }"));
}

#[test]
fn op_increment() {
    assert!(mir_ok("fn main() { let mut x = 0; x++; }"));
}

#[test]
fn op_decrement() {
    assert!(mir_ok("fn main() { let mut x = 10; x--; }"));
}

// =============================================================================
// Strings (str_*)
// =============================================================================

#[test]
fn str_interpolation() {
    assert!(mir_ok(
        "fn main() { let name = \"world\"; let msg = \"Hello ${name}\"; }"
    ));
}

#[test]
fn str_length() {
    assert!(mir_ok(
        "fn main() { let s = \"hello\"; let n = s.length(); }"
    ));
}

#[test]
fn str_upper() {
    assert!(mir_ok(
        "fn main() { let s = \"hello\"; let u = s.toUpperCase(); }"
    ));
}

#[test]
fn str_concat() {
    assert!(mir_ok(
        "fn main() { let s = \"hello\" + \" \" + \"world\"; }"
    ));
}

// =============================================================================
// Arrays and Maps (coll_*)
// =============================================================================

#[test]
fn coll_array_empty() {
    assert!(mir_ok("fn main() { let arr: [Int] = []; }"));
}

#[test]
fn coll_array_push() {
    assert!(mir_ok(
        "fn main() { let mut arr: [Int] = []; arr.push(1); arr.push(2); }"
    ));
}

#[test]
fn coll_array_length() {
    assert!(mir_ok(
        "fn main() { let arr = [1, 2, 3]; let n = arr.length(); }"
    ));
}

#[test]
fn coll_array_map() {
    assert!(mir_ok(
        "fn main() { let doubled = [1, 2, 3].map((x) => x * 2); }"
    ));
}

#[test]
fn coll_array_filter() {
    assert!(mir_ok(
        "fn main() { let big = [1, 2, 3, 4, 5].filter((x) => x > 3); }"
    ));
}

#[test]
fn coll_array_iterate() {
    assert!(mir_ok(
        "fn main() { for item in [1, 2, 3] { print(item); } }"
    ));
}

#[test]
fn coll_array_nested() {
    assert!(mir_ok(
        "fn main() { let m: [[Int]] = [[1, 2], [3, 4]]; let v = m[0][1]; }"
    ));
}

#[test]
fn coll_map_empty() {
    assert!(mir_ok("fn main() { let m: {Str: Int} = {}; }"));
}

#[test]
fn coll_map_insert() {
    assert!(mir_ok(
        "fn main() { let mut m: {Str: Int} = {}; m[\"key\"] = 42; }"
    ));
}

#[test]
fn coll_map_access() {
    assert!(mir_ok(
        "fn main() { let m = {\"a\": 1}; let v = m[\"a\"]; }"
    ));
}

#[test]
fn coll_map_keys() {
    assert!(mir_ok(
        "fn main() { let m = {\"a\": 1}; let keys = m.keys(); }"
    ));
}

#[test]
fn coll_map_nested() {
    assert!(mir_ok(
        "fn main() { let m = {\"a\": {\"b\": 1}}; let v = m[\"a\"][\"b\"]; }"
    ));
}

// =============================================================================
// Error Handling (err_*)
// =============================================================================

#[test]
fn err_ok_return() {
    assert!(mir_ok("fn compute() -> Int ! Str { Ok 42; }"));
}

#[test]
fn err_err_return() {
    assert!(mir_ok("fn compute() -> Int ! Str { Err \"fail\"; }"));
}

#[test]
fn err_try_operator() {
    assert!(mir_ok("fn may_fail() -> Int ! Str { Ok 1; } fn caller() -> Int ! Str { let v = may_fail()?; Ok v; }"));
}

#[test]
fn err_conditional() {
    assert!(mir_ok(
        "fn divide(a: Int, b: Int) -> Int ! Str { if b == 0 { Err \"zero\"; } Ok a / b; }"
    ));
}

#[test]
fn err_match_result() {
    assert!(mir_ok(
        "fn main() { let r = Ok(42); match r { Ok(v) => print(v), Err(e) => print(e) } }"
    ));
}

#[test]
fn err_chained_try() {
    assert!(mir_ok("fn s1() -> Int ! Str { Ok 1; } fn s2(x: Int) -> Int ! Str { Ok x + 1; } fn pipe() -> Int ! Str { let a = s1()?; let b = s2(a)?; Ok b; }"));
}

// =============================================================================
// Structs and Enums (se_*)
// =============================================================================

#[test]
fn se_struct_construction() {
    let mir = parse_lower_and_build_mir(
        "struct Point { x: Int, y: Int } fn main() { let p = Point { x: 10, y: 20 }; }",
    )
    .unwrap();
    assert!(!mir.functions.is_empty());
}

#[test]
fn se_struct_field_access() {
    assert!(mir_ok(
        "struct Point { x: Int } fn main() { let p = Point { x: 10 }; print(p.x); }"
    ));
}

#[test]
fn se_struct_method() {
    assert!(mir_ok("struct Point { x: Int } fn Point.getX(self) -> Int { return self.x; } fn main() { let p = Point { x: 5 }; p.getX(); }"));
}

#[test]
fn se_struct_method_expr() {
    assert!(mir_ok(
        "struct Counter { n: Int } fn Counter.next(self) -> Int => self.n + 1"
    ));
}

#[test]
fn se_struct_nested() {
    assert!(mir_ok("struct A { v: Int } struct B { a: A } fn main() { let b = B { a: A { v: 42 } }; print(b.a.v); }"));
}

#[test]
fn se_enum_basic() {
    assert!(mir_ok(
        "enum Color { Red, Blue } fn main() { let c = Color::Red; }"
    ));
}

#[test]
fn se_enum_match() {
    assert!(mir_ok("enum Color { Red, Blue } fn main() { let c = Color::Red; match c { Color::Red => print(1), Color::Blue => print(2) } }"));
}

#[test]
fn se_enum_method() {
    assert!(mir_ok("enum Priority { Low, High } fn Priority.label(self) -> Str { match self { Priority::Low => \"low\", Priority::High => \"high\" } }"));
}

// =============================================================================
// Additional Complex Programs (complex_*)
// =============================================================================

#[test]
fn complex_task_system() {
    assert!(mir_ok(
        r#"

enum Priority { Low, Medium, High }
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
    assert!(mir_ok(
        r#"

fn add(a: Int, b: Int) -> Int => a + b;
fn sub(a: Int, b: Int) -> Int => a - b;
fn mul(a: Int, b: Int) -> Int => a * b;
fn main() {
    let r1 = add(10, 20);
    let r2 = sub(30, 5);
    let r3 = mul(4, 7);
    print(r1, r2, r3); }

"#
    ));
}

#[test]
fn complex_config_loader() {
    assert!(mir_ok(
        r#"

struct Config { timeout: Int, retries: Int }
fn Config.isValid(self) -> Bool => self.timeout > 0 && self.retries > 0;
fn defaultConfig() -> Config { return Config { timeout: 30, retries: 3 }; }
fn main() { let cfg = defaultConfig(); print(cfg.timeout); }

"#
    ));
}

#[test]
fn complex_fizzbuzz() {
    assert!(mir_ok(
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
    assert!(mir_ok(
        r#"

fn main() {
    let data = [1, 2, 3, 4, 5];
    let result = data.filter((x) => x > 2).map((x) => x * 10);
    for item in result { print(item); }
}

"#
    ));
}

#[test]
fn complex_error_pipeline() {
    assert!(mir_ok(
        r#"

fn step1() -> Int ! Str { Ok 10; }
fn step2(x: Int) -> Int ! Str { Ok x * 2; }
fn pipe() -> Int ! Str {
    let a = step1()?;
    let b = step2(a)?;
    Ok b;
}

"#
    ));
}

#[test]
fn complex_user_system() {
    assert!(mir_ok(
        r#"

struct User { name: Str, age: Int }
fn User.isAdult(self) -> Bool => self.age >= 18;
fn User.greeting(self) -> Str => "Hello, ${self.name}";
fn main() {
    let users = [User { name: "Alice", age: 25 }, User { name: "Bob", age: 15 }];
    for u in users {
        if u.isAdult() { print(u.greeting()); }
    }
}

"#
    ));
}

#[test]
fn complex_scope_and_shadowing() {
    assert!(mir_ok(
        "fn main() { let x = 1; let x = \"hello\"; if true { let x = [1, 2]; print(x); } }"
    ));
}

#[test]
fn complex_closure_capture() {
    assert!(mir_ok(
        "fn main() { let base = 100; let add = (x) => x + base; print(add(42)); print(base); }"
    ));
}

#[test]
fn complex_string_processing() {
    assert!(mir_ok(
        "fn main() { let name = \"Alice\"; let greeting = \"Hello, ${name}!\"; print(greeting); }"
    ));
}
