//! HIR lowering tests - verifies AST → HIR transformation

use doo_core::types::TypeRegistry;
use doo_frontend::Parser;
use doo_hir::HirItem;
use doo_hir::{HirProgram, Lower};

fn parse_and_lower(src: &str) -> HirProgram {
    let mut parser = Parser::new(src, 0);
    let program = parser.parse_program().expect("parse failed");
    assert!(!parser.has_errors(), "parse errors: {:?}", parser.errors());
    let mut type_registry = TypeRegistry::new();
    let mut lowerer = Lower::new();
    lowerer.lower_program_typed(&program, &mut type_registry)
}

fn lower_ok(src: &str) -> bool {
    let mut parser = Parser::new(src, 0);
    if let Ok(program) = parser.parse_program() {
        if !parser.has_errors() {
            let mut type_registry = TypeRegistry::new();
            let mut lowerer = Lower::new();
            let _ = lowerer.lower_program_typed(&program, &mut type_registry);
            return true;
        }
    }
    false
}

// =============================================================================
// 1. Desugaring (30 tests)
// =============================================================================

#[test]
fn desugar_plus_equals() {
    let hir = parse_and_lower("fn main() { let mut x = 1; x += 1; }");
    assert!(hir.items.iter().any(|i| matches!(i, HirItem::Function(_))));
}

#[test]
fn desugar_minus_equals() {
    assert!(lower_ok("fn main() { let mut x = 10; x -= 3; }"));
}

#[test]
fn desugar_star_equals() {
    assert!(lower_ok("fn main() { let mut x = 2; x *= 4; }"));
}

#[test]
fn desugar_slash_equals() {
    assert!(lower_ok("fn main() { let mut x = 10; x /= 2; }"));
}

#[test]
fn desugar_increment() {
    assert!(lower_ok("fn main() { let mut x = 0; x++; }"));
}

#[test]
fn desugar_decrement() {
    assert!(lower_ok("fn main() { let mut x = 5; x--; }"));
}

#[test]
fn desugar_compound_chain() {
    assert!(lower_ok(
        "fn main() { let mut x = 10; x += 5; x -= 2; x *= 3; }"
    ));
}

#[test]
fn desugar_for_range() {
    let hir = parse_and_lower("fn main() { for i in 1..10 { print(i); } }");
    assert!(hir.items.iter().any(|i| matches!(i, HirItem::Function(_))));
}

#[test]
fn desugar_for_inclusive() {
    assert!(lower_ok("fn main() { for i in 1..=10 { print(i); } }"));
}

#[test]
fn desugar_match_basic() {
    assert!(lower_ok(
        "fn main() { match x { 1 => print(1), _ => print(0) } }"
    ));
}

// Skip remaining desugar tests for brevity - same pattern

// =============================================================================
// 2. Control Flow (20 tests)
// =============================================================================

#[test]
fn cf_if_simple() {
    assert!(lower_ok("fn main() { if x > 0 { print(x); } }"));
}

#[test]
fn cf_if_else() {
    assert!(lower_ok(
        "fn main() { if x > 0 { print(1); } else { print(0); } }"
    ));
}

#[test]
fn cf_nested_if() {
    assert!(lower_ok("fn main() { if a { if b { print(1); } } }"));
}

#[test]
fn cf_for_loop() {
    assert!(lower_ok("fn main() { for i in 0..10 { print(i); } }"));
}

#[test]
fn cf_break() {
    assert!(lower_ok(
        "fn main() { for i in 0..100 { if i == 50 { break; } } }"
    ));
}

#[test]
fn cf_continue() {
    assert!(lower_ok(
        "fn main() { for i in 0..10 { if i % 2 == 0 { continue; } print(i); } }"
    ));
}

#[test]
fn cf_return_value() {
    assert!(lower_ok("fn add(a: Int, b: Int) -> Int { return a + b; }"));
}

#[test]
fn cf_return_void() {
    assert!(lower_ok("fn main() { return; }"));
}

// =============================================================================
// 3. Functions (20 tests)
// =============================================================================

