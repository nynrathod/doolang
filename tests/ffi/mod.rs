//! FFI integration tests — HTTP, Database, Auth, File, JSON, WebSocket, Process, Async
//! Uses the FULL compiler pipeline via common::compile_snippet

pub mod http_tests;
pub mod database_tests;
pub mod auth_tests;
pub mod file_tests;
pub mod json_tests;
pub mod websocket_tests;
pub mod process_tests;
pub mod async_tests;

use crate::common::compile_snippet;

/// Assert FFI code compiles through the full pipeline (lex → parse → analyze → MIR → codegen)
pub fn ffi_compiles(code: &str) -> bool {
    compile_snippet(code).is_ok()
}

/// Assert FFI code compiles and IR contains expected FFI call
pub fn ffi_compiles_with(code: &str, ir_pattern: &str) -> bool {
    match compile_snippet(code) {
        Ok(ir) => ir.contains(ir_pattern),
        Err(_) => false,
    }
}

/// Assert FFI code compiles AND contains expected IR pattern, or panic
pub fn assert_ffi_compiles_with(code: &str, ir_pattern: &str) {
    match compile_snippet(code) {
        Ok(ir) => {
            if !ir.contains(ir_pattern) {
                panic!("FFI compilation succeeded but IR missing pattern '{}'.\nGot IR:\n{}", ir_pattern, ir);
            }
        },
        Err(e) => panic!("FFI test failed to compile: {}\nCode:\n{}", e, code),
    }
}

/// Assert FFI code compiles through full pipeline or panic with details
pub fn assert_ffi_compiles(code: &str) {
    match compile_snippet(code) {
        Ok(_) => (),
        Err(e) => panic!("FFI test failed to compile: {}\nCode:\n{}", e, code),
    }
}

/// Assert FFI code FAILS to compile (negative test)
pub fn assert_ffi_fails(code: &str) {
    match compile_snippet(code) {
        Err(_) => (),
        Ok(_) => panic!("Expected FFI code to fail but it compiled:\n{}", code),
    }
}
