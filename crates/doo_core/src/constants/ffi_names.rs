//! FFI Function Names - Single Source of Truth
//!
//! ALL FFI function name strings are centralized here.
//! This file contains ONLY Tier A ABI symbols:
//! - C standard library functions (used by compiler internally)
//! - Doo core runtime functions (like Rust's __rust_alloc)
//! - Doo async runtime functions
//!
//! NO JSON, NO module names, NO library method names.
//! Those belong in library/std/*.doo as @extern declarations.

// ============================================================================
// Compiler Internal Names
// ============================================================================

/// Name used for anonymous object literals `{ key: value }` in HIR.
/// ObjectLit is lowered to `HirExprKind::Struct { name: OBJECT_LIT_NAME, .. }`
/// and compiled to a `HashMap<String, String>` at codegen time.
pub const OBJECT_LIT_NAME: &str = "__anon";

/// Check if a struct name is an anonymous object literal
#[inline]
pub fn is_object_lit(name: &str) -> bool {
    name == OBJECT_LIT_NAME
}

// ============================================================================
// Standard C Library Functions
// ============================================================================

pub const MALLOC: &str = "malloc";
pub const FREE: &str = "free";
pub const REALLOC: &str = "realloc";
pub const MEMCPY: &str = "memcpy";
pub const MEMSET: &str = "memset";
pub const MEMMOVE: &str = "memmove";
pub const STRLEN: &str = "strlen";
pub const STRCMP: &str = "strcmp";
pub const STRSTR: &str = "strstr";
pub const STRNCMP: &str = "strncmp";
pub const STRCPY: &str = "strcpy";
pub const STRCAT: &str = "strcat";
pub const PRINTF: &str = "printf";
pub const SNPRINTF: &str = "snprintf";
pub const PUTCHAR: &str = "putchar";
pub const PUTS: &str = "puts";
pub const SPRINTF: &str = "sprintf";
pub const EXIT: &str = "exit";
pub const FFLUSH: &str = "fflush";

// ============================================================================
// Doo Core Runtime Functions
// ============================================================================

pub const DOO_ALLOC: &str = "doo_alloc";
pub const DOO_FREE: &str = "doo_free";
pub const DOO_REALLOC: &str = "doo_realloc";

// ============================================================================
// Doo Async Runtime Functions
// ============================================================================

pub const DOO_SPAWN: &str = "doo_spawn";
pub const DOO_SPAWN_DETACH: &str = "doo_spawn_detach";
pub const DOO_SPAWN_BLOCKING: &str = "doo_spawn_blocking";
pub const DOO_SCOPE_CREATE: &str = "doo_scope_create";
pub const DOO_SCOPE_SPAWN: &str = "doo_scope_spawn";
pub const DOO_SCOPE_WAIT: &str = "doo_scope_wait";
pub const DOO_SCOPE_FREE: &str = "doo_scope_free";
pub const DOO_TASK_AWAIT: &str = "doo_task_await";
pub const DOO_TASK_CANCEL: &str = "doo_task_cancel";
pub const DOO_TASK_FREE: &str = "doo_task_free";
pub const DOO_RUNTIME_INIT: &str = "doo_runtime_init";
pub const DOO_RUNTIME_BLOCK_ON: &str = "doo_runtime_block_on";
pub const DOO_SLEEP: &str = "doo_sleep";
pub const DOO_SLEEP_ASYNC: &str = "doo_sleep_async";
pub const DOO_TIMEOUT: &str = "doo_timeout";
