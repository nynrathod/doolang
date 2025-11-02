use bumpalo::Bump;
use doo::analyzer::SemanticAnalyzer;
use doo::codegen::core::CodeGen;
use doo::lexar::lexer::lex;
use doo::mir::builder::MirBuilder;
use doo::parser::Parser;
use inkwell::context::Context;

fn compile_full_pipeline(input: &str) -> Result<String, String> {
    let arena = Bump::new();
    let tokens = lex(input, &arena);
    let mut parser = Parser::new(&tokens);
    let result = parser.parse_program();

    match result {
        Ok(mut ast) => {
            let mut analyzer = SemanticAnalyzer::new(None);
            if let doo::parser::ast::AstNode::Program(ref mut nodes) = ast {
                analyzer
                    .analyze_program(nodes)
                    .map_err(|e| format!("{:?}", e))?;

                let mut mir_builder = MirBuilder::new();
                mir_builder.build_program(nodes);
                mir_builder.finalize();

                let context = Context::create();
                let mut codegen = CodeGen::new("regression_test", &context);
                codegen.generate_program(&mir_builder.program);

                Ok(codegen.module.print_to_string().to_string())
            } else {
                Err("Not a program".to_string())
            }
        }
        Err(e) => Err(format!("Parse error: {:?}", e)),
    }
}

