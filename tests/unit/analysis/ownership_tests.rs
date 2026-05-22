//! Ownership analysis tests - verifies move/copy semantics

use doo_analysis::OwnershipAnalyzer;
use doo_core::types::TypeRegistry;
use doo_frontend::Parser;
use doo_hir::Lower;

fn ownership_check(src: &str) -> Result<(), Vec<String>> {
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

    let mut analyzer = OwnershipAnalyzer::new();
    analyzer
        .analyze(&hir)
        .map(|_results| ())
        .map_err(|errors| errors.iter().map(|e| format!("{:?}", e)).collect())
}

fn ownership_ok(src: &str) -> bool {
    ownership_check(src).is_ok()
}

// ===========================================================================
// 1. Auto-drop patterns (25 tests)
// ===========================================================================

#[test]
fn drop_int_at_end_of_scope() {
    assert!(ownership_ok("fn foo() { let x = 42; }"));
}

#[test]
fn drop_str_at_end_of_scope() {
    assert!(ownership_ok("fn foo() { let s = \"hello\"; }"));
}

#[test]
fn drop_bool_at_end_of_scope() {
    assert!(ownership_ok("fn foo() { let b = true; }"));
}

#[test]
fn drop_float_at_end_of_scope() {
    assert!(ownership_ok("fn foo() { let f = 3.14; }"));
}

#[test]
fn drop_array_at_end_of_scope() {
    assert!(ownership_ok("fn foo() { let arr = [1, 2, 3]; }"));
}

#[test]
fn drop_map_at_end_of_scope() {
    assert!(ownership_ok("fn foo() { let m = {\"a\": 1}; }"));
}

#[test]
fn drop_struct_at_end_of_scope() {
    assert!(ownership_ok("fn foo() { let u = User { name: \"A\" }; }"));
}

#[test]
fn drop_optional_at_end_of_scope() {
    assert!(ownership_ok("fn foo() { let x: Int? = nil; }"));
}

#[test]
fn drop_multiple_vars_end_of_scope() {
    assert!(ownership_ok(
        "fn foo() { let a = 1;\nlet b = 2;\nlet c = 3; }"
    ));
}

#[test]
fn drop_nested_scope() {
    assert!(ownership_ok("fn foo() { if true { let x = 5; } }"));
}

#[test]
fn drop_in_for_loop_body() {
    assert!(ownership_ok(
        "fn foo() { for i in 0..3 { let x = i * 2; } }"
    ));
}

#[test]
fn drop_closure_at_end_of_scope() {
    assert!(ownership_ok("fn foo() { let f = (x) => x + 1; }"));
}

#[test]
fn drop_tuple_at_end_of_scope() {
    assert!(ownership_ok("fn foo() { let t = (1, \"hi\", true); }"));
}

#[test]
fn drop_after_last_use() {
    assert!(ownership_ok("fn foo() { let x = 42;\nprint(x); }"));
}

#[test]
fn drop_unused_variable() {
    assert!(ownership_ok(
        "fn foo() { let x = 42;\nlet y = 10;\nprint(y); }"
    ));
}

#[test]
fn drop_in_else_branch() {
    assert!(ownership_ok(
        "fn foo() { if false { let a = 1; } else { let b = 2; } }"
    ));
}

#[test]
fn drop_nested_struct() {
    assert!(ownership_ok("fn foo() { let addr = Address { city: \"NYC\" };\nlet u = User { name: \"A\", address: addr }; }"));
}

#[test]
fn drop_array_of_structs() {
    assert!(ownership_ok(
        "fn foo() { let users = [User { name: \"A\" }, User { name: \"B\" }]; }"
    ));
}

#[test]
fn drop_map_of_arrays() {
    assert!(ownership_ok(
        "fn foo() { let m: {Str: [Int]} = {\"a\": [1, 2]}; }"
    ));
}

#[test]
fn drop_result_wrapper() {
    assert!(ownership_ok("fn foo() { let r = Ok(42); }"));
}

#[test]
fn drop_err_wrapper() {
    assert!(ownership_ok("fn foo() { let r = Err(\"fail\"); }"));
}

#[test]
fn drop_after_method_call() {
    assert!(ownership_ok(
        "fn foo() { let arr = [1, 2, 3];\nlet len = arr.length(); }"
    ));
}

#[test]
fn drop_mut_variable() {
    assert!(ownership_ok("fn foo() { let mut x = 0;\nx = 5; }"));
}

#[test]
fn drop_deeply_nested_scope() {
    assert!(ownership_ok(
        "fn foo() { if true { for i in 0..1 { if true { let x = 42; } } } }"
    ));
}

#[test]
fn drop_string_interpolation_result() {
    assert!(ownership_ok(
        "fn foo() { let name = \"world\";\nlet msg = \"hello ${name}\"; }"
    ));
}

// ===========================================================================
// 2. Auto-clone on reuse (25 tests)
// ===========================================================================

