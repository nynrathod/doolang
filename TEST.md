## 🚀 Quick Start Commands

### Run All Tests
##### This won't include fuzz test and memory leak test, do read further for this
```powershell
cargo test
```

### Run All Memory and Fuzz Tests

To run all memory checks, Valgrind tests, circular import detection, memory stress, regression, and unit tests, use the provided scripts:

#### Run all memory and regression/unit tests (Valgrind, circular import, stress, regressions, unit)
```bash
sh doo/test_all.sh
```
This script will:
- Build the DooLang compiler in release mode
- Run Valgrind memory leak checks on `.doo` test programs in `tests/`
- Check for circular import detection (in `tests/circular_import_test/`)
- Run memory stress tests (`cargo test --test memory_stress`)
- Run all regression tests (`cargo test --test regressions`)
- Run all unit tests (`cargo test --lib`)
- Print a summary of passed/failed/skipped tests

#### Run all fuzzers (for 1 hour each, in parallel)
```bash
sh doo/run_all_fuzzers.sh
```
This script will:
- Start all 5 fuzz targets in parallel for 1 hour each:
  - `fuzz_lexer`
  - `fuzz_parser`
  - `fuzz_analyzer`
  - `fuzz_mir`
  - `fuzz_codegen`
- Monitor logs in `fuzz_logs/`
- Report any crashes or new artifacts found in `fuzz/artifacts/`
- Print a summary at the end

### Run Specific Unit Stage
```powershell
cargo test --lib lexer_tests      # Lexer only
cargo test --lib parser_tests     # Parser only
cargo test --lib analyzer_tests   # Analyzer only
cargo test --lib mir_tests        # MIR only
cargo test --lib codegen_tests    # Codegen only
cargo test --test integration_tests  # Integration only
```

### Verbose Output
```powershell
cargo test -- --nocapture
```

### Single Test
```powershell
cargo test test_basic_tokens
```

# Show output for specific test
```powershell
cargo test test_basic_tokens -- --nocapture
```

### Verbose Mode
```powershell
cargo test -- --test-threads=1 --nocapture
```

## Adding New Tests

### Example: Add a new parser test
```rust
#[test]
fn test_my_feature() {
    let input = "your code here";
    let tokens = lex(input);
    let mut parser = Parser::new(&tokens);
    let result = parser.parse_statement();
    assert!(result.is_ok());
}
```

### Example: Add a new analyzer test
```rust
#[test]
fn test_my_semantic_check() {
    let input = "fn main() { /* your code */ }";
    assert!(analyze_code(input).is_ok());
}
```
