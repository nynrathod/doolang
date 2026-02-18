pub mod common;

// Unit tests - test individual compiler stages
#[cfg(test)]
mod unit {
    mod frontend {
        mod lexer_tests;
        mod parser_expr_tests;
        mod parser_tests;
    }
    mod hir {
        mod hir_tests;
    }
    mod mir {
        mod mir_tests;
    }
    mod codegen {
        mod codegen_tests;
    }
    mod analysis {
        mod borrow_checking_tests;
        mod ownership_tests;
        mod type_checking_tests;
    }
}

// Integration tests
#[cfg(test)]
mod integration {
    pub mod dev_test_runner;
}

// Compilation tests
#[cfg(test)]
mod compile_pass;

#[cfg(test)]
mod compile_fail;

#[cfg(test)]
mod codegen_verify;

#[cfg(test)]
mod run_pass;

// UI and diagnostic tests
#[cfg(test)]
mod ui;

#[cfg(test)]
mod crashes;

// Stress tests
#[cfg(test)]
mod stress {
    pub mod memory;
}

// FFI integration tests
#[cfg(test)]
mod ffi;