#[test]
fn clone_int_used_twice() {
    assert!(ownership_ok("let x = 5;\nlet a = x;\nlet b = x;"));
}

#[test]
fn clone_str_used_twice() {
    assert!(ownership_ok("let s = \"hello\";\nlet a = s;\nlet b = s;"));
}

#[test]
fn clone_array_used_in_two_calls() {
    assert!(ownership_ok("let arr = [1, 2, 3];\nprint(arr);\nprint(arr);"));
}

#[test]
fn clone_map_used_in_two_calls() {
    assert!(ownership_ok("let m = {\"a\": 1};\nprint(m);\nprint(m);"));
}

#[test]
fn clone_struct_used_in_two_calls() {
    assert!(ownership_ok(
        "let u = User { name: \"A\" };\nprint(u);\nprint(u);"
    ));
}

#[test]
fn clone_variable_in_binary_and_print() {
    assert!(ownership_ok("let x = 10;\nlet y = x + 5;\nprint(x);"));
}

#[test]
fn clone_variable_in_multiple_binaries() {
    assert!(ownership_ok(
        "let x = 5;\nlet a = x + 1;\nlet b = x + 2;\nlet c = x + 3;"
    ));
}

#[test]
fn clone_array_in_loop_and_method() {
    assert!(ownership_ok(
        "let arr = [1, 2, 3];\nfor item in arr { print(item); }\nlet len = arr.length();"
    ));
}

#[test]
fn clone_string_in_concat_and_print() {
    assert!(ownership_ok(
        "let s = \"base\";\nlet a = s + \"_1\";\nprint(s);"
    ));
}

#[test]
fn clone_passed_to_multiple_functions() {
    assert!(ownership_ok(
        "fn foo(x: Int) { print(x); }\nfn bar(x: Int) { print(x); }\nlet v = 42;\nfoo(v);\nbar(v);"
    ));
}

#[test]
fn clone_in_conditional_and_after() {
    assert!(ownership_ok("let x = 5;\nif x > 0 { print(x); }\nprint(x);"));
}

#[test]
fn clone_in_match_and_after() {
    assert!(ownership_ok(
        "let x = 42;\nmatch { x > 0 => print(\"pos\"), _ => print(\"neg\") }\nprint(x);"
    ));
}

#[test]
fn clone_struct_fields_accessed_multiple_times() {
    assert!(ownership_ok(
        "let u = User { name: \"A\", age: 30 };\nlet n = u.name;\nlet a = u.age;\nprint(u);"
    ));
}

#[test]
fn clone_in_closure_and_after() {
    assert!(ownership_ok(
        "let x = 5;\nlet f = () => x;\nprint(x);\nprint(f());"
    ));
}

#[test]
fn clone_array_in_spread_and_iteration() {
    assert!(ownership_ok(
        "let a = [1, 2, 3];\nlet b = [...a, 4];\nfor item in a { print(item); }"
    ));
}

#[test]
fn clone_bool_in_multiple_conditions() {
    assert!(ownership_ok(
        "let flag = true;\nif flag { print(\"a\"); }\nif !flag { print(\"b\"); }\nprint(flag);"
    ));
}

#[test]
fn clone_float_in_arithmetic_and_cast() {
    assert!(ownership_ok(
        "let f = 3.14;\nlet a = f * 2.0;\nlet b = f as Int;"
    ));
}

#[test]
fn clone_variable_as_both_sides_of_binary() {
    assert!(ownership_ok("let x = 5;\nlet y = x + x;"));
}

#[test]
fn clone_variable_in_nested_calls() {
    assert!(ownership_ok("let x = 5;\nlet y = add(x, mul(x, x));"));
}

#[test]
fn clone_map_access_and_print() {
    assert!(ownership_ok(
        "let m = {\"a\": 1};\nlet v = m[\"a\"];\nprint(m);"
    ));
}

#[test]
fn clone_in_ternary_and_after() {
    assert!(ownership_ok(
        "let x = 5;\nlet y = if x > 0 { x } else { 0 };\nprint(x);"
    ));
}

#[test]
fn clone_tuple_used_twice() {
    assert!(ownership_ok("let t = (1, \"hi\");\nprint(t);\nprint(t);"));
}

#[test]
fn clone_ok_result_used_twice() {
    assert!(ownership_ok("let r = Ok(42);\nprint(r);\nprint(r);"));
}

#[test]
fn clone_in_array_elements() {
    assert!(ownership_ok("let x = 42;\nlet arr = [x, x, x, x];"));
}

#[test]
fn clone_in_struct_fields() {
    assert!(ownership_ok(
        "let name = \"Alice\";\nlet u1 = User { name: name };\nlet u2 = User { name: name };"
    ));
}

// ===========================================================================
// 3. Move to function (25 tests)
// ===========================================================================

