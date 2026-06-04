//! Borrow checking tests - verifies borrow safety

use doo_analysis::BorrowChecker;
use doo_core::types::TypeRegistry;
use doo_frontend::Parser;
use doo_hir::Lower;

fn borrow_check(src: &str) -> Result<(), Vec<String>> {
    let mut parser = Parser::new(src, 0);
    let program = parser
        .parse_program()
        .map_err(|e| vec![format!("parse: {:?}", e)])?;
    if parser.has_errors() {
        return Err(parser.errors().iter().map(|e| format!("{:?}", e)).collect());
    }

    let mut type_registry = TypeRegistry::new();
    let mut lowerer = Lower::new();
    let hir = lowerer.lower_program_typed(&program, &mut type_registry);

    let mut checker = BorrowChecker::new();
    checker
        .check(&hir)
        .map_err(|errors| errors.iter().map(|e| format!("{:?}", e)).collect())
}

fn borrows_ok(src: &str) -> bool {
    match borrow_check(src) {
        Ok(()) => true,
        Err(errs) => {
            eprintln!("[BORROWS_OK FAIL] src: {:?}", &src[..src.len().min(100)]);
            for e in &errs {
                eprintln!("  ERR: {}", e);
            }
            false
        }
    }
}

fn borrows_fail(src: &str) -> bool {
    borrow_check(src).is_err()
}

// ===========================================================================
// 1. Simple variable reads (20 tests)
// ===========================================================================

#[test]
fn read_int_variable() {
    assert!(borrows_ok("let x = 5;\nprint(x);"));
}

#[test]
fn read_str_variable() {
    assert!(borrows_ok("let s = \"hello\";\nprint(s);"));
}

#[test]
fn read_bool_variable() {
    assert!(borrows_ok("let b = true;\nprint(b);"));
}

#[test]
fn read_float_variable() {
    assert!(borrows_ok("let f = 3.14;\nprint(f);"));
}

#[test]
fn read_variable_in_expression() {
    assert!(borrows_ok("let x = 5;\nlet y = x + 1;"));
}

#[test]
fn read_variable_in_comparison() {
    assert!(borrows_ok("let x = 5;\nif x > 3 { print(x); }"));
}

#[test]
fn read_variable_twice() {
    assert!(borrows_ok("let x = 5;\nprint(x);\nprint(x);"));
}

#[test]
fn read_two_variables() {
    assert!(borrows_ok("let x = 5;\nlet y = 10;\nlet z = x + y;"));
}

#[test]
fn read_variable_in_return() {
    assert!(borrows_ok("fn foo() -> Int { let x = 42;\nreturn x; }"));
}

#[test]
fn read_variable_in_string_interpolation() {
    assert!(borrows_ok(
        "let name = \"world\";\nlet msg = \"hello ${name}\";"
    ));
}

#[test]
fn read_nil_variable() {
    assert!(borrows_ok("let x: Int? = nil;\nprint(x);"));
}

#[test]
fn read_variable_in_ternary() {
    assert!(borrows_ok(
        "let x = 5;\nlet y = if x > 3 { \"big\" } else { \"small\" };"
    ));
}

#[test]
fn read_variable_in_array_literal() {
    assert!(borrows_ok("let x = 1;\nlet arr = [x, x, x];"));
}

#[test]
fn read_variable_in_map_literal() {
    assert!(borrows_ok("let v = 42;\nlet m = {\"key\": v};"));
}

#[test]
fn read_variable_in_struct_literal() {
    assert!(borrows_ok(
        "let name = \"John\";\nlet u = User { name: name };"
    ));
}

#[test]
fn read_variable_multiple_expressions() {
    assert!(borrows_ok(
        "let x = 5;\nlet a = x + 1;\nlet b = x * 2;\nlet c = x - 3;"
    ));
}

#[test]
fn read_variable_in_logical_expr() {
    assert!(borrows_ok("let a = true;\nlet b = false;\nlet c = a && b;"));
}

#[test]
fn read_variable_in_null_coalesce() {
    assert!(borrows_ok(
        "let x: Int? = nil;\nlet y = x ?? panic(\"nil\");"
    ));
}

#[test]
fn read_variable_in_cast() {
    assert!(borrows_ok("let x = 42;\nlet f = x as Float;"));
}

#[test]
fn read_variable_in_range() {
    assert!(borrows_ok("let n = 10;\nfor i in 0..n { print(i); }"));
}

// ===========================================================================
// 2. Mutable variables (20 tests)
// ===========================================================================

#[test]
fn mut_simple_reassign() {
    assert!(borrows_ok("let mut x = 5;\nx = 10;"));
}

#[test]
fn mut_reassign_str() {
    assert!(borrows_ok("let mut s = \"hello\";\ns = \"world\";"));
}

