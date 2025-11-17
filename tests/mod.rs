//! Restructured Test Suite for Doo Compiler
//!
//! Test Organization:
//!
//! - common: Shared utilities and helpers
//!
//! - integration/: Full pipeline end-to-end tests
//!   - basic.rs: Entry-level feature concepts
//!   - functions.rs: Function patterns and semantics
//!   - types.rs: Type system behavior
//!   - control_flow.rs: Control flow patterns
//!   - collections.rs: Array and Map operations
//!   - builtins.rs: Built-in methods
//!   - imports.rs: Import system
//!   - program_files.rs: Actual .doo file compilation (PRIMARY TESTS)
//!
//!
//! - regression/: Bug regression tests
//!   - regression.rs: Previously broken bugs now fixed (should all pass)
//!
//! - stress/: Performance and stress tests
//!   - memory.rs: Large programs, deep nesting, stress scenarios
//!
//!
//! Test Philosophy:
//! - INTEGRATION TESTS: Conceptual tests using inline code strings
//! - PROGRAM_FILES: PRIMARY tests using actual .doo fixture files
//! - REGRESSION: Previously fixed bugs that shouldn't regress
//! - STRESS: Large programs and edge cases
//! - FEATURES: Advanced language features not yet fully tested

pub mod common;

#[cfg(test)]
mod integration {
    pub mod basic;
    pub mod builtins;
    pub mod collections;
    pub mod control_flow;
    pub mod functions;
    pub mod imports;
    pub mod program_files;
    pub mod types;
}

#[cfg(test)]
mod regression {
    pub mod regression;
}

#[cfg(test)]
mod stress {
    pub mod memory;
}