#[test]
fn move_int_to_function() {
    assert!(ownership_ok(
        "fn consume(x: Int) { print(x); }\nlet v = 42;\nconsume(v);"
    ));
}

#[test]
fn move_str_to_function() {
    assert!(ownership_ok(
        "fn consume(s: Str) { print(s); }\nlet v = \"hello\";\nconsume(v);"
    ));
}

#[test]
fn move_bool_to_function() {
    assert!(ownership_ok(
        "fn consume(b: Bool) { print(b); }\nlet v = true;\nconsume(v);"
    ));
}

#[test]
fn move_float_to_function() {
    assert!(ownership_ok(
        "fn consume(f: Float) { print(f); }\nlet v = 3.14;\nconsume(v);"
    ));
}

#[test]
fn move_array_to_function() {
    assert!(ownership_ok(
        "fn process(arr: [Int]) { print(arr); }\nlet data = [1, 2, 3];\nprocess(data);"
    ));
}

#[test]
fn move_map_to_function() {
    assert!(ownership_ok(
        "fn process(m: {Str: Int}) { print(m); }\nlet data = {\"a\": 1};\nprocess(data);"
    ));
}

#[test]
fn move_struct_to_function() {
    assert!(ownership_ok(
        "fn process(u: User) { print(u.name); }\nlet u = User { name: \"A\" };\nprocess(u);"
    ));
}

#[test]
fn move_closure_to_function() {
    assert!(ownership_ok("let f = (x) => x + 1;\nlet result = f(5);"));
}

#[test]
fn move_tuple_to_function() {
    assert!(ownership_ok(
        "fn process(t: (Int, Str)) { print(t); }\nlet pair = (1, \"hi\");\nprocess(pair);"
    ));
}

#[test]
fn move_optional_to_function() {
    assert!(ownership_ok(
        "fn process(x: Int?) { print(x); }\nlet v: Int? = 42;\nprocess(v);"
    ));
}

#[test]
fn move_result_ok_to_function() {
    assert!(ownership_ok(
        "fn process(r: Int) { print(r); }\nlet v = Ok(42);\nprocess(v);"
    ));
}

#[test]
fn move_literal_to_function() {
    assert!(ownership_ok(
        "fn consume(x: Int) { print(x); }\nconsume(42);"
    ));
}

#[test]
fn move_expression_to_function() {
    assert!(ownership_ok(
        "fn consume(x: Int) { print(x); }\nconsume(1 + 2 * 3);"
    ));
}

#[test]
fn move_nested_struct_to_function() {
    assert!(ownership_ok("fn process(u: User) { print(u); }\nlet addr = Address { city: \"NYC\" };\nlet u = User { name: \"A\", address: addr };\nprocess(u);"));
}

#[test]
fn move_array_of_structs_to_function() {
    assert!(ownership_ok("fn process(users: [User]) { print(users); }\nlet us = [User { name: \"A\" }, User { name: \"B\" }];\nprocess(us);"));
}

#[test]
fn move_map_of_arrays_to_function() {
    assert!(ownership_ok("fn process(data: {Str: [Int]}) { print(data); }\nlet d: {Str: [Int]} = {\"a\": [1, 2]};\nprocess(d);"));
}

#[test]
fn move_multiple_args_to_function() {
    assert!(ownership_ok("fn foo(a: Int, b: Str, c: Bool) { print(a); }\nlet x = 1;\nlet s = \"hi\";\nlet b = true;\nfoo(x, s, b);"));
}

#[test]
fn move_nested_function_result() {
    assert!(ownership_ok(
        "fn inner() -> Int { return 42; }\nfn outer(x: Int) { print(x); }\nouter(inner());"
    ));
}

#[test]
fn move_method_result_to_function() {
    assert!(ownership_ok(
        "fn process(s: Str) { print(s); }\nlet name = \"hello\";\nprocess(name.toUpperCase());"
    ));
}

#[test]
fn move_field_access_to_function() {
    assert!(ownership_ok(
        "fn greet(name: Str) { print(name); }\nlet u = User { name: \"Alice\" };\ngreet(u.name);"
    ));
}

#[test]
fn move_array_element_to_function() {
    assert!(ownership_ok(
        "fn process(x: Int) { print(x); }\nlet arr = [10, 20, 30];\nprocess(arr[0]);"
    ));
}

#[test]
fn move_map_value_to_function() {
    assert!(ownership_ok(
        "fn process(x: Int) { print(x); }\nlet m = {\"key\": 42};\nprocess(m[\"key\"]);"
    ));
}

#[test]
fn move_ternary_to_function() {
    assert!(ownership_ok(
        "fn process(x: Int) { print(x); }\nlet flag = true;\nprocess(if flag { 1 } else { 2 });"
    ));
}

#[test]
fn move_cast_to_function() {
    assert!(ownership_ok(
        "fn process(f: Float) { print(f); }\nlet x = 42;\nprocess(x as Float);"
    ));
}