#[test]
fn mut_reassign_bool() {
    assert!(borrows_ok("let mut b = true;\nb = false;"));
}

#[test]
fn mut_reassign_float() {
    assert!(borrows_ok("let mut f = 1.0;\nf = 2.5;"));
}

#[test]
fn mut_compound_add() {
    assert!(borrows_ok("let mut x = 5;\nx += 3;"));
}

#[test]
fn mut_compound_sub() {
    assert!(borrows_ok("let mut x = 10;\nx -= 4;"));
}

#[test]
fn mut_compound_mul() {
    assert!(borrows_ok("let mut x = 3;\nx *= 2;"));
}

#[test]
fn mut_compound_div() {
    assert!(borrows_ok("let mut x = 10;\nx /= 2;"));
}

#[test]
fn mut_compound_mod() {
    assert!(borrows_ok("let mut x = 10;\nx %= 3;"));
}

#[test]
fn mut_increment() {
    assert!(borrows_ok("let mut x = 0;\nx++;"));
}

#[test]
fn mut_decrement() {
    assert!(borrows_ok("let mut x = 10;\nx--;"));
}

#[test]
fn mut_reassign_multiple_times() {
    assert!(borrows_ok("let mut x = 0;\nx = 1;\nx = 2;\nx = 3;"));
}

#[test]
fn mut_reassign_with_expression() {
    assert!(borrows_ok("let mut x = 5;\nx = x + 1;"));
}

#[test]
fn mut_reassign_with_function_call() {
    assert!(borrows_ok("let mut x = 0;\nx = getValue();"));
}

#[test]
fn mut_reassign_in_if() {
    assert!(borrows_ok("let mut x = 0;\nif true { x = 1; }"));
}

#[test]
fn mut_reassign_in_for() {
    assert!(borrows_ok("let mut sum = 0;\nfor i in 0..10 { sum += i; }"));
}

#[test]
fn mut_array_push() {
    assert!(borrows_ok("let mut arr: [Int] = [];\narr.push(1);"));
}

#[test]
fn mut_read_after_write() {
    assert!(borrows_ok("let mut x = 0;\nx = 42;\nprint(x);"));
}

#[test]
fn mut_typed_reassign() {
    assert!(borrows_ok("let mut x: Int = 0;\nx = 100;"));
}

#[test]
fn mut_reassign_nil_optional() {
    assert!(borrows_ok("let mut x: Int? = 5;\nx = nil;"));
}

// ===========================================================================
// 3. Auto-clone patterns (20 tests)
// ===========================================================================

#[test]
fn clone_variable_used_twice_in_call() {
    assert!(borrows_ok("let x = 5;\nprint(x);\nprint(x);"));
}

#[test]
fn clone_variable_in_two_expressions() {
    assert!(borrows_ok(
        "let s = \"hello\";\nlet a = s + \" world\";\nlet b = s + \" there\";"
    ));
}

#[test]
fn clone_variable_passed_to_two_functions() {
    assert!(borrows_ok(
        "fn foo(x: Int) { print(x); }\nfn bar(x: Int) { print(x); }\nlet v = 10;\nfoo(v);\nbar(v);"
    ));
}

#[test]
fn clone_array_used_multiple_times() {
    assert!(borrows_ok(
        "let arr = [1, 2, 3];\nlet a = arr.length();\nlet b = arr.map((x) => x * 2);"
    ));
}

#[test]
fn clone_struct_used_in_multiple_accesses() {
    assert!(borrows_ok(
        "let u = User { name: \"John\", age: 30 };\nprint(u.name);\nprint(u.age);"
    ));
}

#[test]
fn clone_string_used_in_interpolation_and_call() {
    assert!(borrows_ok(
        "let name = \"Alice\";\nlet msg = \"Hello ${name}\";\nprint(name);"
    ));
}

#[test]
fn clone_variable_in_array_and_map() {
    assert!(borrows_ok(
        "let x = 42;\nlet arr = [x, x];\nlet m = {\"val\": x};"
    ));
}

#[test]
fn clone_variable_in_binary_both_sides() {
    assert!(borrows_ok("let x = 5;\nlet y = x + x;"));
}

#[test]
fn clone_variable_in_nested_calls() {
    assert!(borrows_ok("let x = 10;\nlet y = add(x, mul(x, x));"));
}

#[test]
fn clone_variable_in_closure_and_outer() {
    assert!(borrows_ok("let x = 5;\nlet f = (y) => y + x;\nprint(x);"));
}

#[test]
fn clone_map_used_in_loop_and_access() {
    assert!(borrows_ok(
        "let m = {\"a\": 1};\nlet v = m[\"a\"];\nprint(m);"
    ));
}

#[test]
fn clone_array_in_for_and_length() {
    assert!(borrows_ok(
        "let arr = [1, 2, 3];\nfor item in arr { print(item); }\nprint(arr.length());"
    ));
}

