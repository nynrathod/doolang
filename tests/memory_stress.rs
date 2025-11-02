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
                let mut codegen = CodeGen::new("test", &context);
                codegen.generate_program(&mir_builder.program);

                Ok(codegen.module.print_to_string().to_string())
            } else {
                Err("Not a program".to_string())
            }
        }
        Err(e) => Err(format!("Parse error: {:?}", e)),
    }
}

// =====================================================================
// Memory Management Tests
// =====================================================================

#[test]
fn mem_simple_array_allocation() {
    let input = r#"
        fn main() {
            let arr: [Int] = [1, 2, 3, 4, 5];
            print("Array:", arr);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_in_function() {
    let input = r#"
        fn createArray() -> [Int] {
            let arr: [Int] = [10, 20, 30];
            return arr;
        }

        fn main() {
            let result = createArray();
            print("Result:", result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_parameter_passing() {
    let input = r#"
        fn sumArray(arr: [Int]) -> Int {
            let mut total = 0;
            for i in 0..5 {
                total += arr[i];
            }
            return total;
        }

        fn main() {
            let numbers: [Int] = [5, 10, 15, 20, 25];
            let sum = sumArray(numbers);
            print("Sum:", sum);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_return_from_function() {
    let input = r#"
        fn createData() -> [Int] {
            return [1, 2, 3, 4, 5];
        }

        fn main() {
            let data = createData();
            print("Data:", data);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_multiple_arrays_in_scope() {
    let input = r#"
        fn main() {
            let arr1: [Int] = [1, 2, 3];
            let arr2: [Int] = [4, 5, 6];
            let arr3: [Int] = [7, 8, 9];
            print("Arrays allocated");
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_in_loop_creation() {
    let input = r#"
        fn main() {
            for i in 0..5 {
                let temp: [Int] = [i, i + 1, i + 2];
                print("Iteration:", i);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_nested_function_array_passing() {
    let input = r#"
        fn process(arr: [Int]) -> Int {
            let mut sum = 0;
            for i in 0..3 {
                sum += arr[i];
            }
            return sum;
        }

        fn wrapper(arr: [Int]) -> Int {
            return process(arr);
        }

        fn main() {
            let data: [Int] = [10, 20, 30];
            let result = wrapper(data);
            print("Result:", result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_iteration_cleanup() {
    let input = r#"
        fn main() {
            let arr: [Int] = [1, 2, 3, 4, 5];
            for val in arr {
                print("Value:", val);
            }
            print("Done");
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_string_allocation() {
    let input = r#"
        fn main() {
            let str1: Str = "Hello";
            let str2: Str = "World";
            let combined = str1 + " " + str2;
            print(combined);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_string_in_function() {
    let input = r#"
        fn greet(name: Str) -> Str {
            return "Hello, " + name;
        }

        fn main() {
            let greeting = greet("Alice");
            print(greeting);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_map_allocation() {
    let input = r#"
        fn main() {
            let m: {Str: Int} = {"a": 1, "b": 2, "c": 3};
            print("Map:", m);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_map_iteration() {
    let input = r#"
        fn main() {
            let m: {Str: Int} = {"x": 10, "y": 20, "z": 30};
            for (key, value) in m {
                print("Key:", key, "Value:", value);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_map_in_function() {
    let input = r#"
        fn createMap() -> {Str: Int} {
            let m: {Str: Int} = {"a": 100, "b": 200};
            return m;
        }

        fn main() {
            let result = createMap();
            print("Map:", result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_large_array_allocation() {
    let input = r#"
        fn main() {
            let arr: [Int] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20];
            print("Large array created");
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_multiple_string_operations() {
    let input = r#"
        fn main() {
            let s1 = "Hello";
            let s2 = "World";
            let s3 = s1 + " " + s2;
            let s4 = s3 + "!";
            print(s4);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_string_in_loop() {
    let input = r#"
        fn main() {
            for i in 0..5 {
                let msg = "Iteration " + "number";
                print(msg, i);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_function_returning_string() {
    let input = r#"
        fn getMessage() -> Str {
            return "Dynamic message";
        }

        fn main() {
            let msg = getMessage();
            print(msg);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_passed_multiple_times() {
    let input = r#"
        fn processArray(arr: [Int]) -> Int {
            let mut sum = 0;
            for i in 0..3 {
                sum += arr[i];
            }
            return sum;
        }

        fn main() {
            let data: [Int] = [10, 20, 30];
            let result1 = processArray(data);
            let result2 = processArray(data);
            print(result1, result2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_nested_loops_with_arrays() {
    let input = r#"
        fn main() {
            for i in 0..3 {
                let arr: [Int] = [i, i + 1, i + 2];
                for j in 0..3 {
                    print(arr[j]);
                }
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_map_with_string_keys() {
    let input = r#"
        fn main() {
            let config: {Str: Int} = {"width": 800, "height": 600, "fps": 60};
            print("Config:", config);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_map_parameter_passing() {
    let input = r#"
        fn getWidth(config: {Str: Int}) -> Int {
            return 800;
        }

        fn main() {
            let cfg: {Str: Int} = {"width": 1024, "height": 768};
            let w = getWidth(cfg);
            print("Width:", w);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_multiple_function_calls_with_arrays() {
    let input = r#"
        fn double(arr: [Int]) -> [Int] {
            let result: [Int] = [arr[0] * 2, arr[1] * 2, arr[2] * 2];
            return result;
        }

        fn triple(arr: [Int]) -> [Int] {
            let result: [Int] = [arr[0] * 3, arr[1] * 3, arr[2] * 3];
            return result;
        }

        fn main() {
            let data: [Int] = [1, 2, 3];
            let d = double(data);
            let t = triple(data);
            print(d, t);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_string_concatenation_in_function() {
    let input = r#"
        fn buildMessage(part1: Str, part2: Str) -> Str {
            return part1 + " " + part2;
        }

        fn main() {
            let msg = buildMessage("Hello", "World");
            print(msg);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_in_conditional() {
    let input = r#"
        fn main() {
            let flag = true;
            if flag {
                let arr: [Int] = [1, 2, 3];
                print("Array:", arr);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_multiple_scopes_with_arrays() {
    let input = r#"
        fn main() {
            let arr1: [Int] = [1, 2, 3];
            if true {
                let arr2: [Int] = [4, 5, 6];
                print(arr2);
            }
            print(arr1);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_deep_nested_function_calls() {
    let input = r#"
        fn level3(x: Int) -> Int {
            return x * 3;
        }

        fn level2(x: Int) -> Int {
            return level3(x) + 10;
        }

        fn level1(x: Int) -> Int {
            return level2(x) * 2;
        }

        fn main() {
            let result = level1(5);
            print("Result:", result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_with_expressions() {
    let input = r#"
        fn main() {
            let x = 10;
            let arr: [Int] = [x, x + 5, x + 10, x + 15];
            print("Array:", arr);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_string_array_allocation() {
    let input = r#"
        fn main() {
            let names: [Str] = ["Alice", "Bob", "Charlie"];
            print("Names:", names);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_bool_array_allocation() {
    let input = r#"
        fn main() {
            let flags: [Bool] = [true, false, true, false];
            print("Flags:", flags);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_map_with_bool_values() {
    let input = r#"
        fn main() {
            let settings: {Str: Bool} = {"enabled": true, "visible": false};
            print("Settings:", settings);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_recursive_function_with_array() {
    let input = r#"
        fn factorial(n: Int) -> Int {
            if n <= 1 {
                return 1;
            }
            return n * factorial(n - 1);
        }

        fn main() {
            let arr: [Int] = [1, 2, 3, 4, 5];
            for i in 0..5 {
                let result = factorial(arr[i]);
                print("Factorial:", result);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_returned_from_conditional() {
    let input = r#"
        fn getArray(flag: Bool) -> [Int] {
            if flag {
                return [1, 2, 3];
            }
            return [4, 5, 6];
        }

        fn main() {
            let arr1 = getArray(true);
            let arr2 = getArray(false);
            print(arr1, arr2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_multiple_maps_in_scope() {
    let input = r#"
        fn main() {
            let map1: {Str: Int} = {"a": 1, "b": 2};
            let map2: {Str: Int} = {"c": 3, "d": 4};
            let map3: {Str: Int} = {"e": 5, "f": 6};
            print("Maps created");
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_access_in_function() {
    let input = r#"
        fn getElement(arr: [Int], index: Int) -> Int {
            return arr[index];
        }

        fn main() {
            let data: [Int] = [10, 20, 30, 40, 50];
            let val = getElement(data, 2);
            print("Value:", val);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_loop_with_multiple_arrays() {
    let input = r#"
        fn main() {
            let arr1: [Int] = [1, 2, 3];
            let arr2: [Int] = [4, 5, 6];
            for i in 0..3 {
                print(arr1[i], arr2[i]);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_function_with_multiple_array_params() {
    let input = r#"
        fn combine(a: [Int], b: [Int]) -> Int {
            return a[0] + b[0];
        }

        fn main() {
            let x: [Int] = [10, 20];
            let y: [Int] = [30, 40];
            let result = combine(x, y);
            print("Result:", result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_string_in_conditional() {
    let input = r#"
        fn main() {
            let flag = true;
            if flag {
                let msg = "True branch";
                print(msg);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_creation_in_multiple_functions() {
    let input = r#"
        fn createFirstArray() -> [Int] {
            return [1, 2, 3];
        }

        fn createSecondArray() -> [Int] {
            return [4, 5, 6];
        }

        fn main() {
            let a1 = createFirstArray();
            let a2 = createSecondArray();
            print(a1, a2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_map_returned_from_function() {
    let input = r#"
        fn createConfig() -> {Str: Int} {
            return {"width": 1920, "height": 1080};
        }

        fn main() {
            let config = createConfig();
            print("Config:", config);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_complex_expression_with_arrays() {
    let input = r#"
        fn main() {
            let arr: [Int] = [10, 20, 30];
            let result = arr[0] + arr[1] * arr[2];
            print("Result:", result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_mutable_variable_with_array() {
    let input = r#"
        fn main() {
            let mut arr: [Int] = [1, 2, 3];
            arr = [4, 5, 6];
            print("Array:", arr);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_loop_with_string_creation() {
    let input = r#"
        fn main() {
            for i in 0..5 {
                let msg = "Message";
                print(msg, i);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_in_nested_conditionals() {
    let input = r#"
        fn main() {
            if true {
                if true {
                    let arr: [Int] = [1, 2, 3];
                    print("Array:", arr);
                }
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_function_chain_with_arrays() {
    let input = r#"
        fn step1(arr: [Int]) -> [Int] {
            return arr;
        }

        fn step2(arr: [Int]) -> [Int] {
            return arr;
        }

        fn main() {
            let data: [Int] = [1, 2, 3];
            let r1 = step1(data);
            let r2 = step2(r1);
            print(r2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_large_string_concatenation() {
    let input = r#"
        fn main() {
            let s = "a" + "b" + "c" + "d" + "e" + "f" + "g" + "h";
            print(s);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_map_in_loop() {
    let input = r#"
        fn main() {
            for i in 0..3 {
                let m: {Str: Int} = {"key": i};
                print("Map:", m);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_multiple_return_paths_with_arrays() {
    let input = r#"
        fn getData(x: Int) -> [Int] {
            if x > 0 {
                return [1, 2, 3];
            } else {
                if x < 0 {
                    return [4, 5, 6];
                }
            }
            return [7, 8, 9];
        }

        fn main() {
            let a1 = getData(1);
            print(a1);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_with_boolean_elements() {
    let input = r#"
        fn main() {
            let checks: [Bool] = [true, false, true];
            print("Checks:", checks);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_deeply_nested_loops_with_allocation() {
    let input = r#"
        fn main() {
            for i in 0..2 {
                for j in 0..2 {
                    for k in 0..2 {
                        let arr: [Int] = [i, j, k];
                        print("Array:", arr);
                    }
                }
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_function_with_string_return() {
    let input = r#"
        fn formatMessage(name: Str) -> Str {
            return "Hello, " + name;
        }

        fn main() {
            let msg1 = formatMessage("Alice");
            let msg2 = formatMessage("Bob");
            print(msg1, msg2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_with_arithmetic_access() {
    let input = r#"
        fn main() {
            let arr: [Int] = [10, 20, 30, 40, 50];
            let idx = 2;
            let val = arr[idx];
            print("Value:", val);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_multiple_string_variables() {
    let input = r#"
        fn main() {
            let s1 = "First";
            let s2 = "Second";
            let s3 = "Third";
            let s4 = "Fourth";
            print(s1, s2, s3, s4);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_passed_to_multiple_functions() {
    let input = r#"
        fn func1(arr: [Int]) -> Int {
            return arr[0];
        }

        fn func2(arr: [Int]) -> Int {
            return arr[1];
        }

        fn main() {
            let data: [Int] = [100, 200, 300];
            let v1 = func1(data);
            let v2 = func2(data);
            print(v1, v2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_conditional_with_multiple_arrays() {
    let input = r#"
        fn main() {
            let flag = true;
            if flag {
                let arr1: [Int] = [1, 2, 3];
                print(arr1);
            } else {
                let arr2: [Int] = [4, 5, 6];
                print(arr2);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_map_with_multiple_types() {
    let input = r#"
        fn main() {
            let intMap: {Str: Int} = {"a": 1, "b": 2};
            let boolMap: {Str: Bool} = {"x": true, "y": false};
            print(intMap, boolMap);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_element_update_in_loop() {
    let input = r#"
        fn main() {
            let mut arr: [Int] = [0, 0, 0, 0, 0];
            for i in 0..5 {
                arr[i] = i * 10;
            }
            print("Array:", arr);
        }
    "#;
    assert!(compile_full_pipeline(input).is_err());
}

#[test]
fn mem_function_returning_different_arrays() {
    let input = r#"
        fn getSmallArray() -> [Int] {
            return [1, 2];
        }

        fn getLargeArray() -> [Int] {
            return [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        }

        fn main() {
            let small = getSmallArray();
            let large = getLargeArray();
            print(small, large);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_nested_function_with_string_param() {
    let input = r#"
        fn inner(msg: Str) -> Str {
            return msg + "!";
        }

        fn outer(msg: Str) -> Str {
            return inner(msg);
        }

        fn main() {
            let result = outer("Hello");
            print(result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_loop_with_array_access() {
    let input = r#"
        fn main() {
            let data: [Int] = [5, 10, 15, 20, 25];
            for i in 0..5 {
                let val = data[i];
                print("Value:", val);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_complex_array_expressions() {
    let input = r#"
        fn main() {
            let arr1: [Int] = [1, 2, 3];
            let arr2: [Int] = [4, 5, 6];
            let sum = arr1[0] + arr2[0];
            print("Sum:", sum);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_string_parameter_in_multiple_calls() {
    let input = r#"
        fn greet(name: Str) -> Str {
            return "Hello " + name;
        }

        fn main() {
            let g1 = greet("Alice");
            let g2 = greet("Bob");
            let g3 = greet("Charlie");
            print(g1, g2, g3);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_allocation_stress() {
    let input = r#"
        fn main() {
            for i in 0..10 {
                let temp: [Int] = [i, i + 1, i + 2, i + 3, i + 4];
                print("Allocated:", i);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_map_with_computed_values() {
    let input = r#"
        fn main() {
            let x = 10;
            let m: {Str: Int} = {"a": x, "b": x + 5, "c": x + 10};
            print("Map:", m);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_multiple_function_returns() {
    let input = r#"
        fn getValue1() -> Int {
            return 100;
        }

        fn getValue2() -> Int {
            return 200;
        }

        fn getValue3() -> Int {
            return 300;
        }

        fn main() {
            let v1 = getValue1();
            let v2 = getValue2();
            let v3 = getValue3();
            print(v1, v2, v3);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_in_early_return() {
    let input = r#"
        fn getArray(flag: Bool) -> [Int] {
            if flag {
                return [1, 2, 3];
            }
            let arr: [Int] = [4, 5, 6];
            return arr;
        }

        fn main() {
            let result = getArray(true);
            print(result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_string_array_iteration() {
    let input = r#"
        fn main() {
            let names: [Str] = ["Alice", "Bob", "Charlie"];
            for name in names {
                print("Name:", name);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_nested_map_operations() {
    let input = r#"
        fn main() {
            let m1: {Str: Int} = {"a": 1};
            let m2: {Str: Int} = {"b": 2};
            print(m1, m2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_with_loop_variable() {
    let input = r#"
        fn main() {
            for i in 0..5 {
                let arr: [Int] = [i, i * 2, i * 3];
                print("Array:", arr);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_function_with_array_and_int_params() {
    let input = r#"
        fn process(arr: [Int], multiplier: Int) -> Int {
            return arr[0] * multiplier;
        }

        fn main() {
            let data: [Int] = [10, 20, 30];
            let result = process(data, 5);
            print("Result:", result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_conditional_array_creation() {
    let input = r#"
        fn main() {
            let x = 5;
            if x > 0 {
                let arr: [Int] = [1, 2, 3];
                print(arr);
            }
            if x < 10 {
                let arr: [Int] = [4, 5, 6];
                print(arr);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_multiple_array_returns_in_loop() {
    let input = r#"
        fn createArray(seed: Int) -> [Int] {
            return [seed, seed + 1, seed + 2];
        }

        fn main() {
            for i in 0..5 {
                let arr = createArray(i);
                print("Array:", arr);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_string_comparison_with_allocation() {
    let input = r#"
        fn main() {
            let s1 = "hello";
            let s2 = "world";
            let match = true;
            if match {
                print("Match");
            }
            print(s1, s2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_copy_semantics() {
    let input = r#"
        fn main() {
            let arr1: [Int] = [1, 2, 3];
            let arr2 = arr1;
            print(arr1, arr2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_function_with_multiple_string_params() {
    let input = r#"
        fn combine(a: Str, b: Str, c: Str) -> Str {
            return a + b + c;
        }

        fn main() {
            let result = combine("Hello", " ", "World");
            print(result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_loop_with_mutable_counter_and_array() {
    let input = r#"
        fn main() {
            let mut count = 0;
            let arr: [Int] = [10, 20, 30, 40, 50];
            for i in 0..5 {
                count += arr[i];
            }
            print("Total:", count);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_nested_array_creation() {
    let input = r#"
        fn main() {
            let outer: [[Int]] = [[1, 2], [3, 4], [5, 6], [7, 8]];
            print("Nested:", outer);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_map_stress_test() {
    let input = r#"
        fn main() {
            for i in 0..5 {
                let m: {Str: Int} = {"key": i};
                print("Iteration:", i);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_function_returning_bool_with_array() {
    let input = r#"
        fn checkArray(arr: [Int]) -> Bool {
            return arr[0] > 0;
        }

        fn main() {
            let data: [Int] = [5, 10, 15];
            let result = checkArray(data);
            print("Result:", result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_array_with_all_same_values() {
    let input = r#"
        fn main() {
            let arr: [Int] = [42, 42, 42, 42, 42];
            print("Array:", arr);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_string_in_nested_function_calls() {
    let input = r#"
        fn format1(s: Str) -> Str {
            return "[" + s + "]";
        }

        fn format2(s: Str) -> Str {
            return format1(s);
        }

        fn main() {
            let result = format2("test");
            print(result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_loop_with_array_sum() {
    let input = r#"
        fn main() {
            let arr: [Int] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
            let mut total = 0;
            for i in 0..10 {
                total += arr[i];
            }
            print("Total:", total);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn mem_multiple_conditional_branches_with_arrays() {
    let input = r#"
        fn main() {
            let x = 5;
            if x == 5 {
                let arr: [Int] = [5, 5, 5];
                print(arr);
            } else if x == 10 {
                let arr: [Int] = [10, 10, 10];
                print(arr);
            } else {
                let arr: [Int] = [0, 0, 0];
                print(arr);
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}
