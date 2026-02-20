//! Metrics Module — Prometheus-Compatible Metrics Endpoint
//!
//! Provides request counting, latency tracking, and error rate monitoring.
//! Enabled via `app.metrics()` which auto-registers a GET `/metrics` endpoint.
//!
//! ## Prometheus Format Output
//!
//! ```text
//! # HELP doo_http_requests_total Total number of HTTP requests
//! # TYPE doo_http_requests_total counter
//! doo_http_requests_total{method="GET",path="/api/users",status="200"} 1234
//!
//! # HELP doo_http_request_duration_seconds HTTP request duration in seconds
//! # TYPE doo_http_request_duration_seconds histogram
//! doo_http_request_duration_seconds_sum 45.678
//! doo_http_request_duration_seconds_count 1234
//!
//! # HELP doo_http_active_requests Current number of in-flight requests
//! # TYPE doo_http_active_requests gauge
//! doo_http_active_requests 5
//! ```
//!
//! ## Thread Safety
//!
//! All counters use `AtomicU64` (lock-free). Route-level metrics use `DashMap`.
//! Zero per-request allocation for counter increments.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;

use dashmap::DashMap;

// ============================================================================
// Global Metrics State — Lock-Free Atomics
// ============================================================================

/// Whether metrics collection is enabled
static METRICS_ENABLED: AtomicBool = AtomicBool::new(false);

/// Total requests processed
static TOTAL_REQUESTS: AtomicU64 = AtomicU64::new(0);

/// Total successful responses (2xx)
static TOTAL_SUCCESS: AtomicU64 = AtomicU64::new(0);

/// Total client errors (4xx)
static TOTAL_CLIENT_ERRORS: AtomicU64 = AtomicU64::new(0);

/// Total server errors (5xx)
static TOTAL_SERVER_ERRORS: AtomicU64 = AtomicU64::new(0);

/// Cumulative request duration in microseconds (for computing average)
static TOTAL_DURATION_US: AtomicU64 = AtomicU64::new(0);

/// Per-route metrics: "METHOD /path" → RouteMetrics
static ROUTE_METRICS: OnceLock<DashMap<String, RouteMetrics>> = OnceLock::new();

fn get_route_metrics() -> &'static DashMap<String, RouteMetrics> {
    ROUTE_METRICS.get_or_init(DashMap::new)
}

/// Metrics for a specific route
#[derive(Debug, Default)]
pub struct RouteMetrics {
    pub request_count: AtomicU64,
    pub success_count: AtomicU64,
    pub error_count: AtomicU64,
    pub total_duration_us: AtomicU64,
    /// Track status code distribution
    pub status_2xx: AtomicU64,
    pub status_4xx: AtomicU64,
    pub status_5xx: AtomicU64,
}

// ============================================================================
// Metrics Recording — Called from server.rs hot path
// ============================================================================

/// Check if metrics collection is enabled
#[inline]
pub fn is_metrics_enabled() -> bool {
    METRICS_ENABLED.load(Ordering::Relaxed)
}