#[test]
fn move_closure_as_callback() {
    assert!(ownership_ok(
        "let arr = [1, 2, 3];\nlet doubled = arr.map((x) => x * 2);"
    ));
}

// ===========================================================================
// 4. Return ownership (20 tests)
// ===========================================================================

#[test]
fn return_int_literal() {
    assert!(ownership_ok("fn foo() -> Int { return 42; }"));
}

#[test]
fn return_str_literal() {
    assert!(ownership_ok("fn foo() -> Str { return \"hello\"; }"));
}

#[test]
fn return_bool_literal() {
    assert!(ownership_ok("fn foo() -> Bool { return true; }"));
}

#[test]
fn return_float_literal() {
    assert!(ownership_ok("fn foo() -> Float { return 3.14; }"));
}

#[test]
fn return_local_variable() {
    assert!(ownership_ok("fn foo() -> Int { let x = 42;\nreturn x; }"));
}

#[test]
fn return_computed_value() {
    assert!(ownership_ok(
        "fn add(a: Int, b: Int) -> Int { return a + b; }"
    ));
}

#[test]
fn return_array_literal() {
    assert!(ownership_ok("fn getItems() -> [Int] { return [1, 2, 3]; }"));
}

#[test]
fn return_map_literal() {
    assert!(ownership_ok(
        "fn getMap() -> {Str: Int} { return {\"a\": 1}; }"
    ));
}

#[test]
fn return_struct_literal() {
    assert!(ownership_ok(
        "fn getUser() -> User { return User { name: \"Alice\", age: 30 }; }"
    ));
}

#[test]
fn return_tuple_literal() {
    assert!(ownership_ok(
        "fn pair() -> (Int, Str) { return (1, \"hi\"); }"
    ));
}

#[test]
fn return_optional_nil() {
    assert!(ownership_ok("fn find() -> Int? { return nil; }"));
}

#[test]
fn return_optional_value() {
    assert!(ownership_ok("fn find() -> Int? { return 42; }"));
}

#[test]
fn return_ok_wrapper() {
    assert!(ownership_ok("fn compute() -> Int { Ok 42; }"));
}

#[test]
fn return_err_wrapper() {
    assert!(ownership_ok("fn compute() -> Int { Err \"fail\"; }"));
}

#[test]
fn return_local_array() {
    assert!(ownership_ok(
        "fn build() -> [Int] { let arr = [1, 2, 3];\nreturn arr; }"
    ));
}

#[test]
fn return_local_map() {
    assert!(ownership_ok(
        "fn build() -> {Str: Int} { let m = {\"a\": 1};\nreturn m; }"
    ));
}

#[test]
fn return_local_struct() {
    assert!(ownership_ok(
        "fn build() -> User { let u = User { name: \"A\" };\nreturn u; }"
    ));
}

#[test]
fn return_from_expression_fn() {
    assert!(ownership_ok("fn double(x: Int) -> Int => x * 2"));
}

#[test]
fn return_conditional_value() {
    assert!(ownership_ok(
        "fn abs(x: Int) -> Int { if x < 0 { return -x; }\nreturn x; }"
    ));
}

#[test]
fn return_from_match() {
    assert!(ownership_ok("fn classify(x: Int) -> Str { return match { x > 0 => \"pos\", x < 0 => \"neg\", _ => \"zero\" }; }"));
}

// ===========================================================================
// 5. Collection ownership (25 tests)
// ===========================================================================

#[test]
fn coll_own_array_of_ints() {
    assert!(ownership_ok("let arr = [1, 2, 3];\nprint(arr);"));
}

#[test]
fn coll_own_array_of_strs() {
    assert!(ownership_ok("let arr = [\"a\", \"b\", \"c\"];\nprint(arr);"));
}

#[test]
fn coll_own_array_of_floats() {
    assert!(ownership_ok("let arr = [1.0, 2.0, 3.0];\nprint(arr);"));
}

#[test]
fn coll_own_array_of_bools() {
    assert!(ownership_ok("let arr = [true, false, true];\nprint(arr);"));
}

#[test]
fn coll_own_array_of_structs() {
    assert!(ownership_ok(
        "let users = [User { name: \"A\" }, User { name: \"B\" }];\nprint(users);"
    ));
}

#[test]
fn coll_own_nested_array() {
    assert!(ownership_ok("let matrix = [[1, 2], [3, 4]];\nprint(matrix);"));
}

#[test]
fn coll_own_map_str_int() {
    assert!(ownership_ok("let m = {\"a\": 1, \"b\": 2};\nprint(m);"));
}

#[test]
fn coll_own_map_str_str() {
    assert!(ownership_ok("let m = {\"key\": \"value\"};\nprint(m);"));
}

#[test]
fn coll_own_map_int_str() {
    assert!(ownership_ok("let m = {1: \"one\", 2: \"two\"};\nprint(m);"));
}

