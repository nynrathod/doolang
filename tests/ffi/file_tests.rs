//! File FFI Tests — Production-grade coverage
//! Full compiler pipeline: lex → parse → analyze → MIR → codegen
//! Syntax matches dev_test/file/ patterns: File::Read, File::Write (PascalCase)

use super::{assert_ffi_compiles, assert_ffi_compiles_with};

// =============================================================================
// 1. FILE READ OPERATIONS
// =============================================================================

#[test]
fn file_read_text() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    let content = File::Read("data.txt")?;
    print(content);
}
"#,
        "data.txt",
    );
}

#[test]
fn file_read_with_error_handling() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    let content, err = File::Read("data.txt");
    if err == nil {
        print(content);
    } else {
        print("File not found");
    }
}
"#,
        "File not found",
    );
}

#[test]
fn file_read_lines() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    let lines = File::ReadLines("data.txt")?;
    for line in lines {
        print(line);
    }
}
"#,
        "data.txt",
    );
}

// =============================================================================
// 2. FILE WRITE OPERATIONS
// =============================================================================

#[test]
fn file_write_text() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    File::Write("output.txt", "Hello, World!")?;
}
"#,
        "Hello, World!",
    );
}

#[test]
fn file_append_text() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    File::Append("log.txt", "New log entry\n")?;
}
"#,
        "log.txt",
    );
}

#[test]
fn file_write_with_error_handling() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    let _, err = File::Write("/readonly/path.txt", "data");
    if err != nil {
        print("Permission denied");
    }
}
"#,
        "Permission denied",
    );
}

// =============================================================================
// 3. FILE EXISTENCE AND INFO
// =============================================================================

#[test]
fn file_exists() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    let exists = File::Exists("config.json");
    if exists {
        print("Config found");
    } else {
        print("No config");
    }
}
"#,
        "config.json",
    );
}

#[test]
fn file_delete() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    File::Delete("temp.txt")?;
}
"#,
        "temp.txt",
    );
}

#[test]
fn file_size() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    let size, err = File::Size("data.txt");
    if err == nil {
        print("Size:", size);
    }
}
"#,
        "data.txt",
    );
}

#[test]
fn file_metadata() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    let meta, err = File::Metadata("data.txt");
    if err == nil {
        print("Metadata:", meta);
    }
}
"#,
        "data.txt",
    );
}

// =============================================================================
// 4. DIRECTORY OPERATIONS
// =============================================================================

#[test]
fn file_mkdir() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    File::MkDir("output/data")?;
}
"#,
        "output/data",
    );
}

#[test]
fn file_list_dir() {
    assert_ffi_compiles(
        r#"
import std::File;
fn main() {
    let files = File::ListDir(".")?;
    for f in files {
        print(f);
    }
}
"#,
    );
}

#[test]
fn file_rmdir() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    File::RmDir("empty_dir")?;
}
"#,
        "empty_dir",
    );
}

#[test]
fn file_rmdir_all() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    File::RmDirAll("dir_with_contents")?;
}
"#,
        "dir_with_contents",
    );
}

// =============================================================================
// 5. FILE COPY AND MOVE
// =============================================================================

#[test]
fn file_copy() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    File::Copy("source.txt", "dest.txt")?;
}
"#,
        "source.txt",
    );
}

#[test]
fn file_move() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    File::Move("old.txt", "new.txt")?;
}
"#,
        "old.txt",
    );
}

// =============================================================================
// 6. ERROR HANDLING PATTERNS
// =============================================================================

#[test]
fn file_error_propagation() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn readConfig() -> Str ! Str {
    let content = File::Read("config.json")?;
    Ok content;
}
fn main() {
    let cfg, err = readConfig();
    if err != nil {
        print("Error:", err);
    } else {
        print("Config:", cfg);
    }
}
"#,
        "readConfig",
    );
}

#[test]
fn file_error_chain() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn loadAndProcess() -> [Str] ! Str {
    let content = File::Read("data.csv")?;
    let lines = content.split("\n");
    Ok lines;
}
fn main() { }
"#,
        "data.csv",
    );
}

#[test]
fn file_copy_pattern() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn copyFile(src: Str, dst: Str) -> Bool ! Str {
    let content = File::Read(src)?;
    File::Write(dst, content)?;
    Ok true;
}
fn main() {
    let success = copyFile("a.txt", "b.txt")?;
    if success { print("Copied"); }
}
"#,
        "Copied",
    );
}

// =============================================================================
// 7. COMPLEX FILE SCENARIOS
// =============================================================================

#[test]
fn file_multi_operation() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn main() {
    File::MkDir("output")?;
    File::Write("output/data.txt", "Hello")?;
    let content = File::Read("output/data.txt")?;
    print(content);
    File::Append("output/data.txt", "\nWorld")?;
    let updated = File::Read("output/data.txt")?;
    print(updated);
}
"#,
        "output/data.txt",
    );
}

#[test]
fn file_conditional_read_write() {
    assert_ffi_compiles_with(
        r#"
import std::File;
fn ensureConfig(path: Str) -> Str ! Str {
    if File::Exists(path) {
        let content = File::Read(path)?;
        Ok content;
    } else {
        let defaults = "{ \"debug\": false }";
        File::Write(path, defaults)?;
        Ok defaults;
    }
}
fn main() {
    let cfg = ensureConfig("config.json")?;
    print(cfg);
}
"#,
        "debug",
    );
}