#[test]
fn clone_bool_used_in_multiple_conditions() {
    assert!(borrows_ok(
        "let flag = true;\nif flag { print(1); }\nif flag { print(2); }"
    ));
}

#[test]
fn clone_variable_in_ternary_and_print() {
    assert!(borrows_ok(
        "let x = 5;\nlet r = if x > 3 { x } else { 0 };\nprint(x);"
    ));
}

#[test]
fn clone_struct_passed_to_multiple_fns() {
    assert!(borrows_ok("fn show(u: User) { print(u); }\nfn save(u: User) { print(u); }\nlet u = User { name: \"A\" };\nshow(u);\nsave(u);"));
}

#[test]
fn clone_variable_in_match_and_after() {
    assert!(borrows_ok(
        "let x = 42;\nlet r = match { x > 0 => \"pos\", _ => \"neg\" };\nprint(x);"
    ));
}

#[test]
fn clone_float_in_arithmetic_chain() {
    assert!(borrows_ok(
        "let f = 3.14;\nlet a = f * 2.0;\nlet b = f + 1.0;\nlet c = f / 2.0;"
    ));
}

#[test]
fn clone_array_used_in_spread_and_access() {
    assert!(borrows_ok("let a = [1, 2];\nlet b = [...a, 3];\nprint(a);"));
}

#[test]
fn clone_variable_in_if_else_both_branches() {
    assert!(borrows_ok(
        "let x = 5;\nif true { print(x); } else { print(x); }"
    ));
}

#[test]
fn clone_string_concat_multiple_uses() {
    assert!(borrows_ok(
        "let s = \"base\";\nlet a = s + \"_a\";\nlet b = s + \"_b\";\nlet c = s + \"_c\";"
    ));
}

// ===========================================================================
// 4. Function argument passing (20 tests)
// ===========================================================================

#[test]
fn arg_pass_int_literal() {
    assert!(borrows_ok("fn foo(x: Int) { print(x); }\nfoo(42);"));
}

#[test]
fn arg_pass_str_literal() {
    assert!(borrows_ok("fn foo(s: Str) { print(s); }\nfoo(\"hello\");"));
}

#[test]
fn arg_pass_variable() {
    assert!(borrows_ok(
        "fn foo(x: Int) { print(x); }\nlet v = 10;\nfoo(v);"
    ));
}

#[test]
fn arg_pass_expression() {
    assert!(borrows_ok("fn foo(x: Int) { print(x); }\nfoo(1 + 2);"));
}

#[test]
fn arg_pass_struct_field() {
    assert!(borrows_ok(
        "let u = User { name: \"A\", age: 30 };\nprint(u.name);"
    ));
}

#[test]
fn arg_pass_array_element() {
    assert!(borrows_ok("let arr = [1, 2, 3];\nprint(arr[0]);"));
}

#[test]
fn arg_pass_method_result() {
    assert!(borrows_ok("let s = \"hello\";\nprint(s.length());"));
}

#[test]
fn arg_pass_nested_field() {
    assert!(borrows_ok(
        "let u = User { name: \"A\", address: Address { city: \"NYC\" } };\nprint(u.address.city);"
    ));
}

#[test]
fn arg_pass_multiple_args() {
    assert!(borrows_ok(
        "fn add(a: Int, b: Int) -> Int { return a + b; }\nadd(1, 2);"
    ));
}

#[test]
fn arg_pass_mixed_types() {
    assert!(borrows_ok(
        "fn foo(a: Int, b: Str, c: Bool) { print(a); }\nfoo(1, \"hi\", true);"
    ));
}

#[test]
fn arg_pass_array_to_function() {
    assert!(borrows_ok(
        "fn sum(nums: [Int]) -> Int { return 0; }\nsum([1, 2, 3]);"
    ));
}

#[test]
fn arg_pass_map_to_function() {
    assert!(borrows_ok(
        "fn process(m: {Str: Int}) { print(m); }\nprocess({\"a\": 1});"
    ));
}

#[test]
fn arg_pass_struct_to_function() {
    assert!(borrows_ok(
        "fn show(u: User) { print(u); }\nlet u = User { name: \"A\" };\nshow(u);"
    ));
}

#[test]
fn arg_pass_closure_to_method() {
    assert!(borrows_ok("let arr = [1, 2, 3];\narr.map((x) => x * 2);"));
}

#[test]
fn arg_pass_function_result() {
    assert!(borrows_ok(
        "fn getVal() -> Int { return 5; }\nprint(getVal());"
    ));
}

#[test]
fn arg_pass_nested_function_call() {
    assert!(borrows_ok(
        "fn outer(x: Int) -> Int { return x; }\nfn inner() -> Int { return 5; }\nouter(inner());"
    ));
}

#[test]
fn arg_pass_chained_method_result() {
    assert!(borrows_ok(
        "let s = \"hello world\";\nprint(s.split(\" \").length());"
    ));
}