#[test]
fn coll_own_map_of_arrays() {
    assert!(ownership_ok(
        "let m: {Str: [Int]} = {\"a\": [1, 2]};\nprint(m);"
    ));
}

#[test]
fn coll_own_array_of_maps() {
    assert!(ownership_ok(
        "let arr: [{Str: Int}] = [{\"a\": 1}, {\"b\": 2}];\nprint(arr);"
    ));
}

#[test]
fn coll_own_push_to_array() {
    assert!(ownership_ok(
        "let mut arr: [Int] = [];\narr.push(1);\narr.push(2);\nprint(arr);"
    ));
}

#[test]
fn coll_own_array_element_replaced() {
    assert!(ownership_ok(
        "let mut arr = [1, 2, 3];\narr[0] = 10;\nprint(arr);"
    ));
}

#[test]
fn coll_own_map_entry_added() {
    assert!(ownership_ok(
        "let mut m = {\"a\": 1};\nm[\"b\"] = 2;\nprint(m);"
    ));
}

#[test]
fn coll_own_array_from_function() {
    assert!(ownership_ok(
        "fn getItems() -> [Int] { return [1, 2, 3]; }\nlet arr = getItems();\nprint(arr);"
    ));
}

#[test]
fn coll_own_map_from_function() {
    assert!(ownership_ok(
        "fn getMap() -> {Str: Int} { return {\"a\": 1}; }\nlet m = getMap();\nprint(m);"
    ));
}

#[test]
fn coll_own_spread_creates_new_array() {
    assert!(ownership_ok(
        "let a = [1, 2];\nlet b = [...a, 3, 4];\nprint(a);\nprint(b);"
    ));
}

#[test]
fn coll_own_array_method_produces_new() {
    assert!(ownership_ok(
        "let arr = [1, 2, 3];\nlet mapped = arr.map((x) => x * 2);\nprint(arr);\nprint(mapped);"
    ));
}

#[test]
fn coll_own_filter_produces_new() {
    assert!(ownership_ok("let arr = [1, 2, 3, 4, 5];\nlet filtered = arr.filter((x) => x > 2);\nprint(arr);\nprint(filtered);"));
}

#[test]
fn coll_own_reduce_consumes_array() {
    assert!(ownership_ok(
        "let arr = [1, 2, 3];\nlet sum = arr.reduce(0, (a, b) => a + b);\nprint(sum);"
    ));
}

#[test]
fn coll_own_array_in_struct() {
    assert!(ownership_ok("struct Team { members: [Str] }\nlet t = Team { members: [\"A\", \"B\"] };\nprint(t.members);"));
}

#[test]
fn coll_own_map_in_struct() {
    assert!(ownership_ok("struct Config { settings: {Str: Str} }\nlet c = Config { settings: {\"key\": \"val\"} };\nprint(c.settings);"));
}

#[test]
fn coll_own_empty_array_typed() {
    assert!(ownership_ok("let arr: [Int] = [];\nprint(arr);"));
}

#[test]
fn coll_own_empty_map_typed() {
    assert!(ownership_ok("let m: {Str: Int} = {};\nprint(m);"));
}

#[test]
fn coll_own_array_chained_methods() {
    assert!(ownership_ok(
        "let result = [1, 2, 3, 4, 5].filter((x) => x > 2).map((x) => x * 10);\nprint(result);"
    ));
}

// ===========================================================================
// 6. Struct field ownership (20 tests)
// ===========================================================================

#[test]
fn struct_own_simple_fields() {
    assert!(ownership_ok(
        "struct User { name: Str, age: Int }\nlet u = User { name: \"Alice\", age: 30 };"
    ));
}

#[test]
fn struct_own_nested_struct() {
    assert!(ownership_ok("struct Address { city: Str }\nstruct User { name: Str, address: Address }\nlet u = User { name: \"A\", address: Address { city: \"NYC\" } };"));
}

#[test]
fn struct_own_array_field() {
    assert!(ownership_ok(
        "struct Team { members: [Str] }\nlet t = Team { members: [\"A\", \"B\"] };"
    ));
}

#[test]
fn struct_own_map_field() {
    assert!(ownership_ok(
        "struct Config { settings: {Str: Int} }\nlet c = Config { settings: {\"timeout\": 30} };"
    ));
}

#[test]
fn struct_own_optional_field() {
    assert!(ownership_ok(
        "struct User { email: Str? }\nlet u = User { email: nil };"
    ));
}

#[test]
fn struct_own_field_read() {
    assert!(ownership_ok(
        "let u = User { name: \"Alice\" };\nlet n = u.name;\nprint(n);"
    ));
}

#[test]
fn struct_own_field_write() {
    assert!(ownership_ok(
        "let mut u = User { name: \"Alice\" };\nu.name = \"Bob\";"
    ));
}