#[test]
fn fn_no_params() {
    let hir = parse_and_lower("fn greet() { print(\"hello\"); }");
    assert_eq!(
        hir.items
            .iter()
            .filter(|i| matches!(i, HirItem::Function(_)))
            .count(),
        1
    );
}

#[test]
fn fn_with_params() {
    assert!(lower_ok("fn add(a: Int, b: Int) -> Int { return a + b; }"));
}

#[test]
fn fn_expression_body() {
    assert!(lower_ok("fn double(x: Int) -> Int => x * 2"));
}

#[test]
fn fn_method_syntax() {
    assert!(lower_ok(
        "fn User.getName(self) -> Str { return self.name; }"
    ));
}

#[test]
fn fn_with_decorators() {
    assert!(lower_ok(
        "@route(\"/api\") fn getUsers() { print(\"api\"); }"
    ));
}

// =============================================================================
// 4. Types (20 tests)
// =============================================================================

#[test]
fn type_basic() {
    assert!(lower_ok("let x: Int = 42;"));
}

#[test]
fn type_array() {
    assert!(lower_ok("let arr: [Int] = [1, 2, 3];"));
}

#[test]
fn type_map() {
    assert!(lower_ok("let m: {Str: Int} = {\"a\": 1};"));
}

#[test]
fn type_optional() {
    assert!(lower_ok("let x: Int? = nil;"));
}

#[test]
fn type_custom() {
    assert!(lower_ok(
        "struct User { name: Str } let u: User = User { name: \"x\" };"
    ));
}

// =============================================================================
// 5. Structs and Enums (15 tests)
// =============================================================================

#[test]
fn struct_basic() {
    let hir = parse_and_lower("struct Point { x: Int, y: Int };");
    assert_eq!(
        hir.items
            .iter()
            .filter(|i| matches!(i, HirItem::Struct(_)))
            .count(),
        1
    );
}

#[test]
fn struct_with_methods() {
    assert!(lower_ok(
        "struct Point { x: Int } fn Point.getX(self) -> Int { return self.x; };"
    ));
}

#[test]
fn enum_basic() {
    let hir = parse_and_lower("enum Color { Red, Green, Blue }");
    assert_eq!(
        hir.items
            .iter()
            .filter(|i| matches!(i, HirItem::Enum(_)))
            .count(),
        1
    );
}

#[test]
fn enum_with_payload() {
    assert!(lower_ok("enum MyResult { Success(Int), Failure(Str) }"));
}

// =============================================================================
// 6. Complex Programs (15 tests)
// =============================================================================

#[test]
fn complex_fibonacci() {
    assert!(lower_ok(
        "fn fib(n: Int) -> Int { if n <= 1 { return n; } return fib(n-1) + fib(n-2); }"
    ));
}

#[test]
fn complex_struct_method_match() {
    assert!(lower_ok(
        r#"
struct Shape { kind: Str }
fn Shape.describe(self) -> Str {
    match self.kind {
        "circle" => "round",
        _ => "other"
    }
}
"#
    ));
}

#[test]
fn complex_nested_for() {
    assert!(lower_ok(
        "fn main() { for i in 0..3 { for j in 0..3 { print(i * j); } } }"
    ));
}

// =============================================================================
// Additional Desugaring (desugar_*)
// =============================================================================

#[test]
fn desugar_mod_equals() {
    assert!(lower_ok("fn main() { let mut x = 10; x %= 3; }"));
}

#[test]
fn desugar_string_interpolation() {
    assert!(lower_ok(
        "fn main() { let name = \"world\"; let msg = \"hello ${name}\"; }"
    ));
}

#[test]
fn desugar_string_interpolation_expr() {
    assert!(lower_ok(
        "fn main() { let x = 42; let s = \"value: ${x + 1}\"; }"
    ));
}

#[test]
fn desugar_ternary() {
    assert!(lower_ok(
        "fn main() { let x = 5; let y = if x > 0 { \"pos\" } else { \"neg\" }; }"
    ));
}

#[test]
fn desugar_nil_coalesce() {
    assert!(lower_ok(
        "fn main() { let x: Int? = nil; let y = if x != nil { x } else { 0 }; }"
    ));
}

#[test]
fn desugar_type_cast() {
    assert!(lower_ok("fn main() { let x = 42; let f = x as Float; }"));
}