#[test]
fn arg_pass_ternary_result() {
    assert!(borrows_ok(
        "fn foo(x: Int) { print(x); }\nlet b = true;\nfoo(if b { 1 } else { 2 });"
    ));
}

#[test]
fn arg_pass_cast_result() {
    assert!(borrows_ok(
        "fn foo(f: Float) { print(f); }\nlet x = 42;\nfoo(x as Float);"
    ));
}

#[test]
fn arg_pass_null_coalesce_result() {
    assert!(borrows_ok(
        "fn foo(x: Int) { print(x); }\nlet v: Int? = nil;\nfoo(v ?? panic(\"nil\"));"
    ));
}

// ===========================================================================
// 5. Loop variable access (20 tests)
// ===========================================================================

#[test]
fn loop_for_in_array() {
    assert!(borrows_ok("for item in [1, 2, 3] { print(item); }"));
}

#[test]
fn loop_for_in_range() {
    assert!(borrows_ok("for i in 0..10 { print(i); }"));
}

#[test]
fn loop_for_in_inclusive_range() {
    assert!(borrows_ok("for i in 0..=10 { print(i); }"));
}

#[test]
fn loop_for_with_index() {
    assert!(borrows_ok(
        "let arr = [10, 20, 30];\nfor i, val in arr { print(i, val); }"
    ));
}

#[test]
fn loop_for_variable_range_bounds() {
    assert!(borrows_ok(
        "let start = 0;\nlet end = 10;\nfor i in start..end { print(i); }"
    ));
}

#[test]
fn loop_for_nested() {
    assert!(borrows_ok(
        "for i in 0..5 { for j in 0..5 { print(i, j); } }"
    ));
}

#[test]
fn loop_for_with_break() {
    assert!(borrows_ok("for i in 0..100 { if i > 10 { break; } }"));
}

#[test]
fn loop_for_with_continue() {
    assert!(borrows_ok(
        "for i in 0..10 { if i % 2 == 0 { continue; }\nprint(i); }"
    ));
}

#[test]
fn loop_for_accumulator() {
    assert!(borrows_ok(
        "let mut sum = 0;\nfor i in 0..10 { sum += i; }\nprint(sum);"
    ));
}

#[test]
fn loop_for_with_condition_on_var() {
    assert!(borrows_ok(
        "for item in [1, 2, 3, 4, 5] { if item > 3 { print(item); } }"
    ));
}

#[test]
fn loop_for_accessing_outer_variable() {
    assert!(borrows_ok(
        "let multiplier = 2;\nfor i in 0..5 { print(i * multiplier); }"
    ));
}

#[test]
fn loop_for_building_array() {
    assert!(borrows_ok(
        "let mut result: [Int] = [];\nfor i in 0..5 { result.push(i); }"
    ));
}

#[test]
fn loop_for_with_string_array() {
    assert!(borrows_ok(
        "let names = [\"Alice\", \"Bob\"];\nfor name in names { print(name); }"
    ));
}

#[test]
fn loop_for_with_method_call_on_var() {
    assert!(borrows_ok(
        "let words = [\"hello\", \"world\"];\nfor w in words { print(w.length()); }"
    ));
}

#[test]
fn loop_infinite_for() {
    assert!(borrows_ok("for { print(\"loop\");\nbreak; }"));
}

#[test]
fn loop_for_with_struct_array() {
    assert!(borrows_ok("for user in users { print(user.name); }"));
}

#[test]
fn loop_for_with_map_access_in_body() {
    assert!(borrows_ok(
        "let m = {\"a\": 1};\nfor i in 0..3 { print(m[\"a\"]); }"
    ));
}

#[test]
fn loop_for_nested_with_outer_arr() {
    assert!(borrows_ok(
        "let arr = [1, 2, 3];\nfor i in 0..3 { for item in arr { print(i, item); } }"
    ));
}

#[test]
fn loop_for_with_index_and_condition() {
    assert!(borrows_ok(
        "let arr = [10, 20, 30, 40];\nfor i, val in arr { if i > 1 { print(val); } }"
    ));
}

#[test]
fn loop_for_complex_body() {
    assert!(borrows_ok(
        "let mut total = 0;\nfor i in 1..=100 { if i % 3 == 0 { total += i; } }\nprint(total);"
    ));
}

// ===========================================================================
// 6. Scope-based borrowing (20 tests)
// ===========================================================================

#[test]
fn scope_variable_in_if_block() {
    assert!(borrows_ok(
        "let x = 5;\nif x > 0 { let y = x + 1;\nprint(y); }"
    ));
}

#[test]
fn scope_variable_in_if_else() {
    assert!(borrows_ok(
        "let x = 5;\nif x > 0 { print(x); } else { print(x); }"
    ));
}

