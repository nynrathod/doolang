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
                let mut codegen = CodeGen::new("integration_test", &context);
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
// Integration Tests: Multi-Function Systems
// =====================================================================

#[test]
fn integration_math_operations_system() {
    let input = r#"
        fn add(a: Int, b: Int) -> Int {
            return a + b;
        }

        fn subtract(a: Int, b: Int) -> Int {
            return a - b;
        }

        fn multiply(a: Int, b: Int) -> Int {
            return a * b;
        }

        fn divide(a: Int, b: Int) -> Int {
            if b == 0 {
                return 0;
            }
            return a / b;
        }

        fn main() {
            let r1 = add(10, 5);
            let r2 = subtract(20, 8);
            let r3 = multiply(6, 7);
            let r4 = divide(100, 5);
            print("Results:", r1, r2, r3, r4);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_array_processing_pipeline() {
    let input = r#"
        fn sumArray(arr: [Int]) -> Int {
            let mut total = 0;
            for i in 0..5 {
                total += arr[i];
            }
            return total;
        }

        fn averageArray(arr: [Int]) -> Int {
            let total = sumArray(arr);
            return total / 5;
        }

        fn findMax(arr: [Int]) -> Int {
            let mut max = arr[0];
            for i in 1..5 {
                if arr[i] > max {
                    max = arr[i];
                }
            }
            return max;
        }

        fn main() {
            let numbers: [Int] = [10, 20, 30, 40, 50];
            let total = sumArray(numbers);
            let avg = averageArray(numbers);
            let maximum = findMax(numbers);
            print("Total:", total, "Average:", avg, "Max:", maximum);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_string_processing_system() {
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
fn integration_map_iteration_system() {
    let input = r#"
        fn main() {
            let scores: {Str: Int} = {"Alice": 95, "Bob": 87};
            let mut total = 0;
            for (key, value) in scores {
                total += value;
            }
            print(total);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_conditional_processing_chain() {
    let input = r#"
        fn main() {
            let x = 10;
            if x > 5 {
                print("Greater");
            }
            if x < 5 {
                print("Less");
            }
            if x == 10 {
                print("Equal");
            }
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_recursive_computation_system() {
    let input = r#"
        fn factorial(n: Int) -> Int {
            if n <= 1 {
                return 1;
            }
            return n * factorial(n - 1);
        }

        fn fibonacci(n: Int) -> Int {
            if n <= 1 {
                return n;
            }
            return fibonacci(n - 1) + fibonacci(n - 2);
        }

        fn power(base: Int, exp: Int) -> Int {
            if exp == 0 {
                return 1;
            }
            return base * power(base, exp - 1);
        }

        fn main() {
            let f5 = factorial(5);
            let fib8 = fibonacci(8);
            let p23 = power(2, 3);
            print("Factorial:", f5, "Fibonacci:", fib8, "Power:", p23);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_validation_system() {
    let input = r#"
        fn validateEmail(email: Str) -> Int {
            for i in 0..1 {
                if i == 0 {
                    return 1;
                }
            }
            return 0;
        }

        fn validateAge(age: Int) -> Int {
            if age >= 18 {
                return 1;
            }
            return 0;
        }

        fn validateUser(name: Str, age: Int) -> Int {
            if validateAge(age) == 1 {
                if validateEmail(name) == 1 {
                    return 1;
                }
            }
            return 0;
        }

        fn main() {
            let valid = validateUser("test@test", 25);
            print("Valid:", valid);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_data_transformation_pipeline() {
    let input = r#"
        fn double(n: Int) -> Int {
            return n * 2;
        }

        fn increment(n: Int) -> Int {
            return n + 1;
        }

        fn processArray(arr: [Int]) -> Int {
            let mut result = 0;
            for i in 0..5 {
                let doubled = double(arr[i]);
                let incremented = increment(doubled);
                result += incremented;
            }
            return result;
        }

        fn main() {
            let input: [Int] = [1, 2, 3, 4, 5];
            let output = processArray(input);
            print("Processed:", output);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_nested_loop_processing() {
    let input = r#"
        fn sumMatrix() -> Int {
            let mut total = 0;
            for i in 0..3 {
                for j in 0..3 {
                    total += 1;
                }
            }
            return total;
        }

        fn countElements(count: Int) -> Int {
            let mut result = 0;
            for i in 0..count {
                for j in 0..count {
                    result += 1;
                }
            }
            return result;
        }

        fn main() {
            let total = sumMatrix();
            let counted = countElements(3);
            print("Total:", total, "Counted:", counted);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_state_machine_simulation() {
    let input = r#"
        fn nextState(state: Int) -> Int {
            if state == 0 {
                return 1;
            }
            if state == 1 {
                return 2;
            }
            return 0;
        }

        fn runStateMachine(steps: Int) -> Int {
            let mut state = 0;
            for i in 0..steps {
                state = nextState(state);
            }
            return state;
        }

        fn main() {
            let finalState = runStateMachine(5);
            print("Final state:", finalState);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_accumulator_pattern() {
    let input = r#"
        fn accumulateSum(arr: [Int]) -> Int {
            let mut acc = 0;
            for i in 0..5 {
                acc += arr[i];
            }
            return acc;
        }

        fn accumulateProduct(arr: [Int]) -> Int {
            let mut acc = 1;
            for i in 0..5 {
                acc *= arr[i];
            }
            return acc;
        }

        fn accumulateMax(arr: [Int]) -> Int {
            let mut acc = arr[0];
            for i in 1..5 {
                if arr[i] > acc {
                    acc = arr[i];
                }
            }
            return acc;
        }

        fn main() {
            let values: [Int] = [2, 3, 4, 5, 6];
            let sum = accumulateSum(values);
            let product = accumulateProduct(values);
            let max = accumulateMax(values);
            print("Sum:", sum, "Product:", product, "Max:", max);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_multi_parameter_calculations() {
    let input = r#"
        fn calculate(a: Int, b: Int, c: Int) -> Int {
            return a + b * c;
        }

        fn complexCalculate(x: Int, y: Int, z: Int) -> Int {
            let step1 = calculate(x, y, z);
            let step2 = calculate(step1, x, 2);
            return step2;
        }

        fn main() {
            let r1 = calculate(5, 3, 2);
            let r2 = complexCalculate(2, 3, 4);
            print("Results:", r1, r2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_array_filtering_system() {
    let input = r#"
        fn countGreater(arr: [Int], threshold: Int) -> Int {
            let mut count = 0;
            for i in 0..5 {
                if arr[i] > threshold {
                    count += 1;
                }
            }
            return count;
        }

        fn countLess(arr: [Int], threshold: Int) -> Int {
            let mut count = 0;
            for i in 0..5 {
                if arr[i] < threshold {
                    count += 1;
                }
            }
            return count;
        }

        fn main() {
            let numbers: [Int] = [10, 20, 30, 40, 50];
            let greater25 = countGreater(numbers, 25);
            let less35 = countLess(numbers, 35);
            print("Greater:", greater25, "Less:", less35);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_conditional_branching_system() {
    let input = r#"
        fn getGrade(score: Int) -> Str {
            if score >= 90 {
                return "A";
            }
            if score >= 80 {
                return "B";
            }
            if score >= 70 {
                return "C";
            }
            return "F";
        }

        fn processGrades(scores: [Int]) -> Str {
            let mut result = "";
            for i in 0..3 {
                let grade = getGrade(scores[i]);
                result = result + grade;
            }
            return result;
        }

        fn main() {
            let grades: [Int] = [95, 85, 75];
            let result = processGrades(grades);
            print("Grades:", result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_variable_scope_integration() {
    let input = r#"
        fn createCounter() -> Int {
            let mut counter = 0;
            for i in 0..10 {
                counter += 1;
            }
            return counter;
        }

        fn nestedCounter() -> Int {
            let outer = 5;
            for i in 0..3 {
                let inner = outer + i;
                if inner > 6 {
                    return inner;
                }
            }
            return outer;
        }

        fn main() {
            let c1 = createCounter();
            let c2 = nestedCounter();
            print("Counters:", c1, c2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_boolean_logic_system() {
    let input = r#"
        fn isValid(age: Int, hasLicense: Int) -> Int {
            if age >= 18 {
                if hasLicense == 1 {
                    return 1;
                }
            }
            return 0;
        }

        fn checkMultiple(a: Int, b: Int, c: Int) -> Int {
            if a > 0 {
                if b > 0 {
                    if c > 0 {
                        return 1;
                    }
                }
            }
            return 0;
        }

        fn main() {
            let v1 = isValid(25, 1);
            let v2 = checkMultiple(5, 10, 15);
            print("Valid:", v1, v2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_range_iteration_system() {
    let input = r#"
        fn sumRange(start: Int, end: Int) -> Int {
            let mut sum = 0;
            for i in start..end {
                sum += i;
            }
            return sum;
        }

        fn countRange(start: Int, end: Int) -> Int {
            let mut count = 0;
            for i in start..end {
                count += 1;
            }
            return count;
        }

        fn main() {
            let s1 = sumRange(1, 11);
            let c1 = countRange(5, 15);
            print("Sum:", s1, "Count:", c1);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_mutable_state_operations() {
    let input = r#"
        fn incrementCounter(initial: Int, steps: Int) -> Int {
            let mut counter = initial;
            for i in 0..steps {
                counter += 1;
            }
            return counter;
        }

        fn accumulateValues(arr: [Int]) -> Int {
            let mut total = 0;
            for i in 0..5 {
                total += arr[i];
            }
            return total;
        }

        fn main() {
            let result1 = incrementCounter(10, 5);
            let values: [Int] = [1, 2, 3, 4, 5];
            let result2 = accumulateValues(values);
            print("Incremented:", result1, "Accumulated:", result2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_function_chaining() {
    let input = r#"
        fn step1(x: Int) -> Int {
            return x + 10;
        }

        fn step2(x: Int) -> Int {
            return x * 2;
        }

        fn step3(x: Int) -> Int {
            return x / 2;
        }

        fn processChain(input: Int) -> Int {
            let s1 = step1(input);
            let s2 = step2(s1);
            let s3 = step3(s2);
            return s3;
        }

        fn main() {
            let result = processChain(5);
            print("Chain result:", result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_control_flow_integration() {
    let input = r#"
        fn processValue(val: Int) -> Int {
            let mut result = 0;
            if val > 10 {
                for i in 0..val {
                    result += i;
                }
            } else {
                for i in 0..5 {
                    result += 1;
                }
            }
            return result;
        }

        fn main() {
            let r1 = processValue(15);
            let r2 = processValue(5);
            print("Results:", r1, r2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_array_transformation() {
    let input = r#"
        fn transformArray(arr: [Int], multiplier: Int) -> Int {
            let mut sum = 0;
            for i in 0..5 {
                sum += arr[i] * multiplier;
            }
            return sum;
        }

        fn main() {
            let input: [Int] = [2, 4, 6, 8, 10];
            let result = transformArray(input, 3);
            print("Transformed:", result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_map_key_value_operations() {
    let input = r#"
        fn getMapSum(m: {Str: Int}) -> Int {
            let mut total = 0;
            for (key, value) in m {
                total += value;
            }
            return total;
        }

        fn countMapEntries(m: {Str: Int}) -> Int {
            let mut count = 0;
            for (k, v) in m {
                count += 1;
            }
            return count;
        }

        fn main() {
            let data: {Str: Int} = {"a": 5, "b": 10, "c": 15};
            let sum = getMapSum(data);
            let cnt = countMapEntries(data);
            print("Sum:", sum, "Count:", cnt);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_conditional_loop_combination() {
    let input = r#"
        fn findTarget(arr: [Int], target: Int) -> Int {
            for i in 0..5 {
                if arr[i] == target {
                    return 1;
                }
            }
            return 0;
        }

        fn countMatches(arr: [Int], value: Int) -> Int {
            let mut count = 0;
            for i in 0..5 {
                if arr[i] == value {
                    count += 1;
                }
            }
            return count;
        }

        fn main() {
            let numbers: [Int] = [5, 10, 15, 20, 25];
            let found = findTarget(numbers, 15);
            let matches = countMatches(numbers, 10);
            print("Found:", found, "Matches:", matches);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_arithmetic_expression_integration() {
    let input = r#"
        fn arithmeticOp1(x: Int, y: Int) -> Int {
            return x + y * 2;
        }

        fn arithmeticOp2(x: Int, y: Int) -> Int {
            return x * y - x / y;
        }

        fn chainedArithmetic(a: Int, b: Int, c: Int) -> Int {
            let s1 = arithmeticOp1(a, b);
            let s2 = arithmeticOp2(s1, c);
            return s2;
        }

        fn main() {
            let result = chainedArithmetic(3, 4, 2);
            print("Arithmetic:", result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_comparison_operations() {
    let input = r#"
        fn main() {
            let a = 10 > 5;
            let b = 5 < 10;
            let c = 5 == 5;
            print(a);
            print(b);
            print(c);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_string_concatenation_operations() {
    let input = r#"
        fn concatenate(a: Str, b: Str) -> Str {
            return a + b;
        }

        fn createMessage(name: Str, greeting: Str) -> Str {
            let msg1 = concatenate(greeting, " ");
            return concatenate(msg1, name);
        }

        fn main() {
            let message = createMessage("World", "Hello");
            print(message);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_type_consistency_across_functions() {
    let input = r#"
        fn processInt(x: Int) -> Int {
            return x * 2;
        }

        fn processBool(b: Int) -> Str {
            if b == 1 {
                return "true";
            }
            return "false";
        }

        fn main() {
            let intResult = processInt(5);
            let boolResult = processBool(1);
            print("Int:", intResult, "Bool:", boolResult);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_early_return_flow() {
    let input = r#"
        fn findValue(arr: [Int], target: Int) -> Int {
            for i in 0..5 {
                if arr[i] == target {
                    return 1;
                }
            }
            return 0;
        }

        fn main() {
            let numbers: [Int] = [5, 10, 15, 20, 25];
            let result = findValue(numbers, 15);
            print(result);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_loop_with_conditional_break_pattern() {
    let input = r#"
        fn findFirstMatch(arr: [Int], threshold: Int) -> Int {
            for i in 0..5 {
                if arr[i] > threshold {
                    return arr[i];
                }
            }
            return 0;
        }

        fn processUntilCondition(limit: Int) -> Int {
            let mut result = 0;
            for i in 0..limit {
                result += i;
                if result > 20 {
                    return result;
                }
            }
            return result;
        }

        fn main() {
            let arr: [Int] = [5, 15, 25, 35, 45];
            let match1 = findFirstMatch(arr, 20);
            let match2 = processUntilCondition(10);
            print("Results:", match1, match2);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}

#[test]
fn integration_multiple_array_operations() {
    let input = r#"
        fn sumTwo(arr1: [Int], arr2: [Int]) -> Int {
            let mut total = 0;
            for i in 0..5 {
                total += arr1[i] + arr2[i];
            }
            return total;
        }

        fn compareArrays(arr1: [Int], arr2: [Int]) -> Int {
            for i in 0..5 {
                if arr1[i] != arr2[i] {
                    return 0;
                }
            }
            return 1;
        }

        fn main() {
            let a1: [Int] = [1, 2, 3, 4, 5];
            let a2: [Int] = [5, 4, 3, 2, 1];
            let sum = sumTwo(a1, a2);
            let equal = compareArrays(a1, a2);
            print("Sum:", sum, "Equal:", equal);
        }
    "#;
    assert!(compile_full_pipeline(input).is_ok());
}
