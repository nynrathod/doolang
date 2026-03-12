//! Process FFI Tests — Production-grade coverage
//! Full compiler pipeline: lex → parse → analyze → MIR → codegen
//! Syntax matches dev_test/process/main.doo

use super::{assert_ffi_compiles, assert_ffi_compiles_with};

// =============================================================================
// 1. PROCESS IMPORTS
// =============================================================================

#[test]
fn process_import() {
    assert_ffi_compiles("import std::Process::{Process, ProcessError}; fn main() { }");
}

// =============================================================================
// 2. PROCESS::RUN — synchronous command execution
// =============================================================================

#[test]
fn process_run_simple() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let result = Process::run("echo", "[\"hello_doo\"]")?;
    print(result);
}
"#,
        "hello_doo",
    );
}

#[test]
fn process_run_multiple_args() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let result = Process::run("echo", "[\"hello\", \"world\", \"from\", \"doo\"]")?;
    print(result);
}
"#,
        "hello",
    );
}

#[test]
fn process_run_empty_args() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let result = Process::run("echo", "[]")?;
    print(result);
}
"#,
        "echo",
    );
}

// =============================================================================
// 3. PROCESS::OUTPUT — stdout capture
// =============================================================================

#[test]
fn process_output() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let out = Process::output("echo", "[\"process_output_test\"]")?;
    print(out);
}
"#,
        "output",
    );
}

// =============================================================================
// 4. PROCESS::SPAWN — async process management
// =============================================================================

#[test]
fn process_spawn_and_kill() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let handle = Process::spawn("ping", "[\"127.0.0.1\"]")?;
    print(handle);
    let running = Process::isRunning(handle);
    print(running);
    Process::kill(handle)?;
}
"#,
        "spawn",
    );
}

#[test]
fn process_spawn_status() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let handle = Process::spawn("ping", "[\"127.0.0.1\"]")?;
    let status = Process::status(handle)?;
    print(status);
    Process::kill(handle)?;
}
"#,
        "status",
    );
}

#[test]
fn process_spawn_wait_for_output() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let handle = Process::spawn("echo", "[\"wait_test\"]")?;
    let output = Process::waitForOutput(handle)?;
    print(output);
}
"#,
        "wait_test",
    );
}

// =============================================================================
// 5. PROCESS::ACTIVECOUNT — registry tracking
// =============================================================================

#[test]
fn process_active_count() {
    assert_ffi_compiles(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let count = Process::activeCount();
    print(count);
}
"#,
    );
}

// =============================================================================
// 6. PROCESS::SHUTDOWN — bulk cleanup
// =============================================================================

#[test]
fn process_shutdown() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let h1 = Process::spawn("ping", "[\"127.0.0.1\"]")?;
    let h2 = Process::spawn("ping", "[\"127.0.0.1\"]")?;
    Process::shutdown();
    let count = Process::activeCount();
    print(count);
}
"#,
        "shutdown",
    );
}

// =============================================================================
// 7. ERROR HANDLING
// =============================================================================

#[test]
fn process_error_propagation() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn runCommand(cmd: Str, args: Str) -> Str ! Str {
    let result = Process::run(cmd, args)?;
    Ok result;
}
fn main() {
    let out, err = runCommand("echo", "[\"test\"]");
    if err != nil {
        print("Error:", err);
    } else {
        print("Output:", out);
    }
}
"#,
        "runCommand",
    );
}

// =============================================================================
// 8. COMPLEX PROCESS PATTERNS
// =============================================================================

#[test]
fn process_sequential_runs() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    for i in 0..10 {
        let r = Process::run("echo", "[\"iteration\"]")?;
    }
    let count = Process::activeCount();
    print(count);
}
"#,
        "iteration",
    );
}

#[test]
fn process_spawn_kill_cycle() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    for j in 0..5 {
        let sh = Process::spawn("echo", "[\"cycle\"]")?;
        Process::kill(sh)?;
    }
    sleep(100);
    let count = Process::activeCount();
    print(count);
}
"#,
        "cycle",
    );
}

#[test]
fn process_args_with_spaces() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let result = Process::run("echo", "[\"hello world\", \"with multiple\", \"spaces\"]")?;
    print(result);
}
"#,
        "hello world",
    );
}

#[test]
fn process_special_chars_in_args() {
    assert_ffi_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let result = Process::run("echo", "[\"test=value\", \"key:value\"]")?;
    print(result);
}
"#,
        "test=value",
    );
}