#[test]
fn scope_variable_in_nested_if() {
    assert!(borrows_ok(
        "let x = 5;\nif x > 0 { if x < 10 { print(x); } }"
    ));
}

#[test]
fn scope_variable_in_for_loop_body() {
    assert!(borrows_ok("let x = 42;\nfor i in 0..3 { print(x); }"));
}

#[test]
fn scope_outer_variable_after_block() {
    assert!(borrows_ok("let x = 5;\nif true { let y = x; }\nprint(x);"));
}

#[test]
fn scope_nested_function_reads_outer() {
    assert!(borrows_ok(
        "fn outer() -> Int { let x = 5;\nreturn x + 1; }\nfn inner() -> Int { return 1; }"
    ));
}

#[test]
fn scope_variable_in_match() {
    assert!(borrows_ok(
        "let x = 42;\nlet r = match { x > 0 => \"pos\", _ => \"neg\" };"
    ));
}

#[test]
fn scope_variable_across_if_else_if() {
    assert!(borrows_ok("let x = 5;\nif x > 10 { print(\"big\"); } else if x > 0 { print(\"small\"); } else { print(\"neg\"); }"));
}

#[test]
fn scope_deeply_nested_blocks() {
    assert!(borrows_ok(
        "let x = 1;\nif true { if true { if true { print(x); } } }"
    ));
}

#[test]
fn scope_variable_in_for_then_after() {
    assert!(borrows_ok(
        "let mut x = 0;\nfor i in 0..10 { x += i; }\nprint(x);"
    ));
}

#[test]
fn scope_multiple_variables_same_block() {
    assert!(borrows_ok(
        "if true { let a = 1;\nlet b = 2;\nlet c = a + b;\nprint(c); }"
    ));
}

#[test]
fn scope_shadow_variable_in_block() {
    assert!(borrows_ok(
        "let x = 5;\nif true { let x = 10;\nprint(x); }\nprint(x);"
    ));
}

#[test]
fn scope_variable_in_closure_scope() {
    assert!(borrows_ok("let x = 5;\nlet f = () => x + 1;\nprint(f());"));
}

#[test]
fn scope_function_local_variable() {
    assert!(borrows_ok("fn foo() -> Int { let x = 42;\nreturn x; }"));
}

#[test]
fn scope_multiple_functions_local_vars() {
    assert!(borrows_ok(
        "fn foo() -> Int { let x = 1;\nreturn x; }\nfn bar() -> Int { let x = 2;\nreturn x; }"
    ));
}

#[test]
fn scope_variable_in_inline_if_else() {
    assert!(borrows_ok(
        "let x = 5;\nlet y = if x > 0 { x; } else { 0 };"
    ));
}

#[test]
fn scope_variable_in_inline_match() {
    assert!(borrows_ok(
        "let x = 5;\nlet y = match { x > 0 => x, _ => 0 };"
    ));
}

#[test]
fn scope_mut_variable_across_scopes() {
    assert!(borrows_ok(
        "let mut x = 0;\nif true { x = 1; }\nif true { x = 2; }\nprint(x);"
    ));
}

#[test]
fn scope_nested_for_with_scoped_vars() {
    assert!(borrows_ok(
        "for i in 0..3 { let local = i * 2;\nprint(local); }"
    ));
}

#[test]
fn scope_variable_in_struct_method() {
    assert!(borrows_ok(
        "fn User.greet(self) -> Str { let msg = \"Hello \" + self.name;\nreturn msg; }"
    ));
}

// ===========================================================================
// 7. Struct field access (20 tests)
// ===========================================================================

#[test]
fn struct_read_single_field() {
    assert!(borrows_ok(
        "let u = User { name: \"John\" };\nprint(u.name);"
    ));
}

#[test]
fn struct_read_multiple_fields() {
    assert!(borrows_ok(
        "let u = User { name: \"John\", age: 30 };\nprint(u.name);\nprint(u.age);"
    ));
}

#[test]
fn struct_read_nested_field() {
    assert!(borrows_ok(
        "let u = User { name: \"A\", address: Address { city: \"NYC\" } };\nprint(u.address.city);"
    ));
}

#[test]
fn struct_write_field() {
    assert!(borrows_ok(
        "let mut u = User { name: \"John\" };\nu.name = \"Jane\";"
    ));
}

#[test]
fn struct_write_nested_field() {
    assert!(borrows_ok("let mut u = User { name: \"A\", address: Address { city: \"NYC\" } };\nu.address.city = \"LA\";"));
}

#[test]
fn struct_field_in_expression() {
    assert!(borrows_ok(
        "let p = Point { x: 10, y: 20 };\nlet sum = p.x + p.y;"
    ));
}

#[test]
fn struct_field_in_comparison() {
    assert!(borrows_ok(
        "let u = User { name: \"A\", age: 30 };\nif u.age > 18 { print(\"adult\"); }"
    ));
}