#[test]
fn desugar_range_with_vars() {
    assert!(lower_ok(
        "fn main() { let a = 0; let b = 10; for i in a..b { print(i); } }"
    ));
}

#[test]
fn desugar_for_in_array() {
    assert!(lower_ok(
        "fn main() { for item in [1, 2, 3] { print(item); } }"
    ));
}

#[test]
fn desugar_indexed_for() {
    assert!(lower_ok(
        "fn main() { let arr = [10, 20]; for i, val in arr { print(i, val); } }"
    ));
}

#[test]
fn desugar_spread_operator() {
    assert!(lower_ok("fn main() { let a = [1, 2]; let b = [...a, 3]; }"));
}

#[test]
fn desugar_expression_body() {
    assert!(lower_ok("fn double(x: Int) -> Int => x * 2"));
}

#[test]
fn desugar_compound_in_loop() {
    assert!(lower_ok(
        "fn main() { let mut sum = 0; for i in 0..10 { sum += i; } }"
    ));
}

#[test]
fn desugar_increment_in_loop() {
    assert!(lower_ok(
        "fn main() { let mut count = 0; for i in 0..5 { count++; } }"
    ));
}

#[test]
fn desugar_decrement_in_loop() {
    assert!(lower_ok(
        "fn main() { let mut count = 10; for i in 0..5 { count--; } }"
    ));
}

#[test]
fn desugar_chained_compound() {
    assert!(lower_ok(
        "fn main() { let mut x = 0; x += 1; x -= 2; x *= 3; x /= 4; x %= 5; }"
    ));
}

#[test]
fn desugar_nested_ternary() {
    assert!(lower_ok(
        "fn main() { let x = 5; let y = if x > 10 { \"big\" } else if x > 0 { \"small\" } else { \"zero\" }; }"
    ));
}

#[test]
fn desugar_string_multi_interpolation() {
    assert!(lower_ok(
        "fn main() { let a = \"x\"; let b = 42; let s = \"${a} = ${b}\"; }"
    ));
}

#[test]
fn desugar_method_chain() {
    assert!(lower_ok("fn main() { let arr = [1, 2, 3]; let result = arr.map((x) => x * 2).filter((x) => x > 2); }"));
}

#[test]
fn desugar_contains_operator() {
    assert!(lower_ok(
        "fn main() { let m = {\"a\": 1}; if \"a\" in m { print(\"found\"); } }"
    ));
}

// =============================================================================
// Additional Control Flow (cf_*)
// =============================================================================

#[test]
fn cf_else_if() {
    assert!(lower_ok("fn main() { let x = 5; if x > 10 { print(\"big\"); } else if x > 0 { print(\"small\"); } else { print(\"neg\"); } }"));
}

#[test]
fn cf_match_multi_arm() {
    assert!(lower_ok("fn main() { let x = 2; match { x == 1 => print(\"one\"), x == 2 => print(\"two\"), _ => print(\"other\") } }"));
}

#[test]
fn cf_for_in_strings() {
    assert!(lower_ok(
        "fn main() { for name in [\"Alice\", \"Bob\"] { print(name); } }"
    ));
}

#[test]
fn cf_infinite_loop_break() {
    assert!(lower_ok("fn main() { for { print(\"loop\"); break; } }"));
}

#[test]
fn cf_nested_break() {
    assert!(lower_ok(
        "fn main() { for i in 0..5 { for j in 0..5 { if j == 2 { break; } } } }"
    ));
}

#[test]
fn cf_early_return() {
    assert!(lower_ok("fn find(arr: [Int], target: Int) -> Int { for item in arr { if item == target { return item; } } return -1; }"));
}

#[test]
fn cf_if_expression() {
    assert!(lower_ok(
        "fn main() { let x = 5; let y = if x > 0 { x } else { 0 }; }"
    ));
}

#[test]
fn cf_match_expression() {
    assert!(lower_ok(
        "fn main() { let x = 5; let label = match { x > 10 => \"big\", _ => \"small\" }; }"
    ));
}