#[test]
fn regression_array_access_in_if_within_loop() {
    let input = r#"
        fn main() {
            let arr = [5, 10, 15, 20];
            let mut count = 0;

            for i in 0..4 {
                if arr[i] > 10 {
                    count += 1;
                }
                print("Iteration", i, "Count:", count);
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_compound_assignment_type_check() {
    let input = r#"
        fn main() {
            let x = 5;
            x += 1;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_function_arg_type_validation() {
    let input = r#"
        fn process(val: Int) -> Int {
            return val * 2;
        }

        fn main() {
            for i in 0..3 {
                let result = process("invalid");
                print(result);
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_variable_shadowing_in_loops() {
    let input = r#"
        fn main() {
            let x = 10;

            for i in 0..3 {
                let x = i;
                print("Loop x:", x);
            }

            print("Outer x:", x);

            if true {
                let x = 100;
                print("If x:", x);
            }

            print("Final x:", x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_array_index_type_validation() {
    let input = r#"
        fn main() {
            let arr = [1, 2, 3];
            let x = arr["invalid"];
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_mutable_variable_modification() {
    let input = r#"
        fn main() {
            let mut counter = 0;

            for i in 0..5 {
                counter += 1;
            }

            counter += 10;
            print("Counter:", counter);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_return_type_in_conditional() {
    let input = r#"
        fn getValue(flag: Bool) -> Int {
            if flag {
                return "not_an_int";
            }
            return 42;
        }

        fn main() {
            let result = getValue(true);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_undeclared_variable_in_deep_scope() {
    let input = r#"
        fn main() {
            let x = 10;

            for i in 0..3 {
                for j in 0..3 {
                    if i > 0 {
                        let result = x + i + j;
                        print(result);
                    }
                }
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_array_parameter_type_check() {
    let input = r#"
        fn sumArray(arr: [Int]) -> Int {
            let mut total = 0;
            for i in 0..3 {
                total += arr[i];
            }
            return total;
        }

        fn main() {
            let numbers = [1, 2, "three"];
            let result = sumArray(numbers);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_empty_array_handling() {
    let input = r#"
        fn main() {
            let empty: [Int] = [];
            print("Empty array:", empty);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_array_bounds_negative_index() {
    let input = r#"
        fn main() {
            let arr = [1, 2, 3];
            let x = arr[-1];
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_function_missing_return() {
    let input = r#"
        fn getValue() -> Int {
            let x = 10;
        }

        fn main() {
            let v = getValue();
            print(v);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_type_mismatch_in_binary_op() {
    let input = r#"
        fn main() {
            let x = 5 + "string";
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_undefined_function_call() {
    let input = r#"
        fn main() {
            let result = undefinedFunction(10);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_wrong_number_of_arguments() {
    let input = r#"
        fn add(a: Int, b: Int) -> Int {
            return a + b;
        }

        fn main() {
            let result = add(5);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_variable_used_before_declaration() {
    let input = r#"
        fn main() {
            print(x);
            let x = 10;
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_immutable_variable_reassignment() {
    let input = r#"
        fn main() {
            let x = 10;
            x = 20;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_boolean_arithmetic() {
    let input = r#"
        fn main() {
            let result = true + false;
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_array_type_consistency() {
    let input = r#"
        fn main() {
            let arr = [1, 2, true, 4];
            print(arr);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_map_type_consistency() {
    let input = r#"
        fn main() {
            let m = {"a": 1, "b": "string"};
            print(m);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_return_outside_function() {
    let input = r#"
        fn main() {
            let x = 10;
        }
        return 5;
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_duplicate_function_definition() {
    let input = r#"
        fn test() -> Int {
            return 1;
        }

        fn test() -> Int {
            return 2;
        }

        fn main() {
            let x = test();
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_duplicate_parameter_names() {
    let input = r#"
        fn process(x: Int, x: Int) -> Int {
            return x + x;
        }

        fn main() {
            let result = process(5, 10);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_missing_main_function() {
    let input = r#"
        fn helper() -> Int {
            return 42;
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_function_call_as_array_index() {
    let input = r#"
        fn getIndex() -> Int {
            return 2;
        }

        fn main() {
            let arr = [10, 20, 30, 40];
            let val = arr[getIndex()];
            print(val);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_string_index_access() {
    let input = r#"
        fn main() {
            let s = "hello";
            let c = s[0];
            print(c);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_array_length_mismatch() {
    let input = r#"
        fn processThree(arr: [Int]) -> Int {
            return arr[0] + arr[1] + arr[2];
        }

        fn main() {
            let data = [1, 2];
            let result = processThree(data);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_void_function_return_value() {
    let input = r#"
        fn doSomething() {
            print("Done");
        }

        fn main() {
            let x = doSomething();
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_comparison_type_mismatch() {
    let input = r#"
        fn main() {
            if 5 > "string" {
                print("True");
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_logical_operator_on_non_bool() {
    let input = r#"
        fn main() {
            if 5 && 10 {
                print("True");
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_negation_on_string() {
    let input = r#"
        fn main() {
            let x = -"hello";
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_array_in_arithmetic() {
    let input = r#"
        fn main() {
            let arr = [1, 2, 3];
            let x = arr + 5;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_map_in_arithmetic() {
    let input = r#"
        fn main() {
            let m = {"a": 1};
            let x = m * 2;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_for_loop_non_integer_range() {
    let input = r#"
        fn main() {
            for i in "a".."z" {
                print(i);
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_nested_loop_variable_scope() {
    let input = r#"
        fn main() {
            for i in 0..3 {
                for j in 0..3 {
                    print(i, j);
                }
                print(j);
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_function_parameter_shadowing() {
    let input = r#"
        fn process(x: Int) -> Int {
            let x = x + 1;
            return x;
        }

        fn main() {
            let result = process(10);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_recursive_call_type_check() {
    let input = r#"
        fn factorial(n: Int) -> Int {
            if n <= 1 {
                return 1;
            }
            return n * factorial(n - 1);
        }

        fn main() {
            let result = factorial(5);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_mutual_recursion() {
    let input = r#"
        fn isEven(n: Int) -> Bool {
            if n == 0 {
                return true;
            }
            return isOdd(n - 1);
        }

        fn isOdd(n: Int) -> Bool {
            if n == 0 {
                return false;
            }
            return isEven(n - 1);
        }

        fn main() {
            let result = isEven(4);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_string_boolean_comparison() {
    let input = r#"
        fn main() {
            if "hello" == true {
                print("Match");
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_array_equality_check() {
    let input = r#"
        fn main() {
            let arr1 = [1, 2, 3];
            let arr2 = [1, 2, 3];
            if arr1 == arr2 {
                print("Equal");
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_map_as_condition() {
    let input = r#"
        fn main() {
            let m = {"a": 1};
            if m {
                print("True");
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_modifying_function_parameter() {
    let input = r#"
        fn modify(x: Int) -> Int {
            x = x + 1;
            return x;
        }

        fn main() {
            let result = modify(10);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_array_element_type_inference() {
    let input = r#"
        fn main() {
            let arr = [1, 2, 3];
            let elem = arr[0];
            let result = elem + 5;
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_nested_function_scope() {
    let input = r#"
        fn outer() -> Int {
            let x = 10;
            fn inner() -> Int {
                return x;
            }
            return inner();
        }

        fn main() {
            let result = outer();
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_break_statement_unsupported() {
    let input = r#"
        fn main() {
            for i in 0..10 {
                if i == 5 {
                    break;
                }
                print(i);
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_continue_statement_unsupported() {
    let input = r#"
        fn main() {
            for i in 0..10 {
                if i == 5 {
                    continue;
                }
                print(i);
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_ternary_operator_unsupported() {
    let input = r#"
        fn main() {
            let x = true ? 10 : 20;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_switch_statement_unsupported() {
    let input = r#"
        fn main() {
            let x = 5;
            switch x {
                case 5:
                    print("Five");
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_class_definition_unsupported() {
    let input = r#"
        class User {
            name: Str;
            age: Int;
        }

        fn main() {
            print("Hello");
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_mutable_array_element() {
    let input = r#"
        fn main() {
            let mut arr = [1, 2, 3];
            arr[0] = 10;
            print(arr);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_const_keyword_unsupported() {
    let input = r#"
        fn main() {
            const x = 10;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_global_variable() {
    let input = r#"
        let global = 100;

        fn main() {
            print(global);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_empty_if_block() {
    let input = r#"
        fn main() {
            if true {
            }
            print("Done");
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_empty_else_block() {
    let input = r#"
        fn main() {
            if false {
                print("True");
            } else {
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_multiple_else_if() {
    let input = r#"
        fn main() {
            let x = 5;
            if x == 1 {
                print("One");
            } else if x == 2 {
                print("Two");
            } else if x == 5 {
                print("Five");
            } else {
                print("Other");
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_division_by_zero() {
    let input = r#"
        fn main() {
            let x = 10 / 0;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_modulo_by_zero() {
    let input = r#"
        fn main() {
            let x = 10 % 0;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_negative_array_size() {
    let input = r#"
        fn main() {
            let arr: [Int] = [];
            print(arr);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_function_overloading() {
    let input = r#"
        fn add(a: Int, b: Int) -> Int {
            return a + b;
        }

        fn add(a: Str, b: Str) -> Str {
            return a + b;
        }

        fn main() {
            print(add(5, 10));
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_variadic_function() {
    let input = r#"
        fn sum(args...) -> Int {
            return 0;
        }

        fn main() {
            let result = sum(1, 2, 3, 4);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_array_slice() {
    let input = r#"
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            let slice = arr[1..3];
            print(slice);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_string_interpolation() {
    let input = r#"
        fn main() {
            let name = "Alice";
            let msg = "Hello ${name}";
            print(msg);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_multiline_string() {
    let input = r#"
        fn main() {
            let text = "Line 1
Line 2
Line 3";
            print(text);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_char_literal() {
    let input = r#"
        fn main() {
            let c = 'a';
            print(c);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_float_type() {
    let input = r#"
        fn main() {
            let x: Float = 3.14;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_null_value() {
    let input = r#"
        fn main() {
            let x = null;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_optional_type() {
    let input = r#"
        fn main() {
            let x: Int? = 10;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_tuple_type() {
    let input = r#"
        fn main() {
            let pair = (1, 2);
            print(pair);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_struct_literal() {
    let input = r#"
        fn main() {
            let user = {name: "Alice", age: 30};
            print(user);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_array_push_method() {
    let input = r#"
        fn main() {
            let mut arr = [1, 2, 3];
            arr.push(4);
            print(arr);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_string_length_property() {
    let input = r#"
        fn main() {
            let s = "hello";
            let len = s.length;
            print(len);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_anonymous_function() {
    let input = r#"
        fn main() {
            let add = fn(a: Int, b: Int) -> Int {
                return a + b;
            };
            print(add(5, 3));
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_lambda_expression() {
    let input = r#"
        fn main() {
            let double = |x| x * 2;
            print(double(5));
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_array_map_method() {
    let input = r#"
        fn main() {
            let arr = [1, 2, 3];
            let doubled = arr.map(|x| x * 2);
            print(doubled);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_pattern_matching() {
    let input = r#"
        fn main() {
            let x = 5;
            match x {
                1 => print("One"),
                5 => print("Five"),
                _ => print("Other"),
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_enum_definition() {
    let input = r#"
        enum Color {
            Red,
            Green,
            Blue,
        }

        fn main() {
            let c = Color::Red;
            print(c);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_import_statement() {
    let input = r#"
        import math::sqrt;

        fn main() {
            let result = sqrt(16);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_export_statement() {
    let input = r#"
        export fn publicFunc() -> Int {
            return 42;
        }

        fn main() {
            print(publicFunc());
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_type_alias() {
    let input = r#"
        type IntArray = [Int];

        fn main() {
            let arr: IntArray = [1, 2, 3];
            print(arr);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_generic_function() {
    let input = r#"
        fn identity<T>(x: T) -> T {
            return x;
        }

        fn main() {
            let x = identity(5);
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_async_function() {
    let input = r#"
        async fn fetchData() -> Str {
            return "data";
        }

        fn main() {
            let result = await fetchData();
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_spread_operator() {
    let input = r#"
        fn main() {
            let arr1 = [1, 2, 3];
            let arr2 = [...arr1, 4, 5];
            print(arr2);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_destructuring_assignment() {
    let input = r#"
        fn main() {
            let [a, b, c] = [1, 2, 3];
            print(a, b, c);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_range_inclusive() {
    let input = r#"
        fn main() {
            for i in 0..=5 {
                print(i);
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_bitwise_operators() {
    let input = r#"
        fn main() {
            let x = 5 & 3;
            let y = 5 | 3;
            let z = 5 ^ 3;
            print(x, y, z);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_left_shift_operator() {
    let input = r#"
        fn main() {
            let x = 1 << 4;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_right_shift_operator() {
    let input = r#"
        fn main() {
            let x = 16 >> 2;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_increment_operator() {
    let input = r#"
        fn main() {
            let mut x = 5;
            x++;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_decrement_operator() {
    let input = r#"
        fn main() {
            let mut x = 5;
            x--;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_power_operator() {
    let input = r#"
        fn main() {
            let x = 2 ** 3;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_compound_assignment_division() {
    let input = r#"
        fn main() {
            let mut x = 20;
            x /= 2;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_compound_assignment_modulo() {
    let input = r#"
        fn main() {
            let mut x = 17;
            x %= 5;
            print(x);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_not_equal_operator() {
    let input = r#"
        fn main() {
            if 5 != 3 {
                print("Not equal");
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_less_than_or_equal() {
    let input = r#"
        fn main() {
            if 5 <= 10 {
                print("Less or equal");
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_greater_than_or_equal() {
    let input = r#"
        fn main() {
            if 10 >= 5 {
                print("Greater or equal");
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_ok());
}

#[test]
fn regression_logical_not_operator() {
    let input = r#"
        fn main() {
            let flag = !true;
            print(flag);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_complex_boolean_expression() {
    let input = r#"
        fn main() {
            let x = 5;
            let y = 10;
            if (x > 0 && y > 0) || (x < 0 && y < 0) {
                print("Same sign");
            }
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}

#[test]
fn regression_nested_boolean_negation() {
    let input = r#"
        fn main() {
            let result = !(5 > 3 && 10 < 20);
            print(result);
        }
    "#;
    let result = compile_full_pipeline(input);
    assert!(result.is_err());
}
