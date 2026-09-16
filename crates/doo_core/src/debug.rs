//! Centralized debug logging for the Doo compiler pipeline.
//!
//! Single source of truth for all debug output across all compiler crates.

use crate::span::Span;
use crate::types::registry::{TypeId, TypeRegistry};
use std::fmt::Debug;

/// Global debug flag.
static DEBUG_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(cfg!(debug_assertions));

/// Initialize debug mode.
pub fn init(enabled: bool) {
    let should_enable = cfg!(debug_assertions)
        || enabled
        || std::env::var(crate::constants::env_vars::DOO_DEBUG).is_ok();
    DEBUG_ENABLED.store(should_enable, std::sync::atomic::Ordering::Relaxed);
}

/// Check if debug mode is currently enabled.
#[inline(always)]
pub fn is_enabled() -> bool {
    DEBUG_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Debug logging macro for compiler internals.
#[macro_export]
macro_rules! doo_debug {
    ($component:expr, $($arg:tt)*) => {
        if $crate::debug::is_enabled() {
            eprintln!("[{}] {}", $component, format!($($arg)*));
        }
    };
}

/// Fatal error macro — always prints regardless of debug mode.
#[macro_export]
macro_rules! doo_fatal {
    ($($arg:tt)*) => {
        eprintln!("[FATAL] {}", format!($($arg)*));
    };
}

/// Format a span for debug output (file:line:col format).
/// Note: Requires SourceMap from doo_diagnostics for full resolution.
/// This is a simplified version using byte offsets.
pub fn debug_span(span: Span) -> String {
    format!("{}..{}", span.start, span.end)
}

/// Format a type for debug output using the type registry.
pub fn debug_type(ty: TypeId, registry: &TypeRegistry) -> String {
    registry.display_name(ty)
}

/// Format any AST/HIR/MIR node for debug output.
pub fn debug_ast_node(node: &dyn Debug) -> String {
    format!("{:#?}", node)
}