#[test]
fn struct_field_as_function_arg() {
    assert!(borrows_ok(
        "fn show(s: Str) { print(s); }\nlet u = User { name: \"A\" };\nshow(u.name);"
    ));
}

#[test]
fn struct_field_in_string_interpolation() {
    assert!(borrows_ok(
        "let u = User { name: \"Alice\" };\nlet msg = \"Hello ${u.name}\";"
    ));
}

#[test]
fn struct_field_method_call() {
    assert!(borrows_ok(
        "let u = User { name: \"Alice\" };\nlet len = u.name.length();"
    ));
}

#[test]
fn struct_field_in_array() {
    assert!(borrows_ok(
        "let a = Point { x: 1 };\nlet b = Point { x: 2 };\nlet arr = [a.x, b.x];"
    ));
}

#[test]
fn struct_field_in_map_value() {
    assert!(borrows_ok(
        "let u = User { name: \"A\", age: 30 };\nlet m = {\"age\": u.age};"
    ));
}

#[test]
fn struct_field_compound_assign() {
    assert!(borrows_ok("let mut p = Point { x: 0, y: 0 };\np.x += 5;"));
}

#[test]
fn struct_field_in_loop_condition() {
    assert!(borrows_ok("for i in 0..p.x { print(i); }"));
}

#[test]
fn struct_field_in_ternary() {
    assert!(borrows_ok(
        "let u = User { age: 25 };\nlet s = if u.age >= 18 { \"adult\" } else { \"minor\" };"
    ));
}

#[test]
fn struct_field_chained_access() {
    assert!(borrows_ok("let x = a.b.c.d;"));
}

#[test]
fn struct_field_in_match() {
    assert!(borrows_ok(
        "let u = User { age: 25 };\nmatch { u.age > 18 => print(\"adult\"), _ => print(\"minor\") }"
    ));
}

#[test]
fn struct_field_in_return() {
    assert!(borrows_ok("fn getName(u: User) -> Str { return u.name; }"));
}

#[test]
fn struct_field_in_closure() {
    assert!(borrows_ok(
        "let users = [User { name: \"A\" }];\nlet names = users.map((u) => u.name);"
    ));
}

#[test]
fn struct_self_field_access() {
    assert!(borrows_ok(
        "fn User.getName(self) -> Str { return self.name; }"
    ));
}

// ===========================================================================
// 8. Collection element access (20 tests)
// ===========================================================================

#[test]
fn coll_array_index_literal() {
    assert!(borrows_ok("let arr = [1, 2, 3];\nlet x = arr[0];"));
}

#[test]
fn coll_array_index_variable() {
    assert!(borrows_ok(
        "let arr = [1, 2, 3];\nlet i = 1;\nlet x = arr[i];"
    ));
}

#[test]
fn coll_array_index_expression() {
    assert!(borrows_ok("let arr = [1, 2, 3];\nlet x = arr[1 + 1];"));
}

#[test]
fn coll_array_nested_index() {
    assert!(borrows_ok(
        "let matrix = [[1, 2], [3, 4]];\nlet x = matrix[0][1];"
    ));
}

#[test]
fn coll_array_index_assign() {
    assert!(borrows_ok("let mut arr = [1, 2, 3];\narr[0] = 10;"));
}

#[test]
fn coll_map_string_key() {
    assert!(borrows_ok(
        "let m = {\"a\": 1, \"b\": 2};\nlet x = m[\"a\"];"
    ));
}

#[test]
fn coll_map_int_key() {
    assert!(borrows_ok(
        "let m = {1: \"one\", 2: \"two\"};\nlet x = m[1];"
    ));
}

#[test]
fn coll_map_variable_key() {
    assert!(borrows_ok(
        "let m = {\"a\": 1};\nlet key = \"a\";\nlet x = m[key];"
    ));
}

#[test]
fn coll_map_assign_value() {
    assert!(borrows_ok("let mut m = {\"a\": 1};\nm[\"b\"] = 2;"));
}

#[test]
fn coll_array_in_expression() {
    assert!(borrows_ok(
        "let arr = [10, 20, 30];\nlet x = arr[0] + arr[1];"
    ));
}

#[test]
fn coll_array_index_in_condition() {
    assert!(borrows_ok(
        "let arr = [1, 2, 3];\nif arr[0] > 0 { print(\"positive\"); }"
    ));
}

#[test]
fn coll_array_index_as_arg() {
    assert!(borrows_ok("let arr = [1, 2, 3];\nprint(arr[2]);"));
}

#[test]
fn coll_map_access_in_condition() {
    assert!(borrows_ok(
        "let m = {\"key\": 42};\nif m[\"key\"] > 0 { print(\"yes\"); }"
    ));
}

