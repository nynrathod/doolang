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
//! ## Safety Guarantees (Production-Grade)
//!
//! - **No panics cross FFI boundary** — all `extern "C"` functions wrapped in `catch_unwind`
//! - **No nested `block_on` panics** — `safe_block_on` uses `block_in_place` when needed
//! - **No thread pool exhaustion** — detached tasks limited by semaphore-like counter
//! - **No TOCTOU races** — `get_or_init` for runtime initialization
//! - **Shutdown-safe** — all spawn operations check `is_shutdown()` before proceeding
//! - **Mutex-protected scopes** — `ScopeHandle` uses `Mutex<JoinSet>` for safe concurrent access
//! - **No memory leaks on error** — scope tracks Ok results and frees them on error path
//!
//! ## Modules
//!
//! - `runtime` — Global runtime init/shutdown/block_on + `safe_block_on` helper
//! - `task` — Spawn, sleep, timeout FFI functions
//! - `scope` — Structured concurrency (JoinSet-based)
//! - `task_handle` — Awaitable/cancellable task handle

pub mod runtime;
pub mod scope;
pub mod task;
pub mod task_handle;
