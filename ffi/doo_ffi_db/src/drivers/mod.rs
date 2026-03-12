//! Database Driver Implementations
//!
//! Each subfolder is a complete database driver implementing `crate::driver::DbDriver`.
//!
//! ## Current Drivers
//!
//! - `postgres` — PostgreSQL via `tokio-postgres` + `deadpool-postgres`
//!
//! ## Adding a new driver
//!
//! 1. Create `drivers/<name>/mod.rs` implementing `DbDriver`
//! 2. Add `pub mod <name>;` below (with `#[cfg(feature = "<name>")]`)
//! 3. Add deps to `Cargo.toml` as optional (feature-gated)
//! 4. Add `doo_db_connect_<name>()` FFI function in `lib.rs`

#[cfg(feature = "postgres")]
pub mod postgres;
