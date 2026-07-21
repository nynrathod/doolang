//! Framework modules — HTTP framework features (auth, CRUD, webhooks, RBAC, etc.)
//!
//! These modules were extracted from the root of doo_ffi_http into this
//! submodule as part of Phase 4 (FFI shrinking). They depend on the transport
//! layer (server, router, types, helpers) but are framework-level features,
//! not core transport.
//!
//! Future phases will extract this into a separate crate.

// Re-import #[macro_export] macros from the crate root

pub mod auth;
pub mod crud;
pub mod db_bridge;
pub mod map_ops;
pub mod metadata;
pub mod metrics;
pub mod middleware;
pub mod middleware_ffi;
pub mod oauth;
pub mod password_reset;
pub mod rbac;
pub mod validation;
pub mod webhook_engine;
pub mod webhook_log;