#[test]
fn coll_array_method_on_element() {
    assert!(borrows_ok(
        "let arr = [\"hello\", \"world\"];\nlet len = arr[0].length();"
    ));
}

#[test]
fn coll_array_index_in_loop() {
    assert!(borrows_ok(
        "let arr = [10, 20, 30];\nfor i in 0..3 { print(arr[i]); }"
    ));
}

#[test]
fn coll_map_in_check() {
    assert!(borrows_ok(
        "let m = {\"a\": 1};\nif \"a\" in m { print(\"found\"); }"
    ));
}

#[test]
fn coll_array_in_check() {
    assert!(borrows_ok(
        "let arr = [1, 2, 3];\nif 2 in arr { print(\"found\"); }"
    ));
}

#[test]
fn coll_map_access_nested() {
    assert!(borrows_ok(
        "let m = {\"a\": {\"b\": 1}};\nlet x = m[\"a\"][\"b\"];"
    ));
}

#[test]
fn coll_array_spread_from_index() {
    assert!(borrows_ok("let a = [1, 2];\nlet b = [0, ...a];"));
}

#[test]
fn coll_map_keys_method() {
    assert!(borrows_ok(
        "let m = {\"a\": 1, \"b\": 2};\nlet k = m.keys();"
    ));
}

// ===========================================================================
// 9. Closure captures (20 tests)
// ===========================================================================

#[test]
fn closure_capture_int() {
    assert!(borrows_ok("let x = 5;\nlet f = () => x;"));
}

#[test]
fn closure_capture_str() {
    assert!(borrows_ok("let s = \"hello\";\nlet f = () => s;"));
}

#[test]
fn closure_capture_in_expression() {
    assert!(borrows_ok("let offset = 10;\nlet f = (x) => x + offset;"));
}

#[test]
fn closure_capture_multiple_vars() {
    assert!(borrows_ok("let a = 1;\nlet b = 2;\nlet f = () => a + b;"));
}

#[test]
fn closure_capture_in_map_callback() {
    assert!(borrows_ok(
        "let factor = 3;\nlet result = [1, 2, 3].map((x) => x * factor);"
    ));
}

#[test]
fn closure_capture_in_filter_callback() {
    assert!(borrows_ok(
        "let threshold = 5;\nlet result = [1, 2, 10].filter((x) => x > threshold);"
    ));
}

#[test]
fn closure_capture_in_reduce_callback() {
    assert!(borrows_ok(
        "let base = 100;\nlet result = [1, 2, 3].reduce(base, (a, b) => a + b);"
    ));
}

#[test]
fn closure_capture_struct() {
    assert!(borrows_ok(
        "let u = User { name: \"A\" };\nlet f = () => u.name;"
    ));
}

#[test]
fn closure_capture_array() {
    assert!(borrows_ok(
        "let arr = [1, 2, 3];\nlet f = () => arr.length();"
    ));
}

#[test]
fn closure_capture_bool() {
    assert!(borrows_ok("let flag = true;\nlet f = () => !flag;"));
}

#[test]
fn closure_capture_in_nested_closure() {
    assert!(borrows_ok("let x = 5;\nlet f = () => () => x;"));
}

#[test]
fn closure_capture_after_use() {
    assert!(borrows_ok("let x = 5;\nprint(x);\nlet f = () => x;"));
}

#[test]
fn closure_capture_used_after_closure() {
    assert!(borrows_ok("let x = 5;\nlet f = () => x;\nprint(x);"));
}

#[test]
fn closure_capture_in_chained_methods() {
    assert!(borrows_ok(
        "let min = 0;\nlet max = 100;\nlet result = [1, 50, 200].filter((x) => x >= min && x <= max);"
    ));
}

#[test]
fn closure_capture_map_variable() {
    assert!(borrows_ok("let m = {\"a\": 1};\nlet f = () => m[\"a\"];"));
}

#[test]
fn closure_capture_loop_variable_pattern() {
    assert!(borrows_ok(
        "let arr = [1, 2, 3];\nlet fns = arr.map((x) => () => x);"
    ));
}

#[test]
fn closure_capture_outer_parameter() {
    assert!(borrows_ok(
        "fn makeAdder(n: Int) { let f = (x) => x + n;\nprint(f(5)); }"
    ));
}

#[test]
fn closure_capture_in_route_handler() {
    assert!(borrows_ok(
        "let users = [User { name: \"A\" }];\napp.get(\"/users\", (req) => users);"
    ));
}

#[test]
fn closure_capture_complex_expression() {
    assert!(borrows_ok(
        "let base = 10;\nlet scale = 2.0;\nlet f = (x) => (x + base) as Float * scale;"
    ));
}

#[test]
fn closure_capture_with_shadowing() {
    assert!(borrows_ok(
        "let x = 5;\nlet f = (x) => x + 1;\nprint(f(10));\nprint(x);"
    ));
}