#[test]
fn struct_own_field_passed_to_function() {
    assert!(ownership_ok(
        "fn greet(name: Str) { print(name); }\nlet u = User { name: \"Alice\" };\ngreet(u.name);"
    ));
}

#[test]
fn struct_own_field_in_expression() {
    assert!(ownership_ok(
        "let p = Point { x: 10, y: 20 };\nlet sum = p.x + p.y;"
    ));
}

#[test]
fn struct_own_with_default_value() {
    assert!(ownership_ok(
        "struct Config { timeout: Int = 30 }\nlet c = Config { };"
    ));
}

#[test]
fn struct_own_all_optional() {
    assert!(ownership_ok(
        "struct Partial { a: Int?, b: Str?, c: Bool? }\nlet p = Partial { a: nil, b: nil, c: nil };"
    ));
}

#[test]
fn struct_own_method_self_access() {
    assert!(ownership_ok(
        "fn User.greet(self) -> Str { return \"Hello \" + self.name; }"
    ));
}

#[test]
fn struct_own_method_self_mutation() {
    assert!(ownership_ok(
        "fn User.setName(self, name: Str) { self.name = name; }"
    ));
}

#[test]
fn struct_own_created_from_variables() {
    assert!(ownership_ok(
        "let name = \"Alice\";\nlet age = 30;\nlet u = User { name: name, age: age };"
    ));
}

#[test]
fn struct_own_field_chain_access() {
    assert!(ownership_ok(
        "let u = User { address: Address { city: \"NYC\" } };\nlet city = u.address.city;"
    ));
}

#[test]
fn struct_own_returned_from_function() {
    assert!(ownership_ok(
        "fn createUser() -> User { return User { name: \"A\", age: 25 }; }\nlet u = createUser();"
    ));
}

#[test]
fn struct_own_in_array() {
    assert!(ownership_ok(
        "let users = [User { name: \"A\" }, User { name: \"B\" }];\nprint(users[0].name);"
    ));
}

#[test]
fn struct_own_in_map_value() {
    assert!(ownership_ok(
        "let m: {Str: User} = {\"admin\": User { name: \"A\" }};\nprint(m[\"admin\"]);"
    ));
}

#[test]
fn struct_own_decorated() {
    assert!(ownership_ok(
        "@table struct User { name: Str, age: Int }\nlet u = User { name: \"A\", age: 30 };"
    ));
}

#[test]
fn struct_own_with_enum_field() {
    assert!(ownership_ok("enum Status { Active, Inactive }\nstruct User { name: Str, status: Status }\nlet u = User { name: \"A\", status: Status::Active };"));
}

// ===========================================================================
// 7. Closure capture ownership (20 tests)
// ===========================================================================

#[test]
fn closure_own_capture_int() {
    assert!(ownership_ok("let x = 5;\nlet f = () => x;\nprint(f());"));
}

#[test]
fn closure_own_capture_str() {
    assert!(ownership_ok(
        "let s = \"hello\";\nlet f = () => s;\nprint(f());"
    ));
}

#[test]
fn closure_own_capture_in_expression() {
    assert!(ownership_ok(
        "let base = 10;\nlet f = (x) => x + base;\nprint(f(5));"
    ));
}

#[test]
fn closure_own_capture_multiple() {
    assert!(ownership_ok(
        "let a = 1;\nlet b = 2;\nlet f = () => a + b;\nprint(f());"
    ));
}

#[test]
fn closure_own_capture_struct() {
    assert!(ownership_ok(
        "let u = User { name: \"A\" };\nlet f = () => u.name;\nprint(f());"
    ));
}

#[test]
fn closure_own_capture_array() {
    assert!(ownership_ok(
        "let arr = [1, 2, 3];\nlet f = () => arr.length();\nprint(f());"
    ));
}

#[test]
fn closure_own_capture_map() {
    assert!(ownership_ok(
        "let m = {\"a\": 1};\nlet f = () => m[\"a\"];\nprint(f());"
    ));
}

#[test]
fn closure_own_capture_bool() {
    assert!(ownership_ok(
        "let flag = true;\nlet f = () => !flag;\nprint(f());"
    ));
}

#[test]
fn closure_own_capture_in_method_callback() {
    assert!(ownership_ok(
        "let factor = 3;\nlet result = [1, 2, 3].map((x) => x * factor);\nprint(result);"
    ));
}

#[test]
fn closure_own_capture_in_filter() {
    assert!(ownership_ok(
        "let threshold = 5;\nlet result = [1, 2, 10].filter((x) => x > threshold);\nprint(result);"
    ));
}

#[test]
fn closure_own_capture_in_reduce() {
    assert!(ownership_ok(
        "let initial = 100;\nlet result = [1, 2, 3].reduce(initial, (a, b) => a + b);\nprint(result);"
    ));
}