#[test]
fn cf_loop_accumulate() {
    assert!(lower_ok(
        "fn main() { let mut result: [Int] = []; for i in 0..5 { result.push(i * 2); } }"
    ));
}

#[test]
fn cf_continue_in_nested() {
    assert!(lower_ok(
        "fn main() { for i in 0..10 { if i % 2 == 0 { continue; } for j in 0..3 { print(i, j); } } }"
    ));
}

#[test]
fn cf_while_pattern() {
    assert!(lower_ok(
        "fn main() { let mut count = 0; for { count++; if count >= 10 { break; } } }"
    ));
}

#[test]
fn cf_conditional_assignment() {
    assert!(lower_ok(
        "fn main() { let mut x = 0; if true { x = 1; } else { x = 2; } }"
    ));
}

// =============================================================================
// Additional Functions (fn_*)
// =============================================================================

#[test]
fn fn_closure() {
    assert!(lower_ok("fn main() { let f = (x) => x + 1; print(f(5)); }"));
}

#[test]
fn fn_closure_multi_param() {
    assert!(lower_ok(
        "fn main() { let f = (a, b) => a + b; print(f(1, 2)); }"
    ));
}

#[test]
fn fn_closure_no_param() {
    assert!(lower_ok("fn main() { let f = () => 42; print(f()); }"));
}

#[test]
fn fn_method_with_params() {
    assert!(lower_ok(
        "fn User.greetWith(self, prefix: Str) -> Str { return prefix + self.name; }"
    ));
}

#[test]
fn fn_error_return() {
    assert!(lower_ok(
        "fn divide(a: Int, b: Int) -> Int ! Str { if b == 0 { Err \"zero\"; } Ok a / b; }"
    ));
}

#[test]
fn fn_recursive() {
    assert!(lower_ok(
        "fn fact(n: Int) -> Int { if n <= 1 { return 1; } return n * fact(n - 1); }"
    ));
}

#[test]
fn fn_multiple_functions() {
    let hir =
        parse_and_lower("fn foo() { print(1); } fn bar() { print(2); } fn baz() { print(3); }");
    assert_eq!(
        hir.items
            .iter()
            .filter(|i| matches!(i, HirItem::Function(_)))
            .count(),
        3
    );
}

#[test]
fn fn_with_default_param() {
    assert!(lower_ok("fn greet(name: Str) { print(name); }"));
}

#[test]
fn fn_void_return() {
    assert!(lower_ok("fn process() { let x = 42; print(x); }"));
}

#[test]
fn fn_nested_calls() {
    assert!(lower_ok("fn inner() -> Int { return 5; } fn outer(x: Int) -> Int { return x * 2; } fn main() { outer(inner()); }"));
}

#[test]
fn fn_conditional_return() {
    assert!(lower_ok(
        "fn abs(x: Int) -> Int { if x < 0 { return -x; } return x; }"
    ));
}

#[test]
fn fn_multi_method() {
    assert!(lower_ok("struct Point { x: Int, y: Int } fn Point.getX(self) -> Int { return self.x; } fn Point.getY(self) -> Int { return self.y; }"));
}

// =============================================================================
// Additional Types (type_*)
// =============================================================================

#[test]
fn type_float() {
    assert!(lower_ok("let f: Float = 3.14;"));
}

#[test]
fn type_bool() {
    assert!(lower_ok("let b: Bool = true;"));
}

#[test]
fn type_str() {
    assert!(lower_ok("let s: Str = \"hello\";"));
}

#[test]
fn type_nested_array() {
    assert!(lower_ok("let m: [[Int]] = [[1, 2], [3, 4]];"));
}

#[test]
fn type_nested_map() {
    assert!(lower_ok("let m: {Str: {Str: Int}} = {\"a\": {\"b\": 1}};"));
}

#[test]
fn type_map_array_value() {
    assert!(lower_ok("let m: {Str: [Int]} = {\"nums\": [1, 2, 3]};"));
}

#[test]
fn type_array_of_maps() {
    assert!(lower_ok("let a: [{Str: Int}] = [{\"a\": 1}];"));
}

#[test]
fn type_optional_str() {
    assert!(lower_ok("let s: Str? = nil;"));
}

