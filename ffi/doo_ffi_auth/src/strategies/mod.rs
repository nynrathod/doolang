//! Authentication Strategy Implementations
//!
//! Each subfolder is a complete auth strategy implementing `crate::strategy::AuthStrategy`.
//!
//! ## Current Strategies
//!
//! - `jwt` — JSON Web Tokens via HS256 (jsonwebtoken crate)
//!
//! ## Adding a new strategy
//!
//! 1. Create `strategies/<name>/mod.rs` implementing `AuthStrategy`
//! 2. Add `pub mod <name>;` below (with `#[cfg(feature = "<name>")]`)
//! 3. Add deps to `Cargo.toml` as optional (feature-gated)

#[cfg(feature = "jwt")]
pub mod jwt;