#[test]
fn closure_own_capture_nested_closure() {
    assert!(ownership_ok(
        "let x = 5;\nlet f = () => () => x;\nprint(f()());"
    ));
}

#[test]
fn closure_own_capture_and_use_outer() {
    assert!(ownership_ok(
        "let x = 5;\nlet f = () => x + 1;\nprint(x);\nprint(f());"
    ));
}

#[test]
fn closure_own_capture_in_route() {
    assert!(ownership_ok(
        "let data = [1, 2, 3];\napp.get(\"/data\", (req) => data);"
    ));
}

#[test]
fn closure_own_capture_parameter() {
    assert!(ownership_ok(
        "fn makeMultiplier(n: Int) { let f = (x) => x * n;\nprint(f(5)); }"
    ));
}

#[test]
fn closure_own_capture_complex_expression() {
    assert!(ownership_ok(
        "let a = 10;\nlet b = 20;\nlet f = (x) => (x + a) * b;\nprint(f(1));"
    ));
}

#[test]
fn closure_own_capture_in_chained_methods() {
    assert!(ownership_ok("let min = 2;\nlet max = 8;\nlet result = [1, 2, 3, 4, 5, 10].filter((x) => x >= min && x <= max);\nprint(result);"));
}

#[test]
fn closure_own_shadowing_param() {
    assert!(ownership_ok(
        "let x = 5;\nlet f = (x) => x * 2;\nprint(f(10));\nprint(x);"
    ));
}

#[test]
fn closure_own_capture_optional() {
    assert!(ownership_ok(
        "let v: Int? = 42;\nlet f = () => v;\nprint(f());"
    ));
}

#[test]
fn closure_own_capture_enum() {
    assert!(ownership_ok(
        "let status = Status::Active;\nlet f = () => status;\nprint(f());"
    ));
}

// ===========================================================================
// 8. Conditional ownership (20 tests)
// ===========================================================================

#[test]
fn cond_own_if_branch_creates_var() {
    assert!(ownership_ok("if true { let x = 42;\nprint(x); }"));
}

#[test]
fn cond_own_if_else_both_create() {
    assert!(ownership_ok(
        "if true { let x = 1;\nprint(x); } else { let x = 2;\nprint(x); }"
    ));
}

#[test]
fn cond_own_if_else_as_expression() {
    assert!(ownership_ok("let x = if true { 42 } else { 0 };"));
}

#[test]
fn cond_own_ternary() {
    assert!(ownership_ok("let x = if true { 42 } else { 0 };"));
}

#[test]
fn cond_own_match_arms() {
    assert!(ownership_ok(
        "let x = 5;\nlet r = match { x > 0 => \"pos\", _ => \"neg\" };"
    ));
}

#[test]
fn cond_own_match_with_payload() {
    assert!(ownership_ok("enum MyResult { Success(Int), Failure(Str) }\nlet r = MyResult::Success(42);\nmatch r { MyResult::Success(v) => print(v), MyResult::Failure(e) => print(e) }"));
}

#[test]
fn cond_own_if_else_if_chain() {
    assert!(ownership_ok("let x = 5;\nif x > 10 { print(\"big\"); } else if x > 0 { print(\"small\"); } else { print(\"neg\"); }"));
}

#[test]
fn cond_own_variable_used_in_both_branches() {
    assert!(ownership_ok(
        "let x = 5;\nif true { print(x); } else { print(x); }"
    ));
}

#[test]
fn cond_own_mut_variable_set_in_branches() {
    assert!(ownership_ok(
        "let mut x = 0;\nif true { x = 1; } else { x = 2; }\nprint(x);"
    ));
}

#[test]
fn cond_own_struct_in_conditional() {
    assert!(ownership_ok(
        "let flag = true;\nlet u = if flag { User { name: \"A\" } } else { User { name: \"B\" } };"
    ));
}

#[test]
fn cond_own_array_in_conditional() {
    assert!(ownership_ok(
        "let flag = true;\nlet arr = if flag { [1, 2, 3] } else { [4, 5, 6] };"
    ));
}

#[test]
fn cond_own_nested_if() {
    assert!(ownership_ok("let x = 5;\nif x > 0 { if x < 10 { print(\"single digit\"); } else { print(\"multi digit\"); } }"));
}

#[test]
fn cond_own_if_with_local_scope() {
    assert!(ownership_ok(
        "if true { let a = 1;\nlet b = 2;\nlet c = a + b;\nprint(c); }"
    ));
}

#[test]
fn cond_own_match_enum_variants() {
    assert!(ownership_ok("let c = Color::Red;\nmatch c { Color::Red => print(\"red\"), Color::Blue => print(\"blue\"), _ => print(\"other\") }"));
}

#[test]
fn cond_own_null_coalesce_ownership() {
    assert!(ownership_ok("let x: Int? = nil;\nlet y = 42;\nprint(y);"));
}