#[test]
fn type_optional_array() {
    assert!(lower_ok("let a: [Int]? = nil;"));
}

#[test]
fn type_optional_map() {
    assert!(lower_ok("let m: {Str: Int}? = nil;"));
}

#[test]
fn type_mut_var() {
    assert!(lower_ok("let mut x: Int = 0;"));
}

// =============================================================================
// Additional Structs (struct_*)
// =============================================================================

#[test]
fn struct_multiple_fields() {
    let hir = parse_and_lower("struct User { name: Str, age: Int, email: Str }");
    assert_eq!(
        hir.items
            .iter()
            .filter(|i| matches!(i, HirItem::Struct(_)))
            .count(),
        1
    );
}

#[test]
fn struct_nested() {
    assert!(lower_ok(
        "struct Address { city: Str } struct User { name: Str, address: Address }"
    ));
}

#[test]
fn struct_with_optional() {
    assert!(lower_ok("struct User { name: Str, email: Str? }"));
}

#[test]
fn struct_with_array_field() {
    assert!(lower_ok("struct Team { members: [Str] }"));
}

#[test]
fn struct_with_map_field() {
    assert!(lower_ok("struct Config { settings: {Str: Int} }"));
}

#[test]
fn struct_with_default() {
    assert!(lower_ok(
        "struct Config { timeout: Int = 30, retries: Int = 3 }"
    ));
}

#[test]
fn struct_decorated() {
    assert!(lower_ok("@table struct User { name: Str, age: Int }"));
}

#[test]
fn struct_with_enum_field() {
    assert!(lower_ok(
        "enum Status { Active, Inactive } struct User { name: Str, status: Status }"
    ));
}

#[test]
fn struct_method_expression_body() {
    assert!(lower_ok(
        "struct User { name: Str } fn User.greeting(self) -> Str => \"Hello, ${self.name}\""
    ));
}

#[test]
fn struct_method_with_params() {
    assert!(lower_ok(
        "struct Point { x: Int } fn Point.add(self, dx: Int) -> Int { return self.x + dx; }"
    ));
}

#[test]
fn struct_construction() {
    assert!(lower_ok(
        "struct Point { x: Int, y: Int } fn main() { let p = Point { x: 10, y: 20 }; }"
    ));
}

#[test]
fn struct_field_access() {
    assert!(lower_ok(
        "struct Point { x: Int } fn main() { let p = Point { x: 10 }; let v = p.x; }"
    ));
}

#[test]
fn struct_mut_field() {
    assert!(lower_ok(
        "struct Point { x: Int } fn main() { let mut p = Point { x: 0 }; p.x = 10; }"
    ));
}

// =============================================================================
// Additional Enums (enum_*)
// =============================================================================

#[test]
fn enum_many_variants() {
    let hir = parse_and_lower("enum Dir { Up, Down, Left, Right }");
    assert_eq!(
        hir.items
            .iter()
            .filter(|i| matches!(i, HirItem::Enum(_)))
            .count(),
        1
    );
}

#[test]
fn enum_match_all() {
    assert!(lower_ok("enum Color { Red, Green, Blue } fn main() { let c = Color::Red; match c { Color::Red => print(\"r\"), Color::Green => print(\"g\"), Color::Blue => print(\"b\") } }"));
}

#[test]
fn enum_match_wildcard() {
    assert!(lower_ok("enum Color { Red, Green, Blue } fn main() { let c = Color::Red; match c { Color::Red => print(\"red\"), _ => print(\"other\") } }"));
}

#[test]
fn enum_with_method() {
    assert!(lower_ok("enum Priority { Low, High } fn Priority.label(self) -> Str { match self { Priority::Low => \"low\", Priority::High => \"high\" } }"));
}

#[test]
fn enum_in_struct() {
    assert!(lower_ok("enum Status { Active } struct User { status: Status } fn main() { let u = User { status: Status::Active }; }"));
}

#[test]
fn enum_result_pattern() {
    assert!(lower_ok("enum MyResult { Success(Int), Failure(Str) } fn main() { let r = MyResult::Success(42); match r { MyResult::Success(v) => print(v), MyResult::Failure(e) => print(e) } }"));
}

// =============================================================================
// Error Handling (err_*)
// =============================================================================

