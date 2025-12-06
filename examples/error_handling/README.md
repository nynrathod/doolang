### Error Handling Usage Examples

To view all examples of error handling, follow the [@error_handling](file:///X:/Projects/doo/tests/dev_test/error_handling) link.

There are 4 main files in this folder:

- **all_return_type.doo**  
  Tests error handling for all supported types (Int, Float, Bool, Str, structs, arrays, maps). Shows both success and error cases for each type.

- **advance.doo**  
  Comprehensive suite covering primitive types, structs, enums, error propagation, chaining, and complex operations. Demonstrates both manual and automatic error handling, including advanced domain models.

- **error_rules1.doo**  
  Focuses on error handling rules: how return types determine Ok/Err usage, auto-propagation (`?`), manual error handling, multiple return values, and correct/incorrect patterns. Shows both error and success paths.

- **error_rules2.doo**  
  Continues error handling rules with struct errors, panic (`??`), deep/nested propagation, multiple return values, edge cases, and advanced patterns. Demonstrates manual and auto error handling, as well as success and error scenarios.

These files demonstrate:

- **Manual error handling**: Explicitly checking error values and handling them.
- **Automatic error propagation**: Using `?` to auto-return errors up the call stack.
- **Generic error handling**: Using `! Error` to propagate multiple error types.
- **Success and error cases**: Each file includes examples of both successful operations and error scenarios.