/// Record a completed request.
/// Called after every HTTP response is sent.
/// Uses relaxed ordering — exact counts not needed, throughput matters.
#[inline]
pub fn record_request(method: &str, path: &str, status: u16, duration_us: u64) {
    if !is_metrics_enabled() {
        return;
    }

    // Global counters (atomic, lock-free)
    TOTAL_REQUESTS.fetch_add(1, Ordering::Relaxed);
    TOTAL_DURATION_US.fetch_add(duration_us, Ordering::Relaxed);

    match status {
        200..=299 => {
            TOTAL_SUCCESS.fetch_add(1, Ordering::Relaxed);
        }
        400..=499 => {
            TOTAL_CLIENT_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        500..=599 => {
            TOTAL_SERVER_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }

    // Per-route metrics (DashMap — concurrent safe, minimal contention)
    let key = format!("{} {}", method, path);
    let metrics = get_route_metrics();

    // Use entry API for atomic get-or-insert
    let entry = metrics.entry(key).or_insert_with(RouteMetrics::default);
    entry.request_count.fetch_add(1, Ordering::Relaxed);
    entry.total_duration_us.fetch_add(duration_us, Ordering::Relaxed);

    match status {
        200..=299 => {
            entry.success_count.fetch_add(1, Ordering::Relaxed);
            entry.status_2xx.fetch_add(1, Ordering::Relaxed);
        }
        400..=499 => {
            entry.error_count.fetch_add(1, Ordering::Relaxed);
            entry.status_4xx.fetch_add(1, Ordering::Relaxed);
        }
        500..=599 => {
            entry.error_count.fetch_add(1, Ordering::Relaxed);
            entry.status_5xx.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            entry.success_count.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ============================================================================
// Metrics Endpoint — Prometheus Text Format
// ============================================================================

/// Generate Prometheus-compatible metrics text output.
/// This is called when GET /metrics is hit.
pub fn render_metrics() -> String {
    let total_requests = TOTAL_REQUESTS.load(Ordering::Relaxed);
    let total_success = TOTAL_SUCCESS.load(Ordering::Relaxed);
    let total_client_errors = TOTAL_CLIENT_ERRORS.load(Ordering::Relaxed);
    let total_server_errors = TOTAL_SERVER_ERRORS.load(Ordering::Relaxed);
    let total_duration_us = TOTAL_DURATION_US.load(Ordering::Relaxed);

    let avg_duration_ms = if total_requests > 0 {
        (total_duration_us as f64 / total_requests as f64) / 1000.0
    } else {
        0.0
    };

    let total_duration_s = total_duration_us as f64 / 1_000_000.0;

    let uptime_s = crate::server::startup_uptime_secs();

    let mut output = String::with_capacity(4096);

    // Global counters
    output.push_str("# HELP doo_http_requests_total Total number of HTTP requests processed\n");
    output.push_str("# TYPE doo_http_requests_total counter\n");
    output.push_str(&format!("doo_http_requests_total {}\n\n", total_requests));

    output.push_str(
        "# HELP doo_http_requests_success_total Total successful responses (2xx)\n",
    );
    output.push_str("# TYPE doo_http_requests_success_total counter\n");
    output.push_str(&format!(
        "doo_http_requests_success_total {}\n\n",
        total_success
    ));

    output.push_str(
        "# HELP doo_http_requests_client_error_total Total client errors (4xx)\n",
    );
    output.push_str("# TYPE doo_http_requests_client_error_total counter\n");
    output.push_str(&format!(
        "doo_http_requests_client_error_total {}\n\n",
        total_client_errors
    ));

    output.push_str(
        "# HELP doo_http_requests_server_error_total Total server errors (5xx)\n",
    );
    output.push_str("# TYPE doo_http_requests_server_error_total counter\n");
    output.push_str(&format!(
        "doo_http_requests_server_error_total {}\n\n",
        total_server_errors
    ));

    // Duration stats
    output.push_str(
        "# HELP doo_http_request_duration_seconds Total request processing time\n",
    );
    output.push_str("# TYPE doo_http_request_duration_seconds summary\n");
    output.push_str(&format!(
        "doo_http_request_duration_seconds_sum {:.6}\n",
        total_duration_s
    ));
    output.push_str(&format!(
        "doo_http_request_duration_seconds_count {}\n\n",
        total_requests
    ));

    output.push_str(
        "# HELP doo_http_request_duration_avg_ms Average request duration in milliseconds\n",
    );
    output.push_str("# TYPE doo_http_request_duration_avg_ms gauge\n");
    output.push_str(&format!(
        "doo_http_request_duration_avg_ms {:.3}\n\n",
        avg_duration_ms
    ));

    // Error rate
    let error_rate = if total_requests > 0 {
        ((total_client_errors + total_server_errors) as f64 / total_requests as f64) * 100.0
    } else {
        0.0
    };
    output.push_str("# HELP doo_http_error_rate_percent Percentage of requests resulting in errors\n");
    output.push_str("# TYPE doo_http_error_rate_percent gauge\n");
    output.push_str(&format!(
        "doo_http_error_rate_percent {:.2}\n\n",
        error_rate
    ));

    // Uptime
    output.push_str("# HELP doo_process_uptime_seconds Server uptime in seconds\n");
    output.push_str("# TYPE doo_process_uptime_seconds gauge\n");
    output.push_str(&format!("doo_process_uptime_seconds {}\n\n", uptime_s));

    // Per-route metrics
    let route_map = get_route_metrics();
    if !route_map.is_empty() {
        output.push_str(
            "# HELP doo_http_route_requests_total Requests per route\n",
        );
        output.push_str("# TYPE doo_http_route_requests_total counter\n");

        // Collect and sort for stable output
        let mut routes: Vec<_> = route_map
            .iter()
            .map(|entry| {
                let key = entry.key().clone();
                let count = entry.request_count.load(Ordering::Relaxed);
                let success = entry.status_2xx.load(Ordering::Relaxed);
                let err_4xx = entry.status_4xx.load(Ordering::Relaxed);
                let err_5xx = entry.status_5xx.load(Ordering::Relaxed);
                let dur_us = entry.total_duration_us.load(Ordering::Relaxed);
                (key, count, success, err_4xx, err_5xx, dur_us)
            })
            .collect();
        routes.sort_by(|a, b| a.0.cmp(&b.0));

        for (key, count, success, err_4xx, err_5xx, dur_us) in &routes {
            let parts: Vec<&str> = key.splitn(2, ' ').collect();
            let (method, path) = if parts.len() == 2 {
                (parts[0], parts[1])
            } else {
                ("UNKNOWN", key.as_str())
            };

            let avg_ms = if *count > 0 {
                (*dur_us as f64 / *count as f64) / 1000.0
            } else {
                0.0
            };

            output.push_str(&format!(
                "doo_http_route_requests_total{{method=\"{}\",path=\"{}\"}} {}\n",
                method, path, count
            ));
            output.push_str(&format!(
                "doo_http_route_requests_success{{method=\"{}\",path=\"{}\"}} {}\n",
                method, path, success
            ));
            output.push_str(&format!(
                "doo_http_route_requests_4xx{{method=\"{}\",path=\"{}\"}} {}\n",
                method, path, err_4xx
            ));
            output.push_str(&format!(
                "doo_http_route_requests_5xx{{method=\"{}\",path=\"{}\"}} {}\n",
                method, path, err_5xx
            ));
            output.push_str(&format!(
                "doo_http_route_avg_duration_ms{{method=\"{}\",path=\"{}\"}} {:.3}\n",
                method, path, avg_ms
            ));
        }
    }

    output
}

// ============================================================================
// FFI — Enable metrics on server
// ============================================================================

/// Enable Prometheus metrics collection and register /metrics endpoint.
/// Called from Doo code: `app.metrics()`
///
/// This function:
/// 1. Enables the atomic `METRICS_ENABLED` flag
/// 2. The server checks this flag on every request to record metrics
/// 3. `/metrics` is handled as a built-in route (like /health)
#[no_mangle]
pub extern "C" fn doo_http_metrics(
    server: *mut std::ffi::c_void,
) -> *mut std::ffi::c_void {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        doo_ffi_core::ffi_debug!("METRICS", "Enabling Prometheus metrics endpoint at /metrics");
        METRICS_ENABLED.store(true, Ordering::Release);
        server
    })) {
        Ok(result) => result,
        Err(_) => server,
    }
}