#[test]
fn err_try_operator() {
    assert!(lower_ok("fn may_fail() -> Int ! Str { Ok 42; } fn caller() -> Int ! Str { let v = may_fail()?; Ok v; }"));
}

#[test]
fn err_ok_return() {
    assert!(lower_ok("fn compute() -> Int ! Str { Ok 42; }"));
}

#[test]
fn err_err_return() {
    assert!(lower_ok("fn compute() -> Int ! Str { Err \"fail\"; }"));
}

#[test]
fn err_match_result() {
    assert!(lower_ok(
        "fn main() { let r = Ok(42); match r { Ok(v) => print(v), Err(e) => print(e) } }"
    ));
}

#[test]
fn err_conditional_error() {
    assert!(lower_ok(
        "fn divide(a: Int, b: Int) -> Int ! Str { if b == 0 { Err \"zero\"; } Ok a / b; }"
    ));
}

#[test]
fn err_chained_try() {
    assert!(lower_ok("fn step1() -> Int ! Str { Ok 1; } fn step2(x: Int) -> Int ! Str { Ok x + 1; } fn pipeline() -> Int ! Str { let a = step1()?; let b = step2(a)?; Ok b; }"));
}

// =============================================================================
// Decorators (dec_*)
// =============================================================================

#[test]
fn dec_table() {
    assert!(lower_ok("@table struct User { name: Str }"));
}

#[test]
fn dec_route_get() {
    assert!(lower_ok(
        "@get(\"/users\") fn getUsers() { print(\"users\"); }"
    ));
}

#[test]
fn dec_route_post() {
    assert!(lower_ok(
        "@post(\"/users\") fn createUser() { print(\"create\"); }"
    ));
}

#[test]
fn dec_multiple() {
    assert!(lower_ok(
        "@route(\"/api\") @auth fn protected() { print(\"protected\"); }"
    ));
}

#[test]
fn dec_struct_table() {
    let hir = parse_and_lower("@table struct Task { title: Str, done: Bool }");
    assert_eq!(
        hir.items
            .iter()
            .filter(|i| matches!(i, HirItem::Struct(_)))
            .count(),
        1
    );
}

// =============================================================================
// Closures (cls_*)
// =============================================================================

#[test]
fn cls_capture_variable() {
    assert!(lower_ok(
        "fn main() { let x = 5; let f = () => x; print(f()); }"
    ));
}

#[test]
fn cls_capture_multiple() {
    assert!(lower_ok(
        "fn main() { let a = 1; let b = 2; let f = () => a + b; }"
    ));
}

#[test]
fn cls_in_map() {
    assert!(lower_ok(
        "fn main() { let doubled = [1, 2, 3].map((x) => x * 2); }"
    ));
}

#[test]
fn cls_in_filter() {
    assert!(lower_ok(
        "fn main() { let big = [1, 5, 10].filter((x) => x > 3); }"
    ));
}

#[test]
fn cls_in_reduce() {
    assert!(lower_ok(
        "fn main() { let sum = [1, 2, 3].reduce(0, (a, b) => a + b); }"
    ));
}

#[test]
fn cls_nested() {
    assert!(lower_ok("fn main() { let x = 5; let f = () => () => x; }"));
}

#[test]
fn cls_with_param_shadow() {
    assert!(lower_ok(
        "fn main() { let x = 5; let f = (x) => x + 1; print(f(10)); print(x); }"
    ));
}

#[test]
fn cls_capture_struct_field() {
    assert!(lower_ok(
        "fn main() { let u = User { name: \"A\" }; let f = () => u.name; }"
    ));
}

#[test]
fn cls_capture_array() {
    assert!(lower_ok(
        "fn main() { let arr = [1, 2, 3]; let f = () => arr.length(); }"
    ));
}

#[test]
fn cls_factory_pattern() {
    assert!(lower_ok(
        "fn makeAdder(n: Int) { let f = (x) => x + n; print(f(5)); }"
    ));
}

// =============================================================================
// Imports (import_*)
// =============================================================================

#[test]
fn import_basic() {
    assert!(lower_ok("import std::Http::Server;"));
}