// ===========================================================================
// 10. Complex patterns (20 tests)
// ===========================================================================

#[test]
fn complex_multi_variable_interaction() {
    assert!(borrows_ok(
        "let a = 1;\nlet b = 2;\nlet c = a + b;\nlet d = c * a;\nprint(a, b, c, d);"
    ));
}

#[test]
fn complex_conditional_borrow() {
    assert!(borrows_ok(
        "let x = 5;\nlet y = if x > 0 { x + 1 } else { x - 1 };\nprint(x, y);"
    ));
}

#[test]
fn complex_loop_with_closure_capture() {
    assert!(borrows_ok("let mut total = 0;\nlet multiplier = 2;\nfor i in 0..10 { total += i * multiplier; }\nprint(total);"));
}

#[test]
fn complex_struct_build_and_pass() {
    assert!(borrows_ok("let name = \"Alice\";\nlet age = 30;\nlet u = User { name: name, age: age };\nprint(u.name, u.age);"));
}

#[test]
fn complex_array_transform_pipeline() {
    assert!(borrows_ok("let data = [1, 2, 3, 4, 5];\nlet result = data.filter((x) => x > 2).map((x) => x * 2);\nprint(result);"));
}

#[test]
fn complex_multiple_scopes_same_var() {
    assert!(borrows_ok(
        "let x = 5;\nif x > 0 { print(x); }\nfor i in 0..x { print(i); }\nprint(x);"
    ));
}

#[test]
fn complex_mut_in_loop_with_read_after() {
    assert!(borrows_ok(
        "let mut acc = \"\";\nfor name in [\"A\", \"B\", \"C\"] { acc = acc + name; }\nprint(acc);"
    ));
}

#[test]
fn complex_nested_struct_creation() {
    assert!(borrows_ok("let city = \"NYC\";\nlet addr = Address { city: city, street: \"Main\" };\nlet u = User { name: \"A\", address: addr };\nprint(u.address.city);"));
}

#[test]
fn complex_variable_in_match_arms() {
    assert!(borrows_ok("let x = 42;\nlet result = match { x > 100 => \"big\", x > 10 => \"medium\", _ => \"small\" };\nprint(result, x);"));
}

#[test]
fn complex_chained_method_with_captures() {
    assert!(borrows_ok("let min = 2;\nlet scale = 10;\nlet result = [1, 2, 3, 4, 5].filter((x) => x > min).map((x) => x * scale);\nprint(result);"));
}

#[test]
fn complex_return_from_conditional() {
    assert!(borrows_ok("fn classify(x: Int) -> Str { if x > 0 { return \"positive\"; } else if x < 0 { return \"negative\"; } else { return \"zero\"; } }"));
}

#[test]
fn complex_multiple_closures_same_capture() {
    assert!(borrows_ok(
        "let x = 5;\nlet add = (y) => y + x;\nlet mul = (y) => y * x;\nprint(add(1), mul(2));"
    ));
}

#[test]
fn complex_array_of_structs_iteration() {
    assert!(borrows_ok("let users = [User { name: \"A\" }, User { name: \"B\" }];\nfor u in users { print(u.name); }"));
}

#[test]
fn complex_nested_loop_with_outer_vars() {
    assert!(borrows_ok(
        "let rows = [1, 2];\nlet cols = [3, 4];\nfor r in rows { for c in cols { print(r, c); } }"
    ));
}

#[test]
fn complex_conditional_mutation() {
    assert!(borrows_ok("let mut x = 0;\nif true { x = 1; } else { x = 2; }\nlet mut y = 0;\nif x == 1 { y = 10; }\nprint(x, y);"));
}

#[test]
fn complex_struct_field_chain_ops() {
    assert!(borrows_ok("let p1 = Point { x: 1, y: 2 };\nlet p2 = Point { x: 3, y: 4 };\nlet sum_x = p1.x + p2.x;\nlet sum_y = p1.y + p2.y;\nprint(sum_x, sum_y);"));
}

#[test]
fn complex_map_iteration_with_condition() {
    assert!(borrows_ok(
        "let m = {\"a\": 1, \"b\": 2};\nif \"a\" in m { let v = m[\"a\"];\nprint(v); }"
    ));
}

#[test]
fn complex_error_handling_pattern() {
    assert!(borrows_ok(
        "let result = 42;\nlet value = if result > 0 { result } else { 0 };"
    ));
}

#[test]
fn complex_multi_return_with_borrows() {
    assert!(borrows_ok(
        "fn process(data: [Int]) -> Int {\nlet min = data[0];\nreturn min;\n}"
    ));
}

#[test]
fn complex_builder_pattern() {
    assert!(borrows_ok("let mut config = Config { timeout: 30, retries: 3 };\nconfig.timeout = 60;\nconfig.retries = 5;\nprint(config.timeout, config.retries);"));
}