#[test]
fn cond_own_try_operator() {
    assert!(ownership_ok("let v = getData()?;\nprint(v);"));
}

#[test]
fn cond_own_if_expression_with_blocks() {
    assert!(ownership_ok(
        "let result = if true { let x = 42;\nx } else { let y = 0;\ny };"
    ));
}

#[test]
fn cond_own_match_conditional_arms() {
    assert!(ownership_ok("let x = 50;\nmatch { x > 100 => print(\"big\"), x > 10 => print(\"medium\"), _ => print(\"small\") }"));
}

#[test]
fn cond_own_ternary_with_function_calls() {
    assert!(ownership_ok(
        "let result = if isReady() { getValue() } else { getDefault() };"
    ));
}

#[test]
fn cond_own_if_else_returning_different_collections() {
    assert!(ownership_ok(
        "let flag = true;\nlet data = if flag { [1, 2, 3] } else { [] };"
    ));
}

// ===========================================================================
// 9. Loop ownership (20 tests)
// ===========================================================================

#[test]
fn loop_own_for_range_variable() {
    assert!(ownership_ok("for i in 0..10 { print(i); }"));
}

#[test]
fn loop_own_for_array_element() {
    assert!(ownership_ok("for item in [1, 2, 3] { print(item); }"));
}

#[test]
fn loop_own_for_string_array() {
    assert!(ownership_ok(
        "for name in [\"Alice\", \"Bob\"] { print(name); }"
    ));
}

#[test]
fn loop_own_for_struct_array() {
    assert!(ownership_ok("let users = [User { name: \"A\" }, User { name: \"B\" }];\nfor u in users { print(u.name); }"));
}

#[test]
fn loop_own_for_with_index() {
    assert!(ownership_ok(
        "let arr = [10, 20, 30];\nfor i, val in arr { print(i, val); }"
    ));
}

#[test]
fn loop_own_for_accumulator_pattern() {
    assert!(ownership_ok(
        "let mut total = 0;\nfor i in 0..10 { total += i; }\nprint(total);"
    ));
}

#[test]
fn loop_own_for_builder_pattern() {
    assert!(ownership_ok(
        "let mut result: [Int] = [];\nfor i in 0..5 { result.push(i * 2); }\nprint(result);"
    ));
}

#[test]
fn loop_own_for_with_outer_immutable() {
    assert!(ownership_ok(
        "let multiplier = 3;\nfor i in 0..5 { print(i * multiplier); }"
    ));
}

#[test]
fn loop_own_for_with_local_variable() {
    assert!(ownership_ok(
        "for i in 0..5 { let doubled = i * 2;\nprint(doubled); }"
    ));
}

#[test]
fn loop_own_for_with_break() {
    assert!(ownership_ok(
        "for i in 0..100 { if i > 10 { break; } print(i); }"
    ));
}

#[test]
fn loop_own_for_with_continue() {
    assert!(ownership_ok(
        "for i in 0..20 { if i % 2 == 0 { continue; } print(i); }"
    ));
}

#[test]
fn loop_own_nested_for() {
    assert!(ownership_ok(
        "for i in 0..3 { for j in 0..3 { print(i, j); } }"
    ));
}

#[test]
fn loop_own_for_string_concat() {
    assert!(ownership_ok("let mut result = \"\";\nfor name in [\"A\", \"B\", \"C\"] { result = result + name; }\nprint(result);"));
}

#[test]
fn loop_own_for_over_filtered_array() {
    assert!(ownership_ok(
        "let arr = [1, 2, 3, 4, 5];\nfor item in arr.filter((x) => x > 2) { print(item); }"
    ));
}

#[test]
fn loop_own_for_creating_structs() {
    assert!(ownership_ok("let mut users: [User] = [];\nfor name in [\"A\", \"B\"] { users.push(User { name: name }); }"));
}

#[test]
fn loop_own_for_map_access_each_iteration() {
    assert!(ownership_ok(
        "let m = {\"a\": 1, \"b\": 2};\nlet keys = [\"a\", \"b\"];\nfor k in keys { print(m[k]); }"
    ));
}

#[test]
fn loop_own_infinite_loop_with_break() {
    assert!(ownership_ok(
        "let mut count = 0;\nfor { count++;\nif count > 5 { break; } }"
    ));
}

#[test]
fn loop_own_for_variable_bounds() {
    assert!(ownership_ok(
        "let start = 0;\nlet end = 10;\nfor i in start..end { print(i); }"
    ));
}

#[test]
fn loop_own_for_inclusive_range() {
    assert!(ownership_ok("for i in 1..=10 { print(i); }"));
}

#[test]
fn loop_own_complex_loop_body() {
    assert!(ownership_ok("let mut even_sum = 0;\nlet mut odd_sum = 0;\nfor i in 0..20 { if i % 2 == 0 { even_sum += i; } else { odd_sum += i; } }\nprint(even_sum, odd_sum);"));
}
