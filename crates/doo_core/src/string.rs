//! String Utilities
//!
//! String case conversion utilities (`to_snake_case`, `to_pascal_case`)
//! have been moved to `doo_ffi_core::case` per the Compiler↔Framework
//! Separation Audit (§3.1). FFI crates must not depend on `doo_core`;
//! these utilities now live in the Tier A runtime crate `doo_ffi_core`.
//!
//! This module is kept as a placeholder to avoid breaking the module tree.
//! Once all compiler crates have migrated their imports, this module
//! can be removed from `doo_core/src/lib.rs`.
