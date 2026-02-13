//! # Doo FFI Runtime
//!
//! Single source of truth for async runtime, task spawning, structured concurrency,
//! and task handle management.
//!
//! ## Architecture
//!
//! - **One global multi-threaded Tokio runtime** — initialized once, used everywhere
//! - **All async execution** (HTTP server, go blocks, scopes) runs on this runtime
//! - **Pure ownership** — no Rc/Arc exposed to user code; handles are opaque pointers
//!
//! ## Modules
//!
//! - `runtime` — Global runtime init/shutdown/block_on
//! - `task` — Spawn, sleep, timeout FFI functions
//! - `scope` — Structured concurrency (JoinSet-based)
//! - `task_handle` — Awaitable/cancellable task handle

pub mod runtime;
pub mod scope;
pub mod task;
pub mod task_handle;
