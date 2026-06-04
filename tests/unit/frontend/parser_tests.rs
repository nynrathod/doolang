use doo_core::errors::codes::CompilerError;
use doo_frontend::ast::*;
use doo_frontend::Parser;
use doo_frontend::{Expr, ExprKind, Item, Program, Stmt, StmtKind};

// Type aliases for backward compatibility
type Statement = StmtKind;
type Type = TypeExprKind;

fn parse(source: &str) -> Program {
    let mut parser = Parser::new(source, 0);
    parser.parse_program().unwrap()
}

fn parse_with_errors(source: &str) -> (Program, Vec<CompilerError>) {
    let mut parser = Parser::new(source, 0);
    let prog = parser.parse_program().unwrap();
    let errors = parser.errors().to_vec();
    (prog, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================
    // Function Declarations (40 tests)
    // ========================================

    #[test]
    fn test_simple_empty_function() {
        let prog = parse("fn foo() { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.name, "foo");
                assert_eq!(f.params.len(), 0);
                assert!(f.return_type.is_none());
                assert!(f.error_type.is_none());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_params() {
        let prog = parse("fn add(a: Int, b: Int) -> Int { return a + b }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.name, "add");
                assert_eq!(f.params.len(), 2);
                assert_eq!(f.params[0].0, "a");
                assert_eq!(f.params[1].0, "b");
                assert!(f.return_type.is_some());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_return_type() {
        let prog = parse("fn getValue() -> Int { return 42 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.name, "getValue");
                assert!(f.return_type.is_some());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_error_type() {
        let prog = parse("fn read() -> Str ! Error { return \"hello\" }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.name, "read");
                assert!(f.return_type.is_some());
                assert!(f.error_type.is_some());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_expression_function() {
        let prog = parse("fn double(x: Int) -> Int => x * 2");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.name, "double");
                assert_eq!(f.params.len(), 1);
                assert!(f.return_type.is_some());
                assert!(f.is_expr_fn);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_public_function_uppercase() {
        let prog = parse("fn Add(a: Int, b: Int) -> Int { return a + b }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.name, "Add");
                assert!(f.is_public);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_private_function_lowercase() {
        let prog = parse("fn helper() { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.name, "helper");
                assert!(!f.is_public);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_method_with_receiver() {
        let prog = parse("fn User.greet(self) -> Str { return \"hello\" }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(f.receiver.is_some());
                assert_eq!(f.params.len(), 1);
                assert_eq!(f.params[0].0, "self");
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_static_method() {
        let prog = parse("fn User.create() -> User { return User{} }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(f.receiver.is_some());
                assert_eq!(f.params.len(), 0);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_no_params_no_return() {
        let prog = parse("fn doSomething() { print(\"hi\") }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.params.len(), 0);
                assert!(f.return_type.is_none());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_many_parameters() {
        let prog = parse("fn complex(a: Int, b: Str, c: Bool, d: Float, e: Char, f: Int) { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.params.len(), 6);
                assert_eq!(f.params[0].0, "a");
                assert_eq!(f.params[5].0, "f");
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_nested_function_body_with_let() {
        let prog = parse("fn foo() { let x = 10 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(!f.body.is_empty());
                assert_eq!(f.body.len(), 1);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_nested_function_body_with_if() {
        let prog = parse("fn foo() { if true { print(\"yes\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(!f.body.is_empty());
                assert_eq!(f.body.len(), 1);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_nested_function_body_with_for() {
        let prog = parse("fn foo() { for i in 1..10 { print(i) } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(!f.body.is_empty());
                assert_eq!(f.body.len(), 1);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_nested_function_body_with_return() {
        let prog = parse("fn foo() -> Int { return 42 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(!f.body.is_empty());
                assert_eq!(f.body.len(), 1);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_decorator() {
        let prog = parse("@route fn handler() { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.decorators.len(), 1);
                assert_eq!(f.decorators[0].name, "route");
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_multiple_decorators() {
        let prog = parse("@route @auth @validate fn handler() { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.decorators.len(), 3);
                assert_eq!(f.decorators[0].name, "route");
                assert_eq!(f.decorators[1].name, "auth");
                assert_eq!(f.decorators[2].name, "validate");
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_body_with_multiple_statements() {
        let prog = parse("fn foo() { let x = 10;\n let y = 20;\n print(x + y) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(!f.body.is_empty());
                assert_eq!(f.body.len(), 3);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_empty_function_body() {
        let prog = parse("fn empty() { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.body.len(), 0);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_returning_array_type() {
        let prog = parse("fn getItems() -> [Int] { return [] }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(f.return_type.is_some());
                match &f.return_type.as_ref().unwrap().kind {
                    TypeExprKind::Array(_) => {}
                    _ => panic!("Expected array type"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_returning_map_type() {
        let prog = parse("fn getMap() -> {Str: Int} { return {} }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(f.return_type.is_some());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_returning_optional_type() {
        let prog = parse("fn maybeGet() -> Int? { return nil }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(f.return_type.is_some());
                match &f.return_type.as_ref().unwrap().kind {
                    TypeExprKind::Optional(_) => {}
                    _ => panic!("Expected optional type"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_string_return() {
        let prog = parse("fn greet() -> Str { return \"hello\" }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(f.return_type.is_some());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_bool_return() {
        let prog = parse("fn check() -> Bool { return true }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(f.return_type.is_some());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_float_return() {
        let prog = parse("fn calc() -> Float { return 3.14 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(f.return_type.is_some());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_single_param() {
        let prog = parse("fn square(x: Int) -> Int { return x * x }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.params.len(), 1);
                assert_eq!(f.params[0].0, "x");
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_two_params() {
        let prog = parse("fn mul(a: Int, b: Int) -> Int { return a * b }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.params.len(), 2);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_three_params() {
        let prog = parse("fn combine(a: Int, b: Int, c: Int) -> Int { return a + b + c }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.params.len(), 3);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_mixed_param_types() {
        let prog = parse("fn process(name: Str, age: Int, active: Bool) { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.params.len(), 3);
                assert_eq!(f.params[0].0, "name");
                assert_eq!(f.params[1].0, "age");
                assert_eq!(f.params[2].0, "active");
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_array_param() {
        let prog = parse("fn sum(nums: [Int]) -> Int { return 0 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.params.len(), 1);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_optional_param() {
        let prog = parse("fn find(id: Int?) -> User { return User{} }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.params.len(), 1);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_custom_type_param() {
        let prog = parse("fn save(user: User) { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.params.len(), 1);
                assert_eq!(f.params[0].0, "user");
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_expression_body_no_params() {
        let prog = parse("fn getFortyTwo() -> Int => 42");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.params.len(), 0);
                assert!(f.is_expr_fn);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_expression_body_with_call() {
        let prog = parse("fn wrapper(x: Int) -> Int => process(x)");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(f.is_expr_fn);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_nested_type_return() {
        let prog = parse("fn getData() -> [[Int]] { return [[1, 2], [3, 4]] }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(f.return_type.is_some());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_map_param() {
        let prog = parse("fn processMap(data: {Str: Int}) { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.params.len(), 1);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_multiline_body() {
        let prog = parse("fn multi() {\n  let a = 1;\n  let b = 2;\n  let c = 3;\n}");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(!f.body.is_empty());
                assert_eq!(f.body.len(), 3);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_with_error_type_custom() {
        let prog = parse("fn load() -> Data ! LoadError { return Data{} }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(f.error_type.is_some());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_decorator_with_args() {
        let prog = parse("@route(\"/users\") fn getUsers() { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.decorators.len(), 1);
                assert_eq!(f.decorators[0].name, "route");
                assert!(!f.decorators[0].args.is_empty());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_multiple_decorators_with_args() {
        let prog = parse("@route(\"/api\") @auth(\"admin\") fn admin() { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert_eq!(f.decorators.len(), 2);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_function_method_with_mut_self() {
        let prog = parse("fn User.update(self, name: Str) { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                assert!(f.receiver.is_some());
                assert_eq!(f.params.len(), 2);
            }
            _ => panic!("Expected function"),
        }
    }

    // ========================================
    // Struct Declarations (25 tests)
    // ========================================

    #[test]
    fn test_simple_struct() {
        let prog = parse("struct User { name: Str, age: Int }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.name, "User");
                assert_eq!(s.fields.len(), 2);
                assert_eq!(s.fields[0].name, "name");
                assert_eq!(s.fields[1].name, "age");
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_empty_struct() {
        let prog = parse("struct Empty { }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.name, "Empty");
                assert_eq!(s.fields.len(), 0);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_one_field() {
        let prog = parse("struct Single { value: Int }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 1);
                assert_eq!(s.fields[0].name, "value");
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_many_fields() {
        let prog = parse("struct Complex { a: Int, b: Str, c: Bool, d: Float, e: Char, f: Int }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 6);
                assert_eq!(s.fields[0].name, "a");
                assert_eq!(s.fields[5].name, "f");
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_public_struct_uppercase() {
        let prog = parse("struct User { name: Str }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.name, "User");
                assert!(s.is_public);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_optional_fields() {
        let prog = parse("struct User { name: Str?, age: Int? }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 2);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_decorators() {
        let prog = parse("@table struct User { name: Str }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.decorators.len(), 1);
                assert_eq!(s.decorators[0].name, "table");
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_field_with_decorators() {
        let prog = parse("struct User { age: Int @min(1) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 1);
                assert_eq!(s.fields[0].decorators.len(), 1);
                assert_eq!(s.fields[0].decorators[0].name, "min");
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_defaults() {
        let prog = parse("struct Counter { count: Int = 0 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 1);
                assert!(s.fields[0].default.is_some());
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_nested_type_fields() {
        let prog = parse("struct Group { items: [User] }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 1);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_map_type_field() {
        let prog = parse("struct Config { settings: {Str: Int} }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 1);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_multiline_fields() {
        let prog = parse("struct User {\n  name: Str,\n  age: Int,\n  email: Str,\n}");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 3);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_array_field() {
        let prog = parse("struct Data { values: [Int] }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 1);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_bool_field() {
        let prog = parse("struct Flags { active: Bool }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 1);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_float_field() {
        let prog = parse("struct Point { x: Float, y: Float }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 2);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_multiple_decorators() {
        let prog = parse("@table @validate struct User { name: Str }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.decorators.len(), 2);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_field_multiple_decorators() {
        let prog = parse("struct User { age: Int @min(1) @max(100) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields[0].decorators.len(), 2);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_custom_type_field() {
        let prog = parse("struct Post { author: User }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 1);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_nested_array() {
        let prog = parse("struct Matrix { data: [[Int]] }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 1);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_string_default() {
        let prog = parse("struct User { name: Str = \"Guest\" }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert!(s.fields[0].default.is_some());
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_with_bool_default() {
        let prog = parse("struct User { active: Bool = true }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert!(s.fields[0].default.is_some());
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_decorator_with_args() {
        let prog = parse("@table(\"users\") struct User { name: Str }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.decorators.len(), 1);
                assert!(!s.decorators[0].args.is_empty());
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_field_decorator_with_args() {
        let prog = parse("struct User { name: Str @max(50) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields[0].decorators.len(), 1);
                assert!(!s.fields[0].decorators[0].args.is_empty());
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_mixed_fields_and_decorators() {
        let prog = parse("@table struct User { id: Int @key, name: Str, email: Str @index }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 3);
                assert_eq!(s.decorators.len(), 1);
            }
            _ => panic!("Expected struct"),
        }
    }

    #[test]
    fn test_struct_trailing_comma() {
        let prog = parse("struct User { name: Str, age: Int, }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields.len(), 2);
            }
            _ => panic!("Expected struct"),
        }
    }

    // ========================================
    // Enum Declarations (20 tests)
    // ========================================

    #[test]
    fn test_simple_enum() {
        let prog = parse("enum Color { Red, Green, Blue }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.name, "Color");
                assert_eq!(e.variants.len(), 3);
                assert_eq!(e.variants[0].name, "Red");
                assert_eq!(e.variants[1].name, "Green");
                assert_eq!(e.variants[2].name, "Blue");
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_with_data_variants() {
        let prog = parse("enum Result { Success(Int), Failure(Str) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.name, "Result");
                assert_eq!(e.variants.len(), 2);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_single_variant_enum() {
        let prog = parse("enum Single { OnlyOne }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 1);
                assert_eq!(e.variants[0].name, "OnlyOne");
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_many_variants() {
        let prog = parse("enum Status { New, Pending, Active, Inactive, Deleted, Archived }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 6);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_variant_with_multiple_payload_types() {
        let prog = parse("enum Event { Click(Int, Int), Scroll(Float) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 2);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_public_enum_uppercase() {
        let prog = parse("enum Option { Some, None }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.name, "Option");
                assert!(e.is_public);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_with_string_payload() {
        let prog = parse("enum Message { Text(Str) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 1);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_with_int_payload() {
        let prog = parse("enum Number { Value(Int) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 1);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_with_bool_payload() {
        let prog = parse("enum Flag { State(Bool) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 1);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_mixed_variants() {
        let prog = parse("enum Mixed { Empty, WithData(Int), WithTwo(Str, Bool) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 3);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_with_array_payload() {
        let prog = parse("enum Data { Items([Int]) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 1);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_with_optional_payload() {
        let prog = parse("enum Maybe { Value(Int?) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 1);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_with_custom_type_payload() {
        let prog = parse("enum Container { UserData(User) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 1);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_multiline() {
        let prog = parse("enum Color {\n  Red,\n  Green,\n  Blue,\n}");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 3);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_trailing_comma() {
        let prog = parse("enum Color { Red, Green, Blue, }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 3);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_with_decorators() {
        let prog = parse("@tagged enum Message { Text, Binary }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 2);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_variant_three_payloads() {
        let prog = parse("enum Triple { Data(Int, Str, Bool) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 1);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_two_variants() {
        let prog = parse("enum Binary { Left, Right }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 2);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_four_variants() {
        let prog = parse("enum Direction { North, South, East, West }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 4);
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_enum_with_map_payload() {
        let prog = parse("enum Data { Config({Str: Int}) }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants.len(), 1);
            }
            _ => panic!("Expected enum"),
        }
    }

    // ========================================
    // Import Declarations (20 tests)
    // ========================================

    #[test]
    fn test_simple_import() {
        let prog = parse("import std::io");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path.len(), 2);
                assert_eq!(i.path[0], "std");
                assert_eq!(i.path[1], "io");
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_with_items() {
        let prog = parse("import std::io { File, Reader }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert!(!i.items.is_empty());
                assert_eq!(i.items.len(), 2);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_wildcard_import() {
        let prog = parse("import std::io::*");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert!(i.wildcard);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_aliased_import() {
        let prog = parse("import std::io as io");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert!(i.alias.is_some());
                assert_eq!(i.alias.as_ref().unwrap(), "io");
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_multi_segment_path() {
        let prog = parse("import a::b::c::d");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path.len(), 4);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_with_alias_items() {
        let prog = parse("import std { File as F }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert!(!i.items.is_empty());
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_single_segment() {
        let prog = parse("import http");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path.len(), 1);
                assert_eq!(i.path[0], "http");
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_three_items() {
        let prog = parse("import std::io { File, Reader, Writer }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.items.len(), 3);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_single_item() {
        let prog = parse("import std::io { File }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.items.len(), 1);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_nested_module() {
        let prog = parse("import std::collections::map");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path.len(), 3);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_deep_nesting() {
        let prog = parse("import a::b::c::d::e::f");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path.len(), 6);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_many_items() {
        let prog = parse("import std { File, Reader, Writer, Buffer, Stream }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.items.len(), 5);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_with_trailing_comma() {
        let prog = parse("import std::io { File, Reader, }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.items.len(), 2);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_multiline_items() {
        let prog = parse("import std::io {\n  File,\n  Reader,\n  Writer,\n}");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.items.len(), 3);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_http_module() {
        let prog = parse("import http");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path.len(), 1);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_db_module() {
        let prog = parse("import db");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path.len(), 1);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_json_module() {
        let prog = parse("import json");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path.len(), 1);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_auth_module() {
        let prog = parse("import auth");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path.len(), 1);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_two_segments() {
        let prog = parse("import std::fs");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path.len(), 2);
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_import_five_segments() {
        let prog = parse("import a::b::c::d::e");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path.len(), 5);
            }
            _ => panic!("Expected import"),
        }
    }

    // ========================================
    // Let Statements (25 tests)
    // ========================================

    #[test]
    fn test_simple_let() {
        let prog = parse("fn main() { let x = 42 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                assert_eq!(body.len(), 1);
                match &body[0].kind {
                    StmtKind::Let {
                        mutable, pattern, ..
                    } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "x");
                        } else {
                            panic!("Expected ident pattern");
                        }
                        assert!(!mutable);
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_type_annotation() {
        let prog = parse("fn main() { let x: Int = 42 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { type_ann, .. } => {
                        assert!(type_ann.is_some());
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_mutable_let() {
        let prog = parse("fn main() { let mut x = 42 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let {
                        mutable, pattern, ..
                    } => {
                        assert!(mutable);
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "x");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_string_value() {
        let prog = parse("fn main() { let s = \"hello\" }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "s");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_bool_value() {
        let prog = parse("fn main() { let b = true }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "b");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_array_literal() {
        let prog = parse("fn main() { let arr = [1, 2, 3] }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "arr");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_map_literal() {
        let prog = parse("fn main() { let m = Map{} }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "m");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_struct_literal() {
        let prog = parse("fn main() { let u = User{} }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "u");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_function_call_value() {
        let prog = parse("fn main() { let result = getValue() }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "result");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_binary_expression_value() {
        let prog = parse("fn main() { let sum = a + b }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "sum");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_nil() {
        let prog = parse("fn main() { let x = nil }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "x");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_multiple_vars() {
        let prog = parse("fn main() { let x = 1;\n let y = 2 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                assert_eq!(body.len(), 2);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_float_value() {
        let prog = parse("fn main() { let pi = 3.14 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "pi");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_negative_value() {
        let prog = parse("fn main() { let neg = -42 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "neg");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_optional_type() {
        let prog = parse("fn main() { let x: Int? = nil }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { type_ann, .. } => {
                        assert!(type_ann.is_some());
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_array_type() {
        let prog = parse("fn main() { let arr: [Int] = [] }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { type_ann, .. } => {
                        assert!(type_ann.is_some());
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_map_type() {
        let prog = parse("fn main() { let m: {Str: Int} = {} }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { type_ann, .. } => {
                        assert!(type_ann.is_some());
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_custom_type() {
        let prog = parse("fn main() { let user: User = User{} }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { type_ann, .. } => {
                        assert!(type_ann.is_some());
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_multiplication() {
        let prog = parse("fn main() { let product = x * y }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "product");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_division() {
        let prog = parse("fn main() { let quotient = x / y }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "quotient");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_modulo() {
        let prog = parse("fn main() { let remainder = x % y }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "remainder");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_comparison() {
        let prog = parse("fn main() { let result = x > y }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "result");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_logical_and() {
        let prog = parse("fn main() { let result = a && b }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "result");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_logical_or() {
        let prog = parse("fn main() { let result = a || b }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "result");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_let_with_field_access() {
        let prog = parse("fn main() { let name = user.name }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Let { pattern, .. } => {
                        if let PatternKind::Ident(name) = &pattern.kind {
                            assert_eq!(name, "name");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected let statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    // ========================================
    // Control Flow Statements (25 tests)
    // ========================================

    #[test]
    fn test_if_statement() {
        let prog = parse("fn main() { if x > 0 { print(x) } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { .. } => {}
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_if_else_statement() {
        let prog = parse("fn main() { if x > 0 { print(\"pos\") } else { print(\"neg\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { else_branch, .. } => {
                        assert!(else_branch.is_some());
                    }
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_if_else_if_else_chain() {
        let prog = parse("fn main() { if x > 0 { print(\"pos\") } else if x < 0 { print(\"neg\") } else { print(\"zero\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { else_branch, .. } => {
                        assert!(else_branch.is_some());
                    }
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_for_loop() {
        let prog = parse("fn main() { for i in 1..10 { print(i) } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::For { .. } => {}
                    _ => panic!("Expected for statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_for_in_with_array() {
        let prog = parse("fn main() { for item in items { print(item) } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::For { pattern, .. } => {
                        if let PatternKind::Ident(var) = &pattern.kind {
                            assert_eq!(var, "item");
                        } else {
                            panic!("Expected ident pattern");
                        }
                    }
                    _ => panic!("Expected for statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_return_with_value() {
        let prog = parse("fn main() -> Int { return 42 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Return(values) => {
                        assert!(!values.is_empty());
                    }
                    _ => panic!("Expected return statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_return_without_value() {
        let prog = parse("fn main() { return }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Return(values) => {
                        assert!(values.is_empty());
                    }
                    _ => panic!("Expected return statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_break_statement() {
        let prog = parse("fn main() { for i in 1..10 { break } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::For { body: for_body, .. } => match &for_body[0].kind {
                        StmtKind::Break => {}
                        _ => panic!("Expected break statement"),
                    },
                    _ => panic!("Expected for statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_continue_statement() {
        let prog = parse("fn main() { for i in 1..10 { continue } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::For { body: for_body, .. } => match &for_body[0].kind {
                        StmtKind::Continue => {}
                        _ => panic!("Expected continue statement"),
                    },
                    _ => panic!("Expected for statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_nested_if_in_for() {
        let prog = parse("fn main() { for i in 1..10 { if i > 5 { print(i) } } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::For { body: for_body, .. } => match &for_body[0].kind {
                        StmtKind::If { .. } => {}
                        _ => panic!("Expected if statement"),
                    },
                    _ => panic!("Expected for statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_nested_for_in_if() {
        let prog = parse("fn main() { if true { for i in 1..10 { print(i) } } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { then_block, .. } => match &then_block[0].kind {
                        StmtKind::For { .. } => {}
                        _ => panic!("Expected for statement"),
                    },
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_if_with_bool_literal() {
        let prog = parse("fn main() { if true { print(\"yes\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { .. } => {}
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_if_with_false_literal() {
        let prog = parse("fn main() { if false { print(\"no\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { .. } => {}
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_if_with_equality() {
        let prog = parse("fn main() { if x == 10 { print(\"ten\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { .. } => {}
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_if_with_inequality() {
        let prog = parse("fn main() { if x != 10 { print(\"not ten\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { .. } => {}
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_if_with_less_than() {
        let prog = parse("fn main() { if x < 10 { print(\"small\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { .. } => {}
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_if_with_greater_than_or_equal() {
        let prog = parse("fn main() { if x >= 10 { print(\"big\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { .. } => {}
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_if_with_less_than_or_equal() {
        let prog = parse("fn main() { if x <= 10 { print(\"small or equal\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { .. } => {}
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_if_with_logical_and() {
        let prog = parse("fn main() { if x > 0 && x < 10 { print(\"range\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { .. } => {}
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_if_with_logical_or() {
        let prog = parse("fn main() { if x < 0 || x > 10 { print(\"out of range\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { .. } => {}
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_for_with_range() {
        let prog = parse("fn main() { for i in 0..100 { print(i) } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::For { .. } => {}
                    _ => panic!("Expected for statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_for_with_inclusive_range() {
        let prog = parse("fn main() { for i in 0..=100 { print(i) } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::For { .. } => {}
                    _ => panic!("Expected for statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_return_expression() {
        let prog = parse("fn main() -> Int { return x + y }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Return(values) => {
                        assert!(!values.is_empty());
                    }
                    _ => panic!("Expected return statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_return_function_call() {
        let prog = parse("fn main() -> Int { return getValue() }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Return(values) => {
                        assert!(!values.is_empty());
                    }
                    _ => panic!("Expected return statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_if_else_if_chain() {
        let prog =
            parse("fn main() { if x > 10 { print(\"big\") } else if x > 5 { print(\"medium\") } }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::If { else_branch, .. } => {
                        assert!(else_branch.is_some());
                    }
                    _ => panic!("Expected if statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    // ========================================
    // Assignment Statements (20 tests)
    // ========================================

    #[test]
    fn test_simple_assign() {
        let prog = parse("fn main() { x = 42 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Assign { .. } => {}
                    _ => panic!("Expected assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_compound_assign_add() {
        let prog = parse("fn main() { x += 1 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::CompoundAssign { .. } => {}
                    _ => panic!("Expected compound assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_compound_assign_subtract() {
        let prog = parse("fn main() { x -= 1 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::CompoundAssign { .. } => {}
                    _ => panic!("Expected compound assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_compound_assign_multiply() {
        let prog = parse("fn main() { x *= 2 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::CompoundAssign { .. } => {}
                    _ => panic!("Expected compound assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_compound_assign_divide() {
        let prog = parse("fn main() { x /= 2 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::CompoundAssign { .. } => {}
                    _ => panic!("Expected compound assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_compound_assign_modulo() {
        let prog = parse("fn main() { x %= 3 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::CompoundAssign { .. } => {}
                    _ => panic!("Expected compound assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_increment() {
        let prog = parse("fn main() { x++ }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::IncDec { .. } => {}
                    _ => panic!("Expected increment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_decrement() {
        let prog = parse("fn main() { x-- }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::IncDec { .. } => {}
                    _ => panic!("Expected decrement statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_field_assign() {
        let prog = parse("fn main() { obj.field = 42 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Assign { .. } => {}
                    _ => panic!("Expected assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_index_assign() {
        let prog = parse("fn main() { arr[0] = 42 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Assign { .. } => {}
                    _ => panic!("Expected assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_assign_string() {
        let prog = parse("fn main() { s = \"hello\" }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Assign { .. } => {}
                    _ => panic!("Expected assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_assign_bool() {
        let prog = parse("fn main() { flag = true }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Assign { .. } => {}
                    _ => panic!("Expected assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_assign_array() {
        let prog = parse("fn main() { arr = [1, 2, 3] }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Assign { .. } => {}
                    _ => panic!("Expected assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_assign_function_call() {
        let prog = parse("fn main() { result = getValue() }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Assign { .. } => {}
                    _ => panic!("Expected assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_assign_expression() {
        let prog = parse("fn main() { result = x + y * z }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Assign { .. } => {}
                    _ => panic!("Expected assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_nested_field_assign() {
        let prog = parse("fn main() { user.address.city = \"NYC\" }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Assign { .. } => {}
                    _ => panic!("Expected assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_nested_index_assign() {
        let prog = parse("fn main() { matrix[0][1] = 42 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Assign { .. } => {}
                    _ => panic!("Expected assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_compound_assign_with_expression() {
        let prog = parse("fn main() { x += y * 2 }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::CompoundAssign { .. } => {}
                    _ => panic!("Expected compound assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_assign_nil() {
        let prog = parse("fn main() { x = nil }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Assign { .. } => {}
                    _ => panic!("Expected assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_assign_struct_literal() {
        let prog = parse("fn main() { user = User{} }");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            Item::Function(f) => {
                let body = &f.body;
                match &body[0].kind {
                    StmtKind::Assign { .. } => {}
                    _ => panic!("Expected assignment statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    // ========================================
    // Error Cases (25 tests)
    // ========================================

    #[test]
    fn test_missing_closing_brace() {
        let (_, errors) = parse_with_errors("fn main() { let x = 42");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_closing_paren() {
        let (_, errors) = parse_with_errors("fn add(a: Int, b: Int { return a + b }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_closing_bracket() {
        let (_, errors) = parse_with_errors("fn main() { let arr = [1, 2, 3 }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_unexpected_token() {
        let (_, errors) = parse_with_errors("fn main() { @@@@ }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_function_name() {
        let (_, errors) = parse_with_errors("fn () { }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_struct_name() {
        let (_, errors) = parse_with_errors("struct { name: Str }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_empty_source() {
        let prog = parse("");
        assert_eq!(prog.items.len(), 0);
    }

    #[test]
    fn test_only_whitespace() {
        let prog = parse("   \n   \n   ");
        assert_eq!(prog.items.len(), 0);
    }

    #[test]
    fn test_missing_type_annotation_after_colon() {
        let (_, errors) = parse_with_errors("fn main() { let x: = 42 }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_invalid_decorator_syntax() {
        let (_, errors) = parse_with_errors("@@@route fn handler() { }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_function_body() {
        let (_, errors) = parse_with_errors("fn main()");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_struct_body() {
        let (_, errors) = parse_with_errors("struct User");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_enum_body() {
        let (_, errors) = parse_with_errors("enum Color");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_invalid_parameter_syntax() {
        let (_, errors) = parse_with_errors("fn add(a Int, b: Int) { }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_arrow_in_function() {
        let (_, errors) = parse_with_errors("fn main() Int { return 42 }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_import_path() {
        let (_, errors) = parse_with_errors("import");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_invalid_field_syntax() {
        let (_, errors) = parse_with_errors("struct User { name Str }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_equals_in_let() {
        let (_, errors) = parse_with_errors("fn main() { let x 42 }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_if_condition() {
        let (_, errors) = parse_with_errors("fn main() { if { print(\"hi\") } }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_for_variable() {
        let (_, errors) = parse_with_errors("fn main() { for in 1..10 { } }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_missing_for_iterator() {
        // `for i { }` is valid in Doo (infinite loop with named pattern, no iterable)
        // Test truly invalid syntax: missing variable after `for` with `in`
        let (_, errors) = parse_with_errors("fn main() { for in items { } }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_unmatched_opening_brace() {
        let (_, errors) = parse_with_errors("fn main() { { { let x = 42 }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_invalid_return_type_syntax() {
        let (_, errors) = parse_with_errors("fn main() -> { return 42 }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_invalid_enum_variant() {
        let (_, errors) = parse_with_errors("enum Color { Red, , Blue }");
        assert!(errors.len() > 0);
    }

    #[test]
    fn test_multiple_errors() {
        let (_, errors) = parse_with_errors("fn main( { let x: = @@@ }");
        assert!(errors.len() > 0);
    }
}