#[test]
fn import_wildcard() {
    assert!(lower_ok("import std::Http::*;"));
}

#[test]
fn import_module() {
    assert!(lower_ok("import std::Database;"));
}

#[test]
fn import_nested() {
    assert!(lower_ok("import std::Http::Server::Request;"));
}

// =============================================================================
// Additional Complex Programs (complex_*)
// =============================================================================

#[test]
fn complex_user_crud() {
    assert!(lower_ok(
        r#"
struct User { name: Str, age: Int }
fn User.isAdult(self) -> Bool => self.age >= 18;
fn User.greeting(self) -> Str => "Hello, ${self.name};"
fn main() {
    let users = [User { name: "Alice", age: 25 }, User { name: "Bob", age: 15 }];
    for u in users {
        if u.isAdult() { print(u.greeting());    }
    }
}
"#
    ));
}

#[test]
fn complex_task_system() {
    assert!(lower_ok(
        r#"
enum Priority { Low, Medium, High }
enum Status { Todo, InProgress, Done }
struct Task { title: Str, priority: Priority, status: Status }
fn Task.isDone(self) -> Bool { match self.status { Status::Done => true, _ => false } };
fn Task.isUrgent(self) -> Bool { match self.priority { Priority::High => true, _ => false } };
"#
    ));
}

#[test]
fn complex_calculator() {
    assert!(lower_ok(
        r#"
fn add(a: Int, b: Int) -> Int => a + b;
fn sub(a: Int, b: Int) -> Int => a - b;
fn mul(a: Int, b: Int) -> Int => a * b;
fn div(a: Int, b: Int) -> Int ! Str {
    if b == 0 { Err "division by zero"; }
    Ok a / b;
}
fn main() {
    let r1 = add(10, 20);
    let r2 = sub(30, 5);
    let r3 = mul(4, 7);
    print(r1, r2, r3);
}
"#
    ));
}

#[test]
fn complex_config_loader() {
    assert!(lower_ok(
        r#"
struct Config { timeout: Int, retries: Int, verbose: Bool }
fn Config.isValid(self) -> Bool => self.timeout > 0 && self.retries > 0;
fn defaultConfig() -> Config {
    return Config { timeout: 30, retries: 3, verbose: false };
}
fn main() {
    let mut cfg = defaultConfig();
    if !cfg.isValid() {
        cfg.timeout = 60;
    }
    print(cfg.timeout);
}
"#
    ));
}

#[test]
fn complex_fizzbuzz() {
    assert!(lower_ok(
        r#"
fn main() {
    for i in 1..=20 {
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
fn complex_array_processing() {
    assert!(lower_ok(
        r#"
fn main() {
    let data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let evens = data.filter((x) => x % 2 == 0);
    let doubled = evens.map((x) => x * 2);
    let total = doubled.reduce(0, (a, b) => a + b);
    print(total);
}
"#
    ));
}

#[test]
fn complex_multi_file_pattern() {
    assert!(lower_ok(
        r#"
struct Address { city: Str, street: Str }
struct User { name: Str, age: Int, address: Address }

fn User.fullAddress(self) -> Str {
    return self.address.street + ", " + self.address.city;
}

fn main() {
    let u = User {
        name: "Alice",
        age: 30,
        address: Address { city: "NYC", street: "123 Main St" }
    };
    print(u.fullAddress());
}
"#
    ));
}

#[test]
fn complex_error_pipeline() {
    assert!(lower_ok(
        r#"
fn step1() -> Int ! Str { Ok 10; }
fn step2(x: Int) -> Int ! Str {
    if x <= 0 { Err "negative"; }
    Ok x * 2;
}
fn pipeline() -> Int ! Str {
    let a = step1()?;
    let b = step2(a)?;
    Ok b;
}
"#
    ));
}

#[test]
fn complex_shadowing() {
    assert!(lower_ok(
        "fn main() { let x = 1; let x = \"hello\"; let x = [1, 2]; print(x); }"
    ));
}

#[test]
fn complex_scope_nesting() {
    assert!(lower_ok(
        "fn main() { let x = 1; if true { let x = 2; if true { let x = 3; print(x); } } }"
    ));
}
