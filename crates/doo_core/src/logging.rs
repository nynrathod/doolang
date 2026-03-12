//! Structured Logging — `tracing`-based instrumentation for the Doo compiler.
//!
//! Provides structured spans and events alongside the existing `doo_debug!` system.
//! Use these macros for structured data that can be filtered and queried.
//!
//! ## Usage
//!
//! ```ignore
//! use doo_core::logging;
//!
//! // Create a timed span around a compilation phase:
//! let _span = logging::phase_span("codegen", "main.doo");
//!
//! // Emit structured events:
//! logging::event_info("codegen", "Generated function", &[("name", "main"), ("blocks", "5")]);
//! ```
//!
//! ## Design
//!
//! - Built on the `tracing` crate for structured, zero-cost-when-disabled logging
//! - Each compiler phase gets its own tracing span
//! - Events carry structured key-value fields, not just formatted strings
//! - Subscribers can filter by phase, level, or field values

pub use tracing::{self, debug, error, info, trace, warn};
pub use tracing::{debug_span, error_span, info_span, span, trace_span, warn_span};

/// Create a span for a compiler phase.
/// Returns a guard that ends the span when dropped.
///
/// # Example
/// ```ignore
/// let _guard = phase_span("parse", "main.doo");
/// // ... parsing work ...
/// // span ends when _guard is dropped
/// ```
#[inline]
pub fn phase_span(phase: &str, file: &str) -> tracing::span::EnteredSpan {
    let span = tracing::info_span!("compiler_phase", phase = phase, file = file);
    span.entered()
}

/// Emit a structured info event with key-value fields.
#[inline]
pub fn event_info(component: &str, message: &str, fields: &[(&str, &str)]) {
    // Build a formatted field string for structured output
    let field_str: String = fields
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(" ");
    tracing::info!(component = component, fields = %field_str, "{}", message);
}

/// Emit a structured warning event.
#[inline]
pub fn event_warn(component: &str, message: &str, fields: &[(&str, &str)]) {
    let field_str: String = fields
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(" ");
    tracing::warn!(component = component, fields = %field_str, "{}", message);
}

/// Emit a structured error event.
#[inline]
pub fn event_error(component: &str, message: &str, fields: &[(&str, &str)]) {
    let field_str: String = fields
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(" ");
    tracing::error!(component = component, fields = %field_str, "{}", message);
}

/// Compiler-phase-specific span macro.
///
/// Usage: `doo_span!("codegen", "func_name" = "main")`
#[macro_export]
macro_rules! doo_span {
    ($phase:expr) => {
        $crate::logging::info_span!("doo", phase = $phase).entered()
    };
    ($phase:expr, $($key:expr => $val:expr),+ $(,)?) => {
        $crate::logging::info_span!("doo", phase = $phase, $($key = $val),+).entered()
    };
}
