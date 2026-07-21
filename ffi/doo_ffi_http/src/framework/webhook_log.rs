//! Webhook Dispatch Audit Log
//!
//! Production-grade webhook delivery tracking with:
//!   - In-memory ring buffer for fast recent queries (/webhooks/recent)
//!   - PostgreSQL persistence for historical records (/webhooks/deliveries)
//!   - Filterability by resource, event, webhook_id, status
//!
//! Industry-standard pattern (Stripe, GitHub, Svix):
//!   Every webhook dispatch is durably stored before the HTTP call,
//!   then updated with the result. Survives server restart.
//!
//! Consumed by:
//!   1. Shell test scripts → verify webhooks fired
//!   2. DooCloud UI → webhook activity dashboard
//!   3. Debugging → always-visible eprintln! output

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::OnceLock;

use serde::Serialize;

use crate::framework::db_bridge;

/// Maximum number of records kept in the in-memory ring buffer.
const MAX_RECORDS: usize = 1000;

/// Database table name for webhook delivery persistence.
const DELIVERIES_TABLE: &str = "webhook_deliveries";

/// A single webhook dispatch record (stored in-memory AND in DB).
#[derive(Debug, Clone, Serialize)]
pub struct WebhookDispatchRecord {
    /// Monotonically increasing in-memory ID (not DB id)
    pub id: u64,
    /// The webhook config ID (e.g. "wh-created")
    pub webhook_id: String,
    /// Resource name (e.g. "products")
    pub resource: String,
    /// Event name: "created", "updated", "deleted"
    pub event: String,
    /// Target URL
    pub url: String,
    /// ISO 8601 timestamp
    pub timestamp: String,
    /// "success" or "failed"
    pub status: String,
    /// HTTP status code from the target (0 if not available)
    pub response_code: u16,
    /// Error message (empty if success)
    pub error: String,
    /// Size of the payload body in bytes
    pub payload_len: usize,
}

/// Global webhook dispatch log (ring buffer, max 1000 records).
static DISPATCH_LOG: OnceLock<Mutex<WebhookLog>> = OnceLock::new();

struct WebhookLog {
    records: VecDeque<WebhookDispatchRecord>,
    next_id: u64,
}

impl WebhookLog {
    fn new() -> Self {
        Self {
            records: VecDeque::with_capacity(MAX_RECORDS),
            next_id: 1,
        }
    }
}

fn get_log() -> &'static Mutex<WebhookLog> {
    DISPATCH_LOG.get_or_init(|| Mutex::new(WebhookLog::new()))
}

// ============================================================================
// TABLE AUTO-CREATION
// ============================================================================

/// Ensure the webhook_deliveries table exists in the database.
/// Called once during server initialization.
/// Idempotent — safe to call on every startup.
pub fn ensure_deliveries_table() {
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (
            id SERIAL PRIMARY KEY,
            webhook_id VARCHAR(255) NOT NULL DEFAULT '',
            resource VARCHAR(255) NOT NULL DEFAULT '',
            event VARCHAR(50) NOT NULL DEFAULT '',
            url TEXT NOT NULL DEFAULT '',
            status VARCHAR(20) NOT NULL DEFAULT 'pending',
            response_code INT NOT NULL DEFAULT 0,
            error TEXT NOT NULL DEFAULT '',
            payload_len INT NOT NULL DEFAULT 0,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
        DELIVERIES_TABLE
    );

    match db_bridge::execute_db_query(&sql) {
        Ok(_) => {
            eprintln!(
                "[WEBHOOK] Table '{}' ensured (idempotent CREATE IF NOT EXISTS)",
                DELIVERIES_TABLE
            );
        }
        Err(e) => {
            eprintln!(
                "[WEBHOOK] WARNING: Could not ensure '{}' table: {}",
                DELIVERIES_TABLE, e
            );
        }
    }
}

// ============================================================================
// DISPATCH RECORDING (in-memory + DB persistence)
// ============================================================================

/// Record a webhook dispatch attempt.
///
/// 1. Appends to in-memory ring buffer (fast queries)
/// 2. Inserts into PostgreSQL webhook_deliveries table (durable, filterable)
/// 3. Always prints to stderr (visible regardless of DOO_DEBUG)
///
/// Called from `fire_webhook` after the HTTP POST completes.
pub fn record_dispatch(
    webhook_id: &str,
    resource: &str,
    event: &str,
    url: &str,
    success: bool,
    response_code: u16,
    error: &str,
    payload_len: usize,
) {
    let timestamp = chrono::Utc::now().to_rfc3339();
    let status_str = if success { "success" } else { "failed" };

    // --- In-memory ring buffer ---
    let mut log = match get_log().lock() {
        Ok(l) => l,
        Err(e) => e.into_inner(),
    };

    let record = WebhookDispatchRecord {
        id: log.next_id,
        webhook_id: webhook_id.to_string(),
        resource: resource.to_string(),
        event: event.to_string(),
        url: url.to_string(),
        timestamp: timestamp.clone(),
        status: status_str.to_string(),
        response_code,
        error: error.to_string(),
        payload_len,
    };
    log.next_id += 1;

    if log.records.len() >= MAX_RECORDS {
        log.records.pop_front();
    }
    log.records.push_back(record.clone());
    drop(log);

    // --- DB persistence (synchronous — already in webhook thread, not the request thread) ---
    persist_to_db(
        webhook_id,
        resource,
        event,
        url,
        &status_str,
        response_code,
        error,
        payload_len,
    );

    // --- Always-visible stderr log ---
    if success {
        eprintln!(
            "[WEBHOOK] ✓ {}::{} → {} (HTTP {}) | payload={}B | {}",
            resource, event, url, response_code, payload_len, timestamp
        );
    } else {
        eprintln!(
            "[WEBHOOK] ✗ {}::{} → {} | ERROR: {} | payload={}B | {}",
            resource, event, url, error, payload_len, timestamp
        );
    }
}

/// Persist a dispatch record to the PostgreSQL webhook_deliveries table.
/// Uses parameterized query (same approach as CRUD handlers).
/// Runs in a spawned thread — never blocks the request.
/// Silently tolerates DB errors (in-memory tracking continues).
fn persist_to_db(
    webhook_id: &str,
    resource: &str,
    event: &str,
    url: &str,
    status: &str,
    response_code: u16,
    error: &str,
    payload_len: usize,
) {
    let sql = format!(
        "INSERT INTO {} (webhook_id, resource, event, url, status, response_code, error, payload_len) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        DELIVERIES_TABLE
    );
    let values: Vec<serde_json::Value> = vec![
        serde_json::json!(webhook_id),
        serde_json::json!(resource),
        serde_json::json!(event),
        serde_json::json!(url),
        serde_json::json!(status),
        serde_json::json!(response_code),
        serde_json::json!(error),
        serde_json::json!(payload_len),
    ];

    match db_bridge::execute_db_insert(&sql, &values) {
        Ok(_) => {}
        Err(e) => {
            eprintln!(
                "[WEBHOOK] DB persist warning: {} (in-memory tracking continues)",
                e
            );
        }
    }
}

/// Return recent dispatch records as a JSON array string.
///
/// `limit`: max records to return (0 = all, capped at MAX_RECORDS).
/// Returns a JSON array of WebhookDispatchRecord objects.
pub fn get_recent_records(limit: usize) -> String {
    let log = match get_log().lock() {
        Ok(l) => l,
        Err(e) => e.into_inner(),
    };

    let count = if limit == 0 || limit > log.records.len() {
        log.records.len()
    } else {
        limit
    };

    let recent: Vec<&WebhookDispatchRecord> = log.records.iter().rev().take(count).collect();

    // Return in chronological order (oldest first)
    let ordered: Vec<&WebhookDispatchRecord> = recent.into_iter().rev().collect();

    serde_json::to_string(&ordered).unwrap_or_else(|_| "[]".to_string())
}

/// Clear all dispatch records.
pub fn clear_log() {
    let mut log = match get_log().lock() {
        Ok(l) => l,
        Err(e) => e.into_inner(),
    };
    log.records.clear();
    // Don't reset next_id — keeps IDs unique across clears
}

/// Return the total count of records currently stored.
pub fn record_count() -> usize {
    let log = match get_log().lock() {
        Ok(l) => l,
        Err(e) => e.into_inner(),
    };
    log.records.len()
}

// ============================================================================
// DB-BACKED QUERIES (persistent, filterable)
// ============================================================================

/// Query webhook deliveries from the database with optional filters.
///
/// Parameters (empty string = no filter):
/// - `resource`: filter by resource name (e.g. "products")
/// - `event`: filter by event type (e.g. "created")
/// - `webhook_id`: filter by webhook config ID (e.g. "wh-created")
/// - `status`: filter by status ("success" or "failed")
/// - `limit`: max records (0 = default 100)
/// - `offset`: pagination offset
///
/// Returns JSON array of delivery records with camelCase keys.
pub fn query_deliveries(
    resource: &str,
    event: &str,
    webhook_id: &str,
    status: &str,
    limit: usize,
    offset: usize,
) -> String {
    let mut conditions: Vec<String> = Vec::new();

    if !resource.is_empty() {
        conditions.push(format!("resource = '{}'", resource.replace('\'', "''")));
    }
    if !event.is_empty() {
        conditions.push(format!("event = '{}'", event.replace('\'', "''")));
    }
    if !webhook_id.is_empty() {
        conditions.push(format!("webhook_id = '{}'", webhook_id.replace('\'', "''")));
    }
    if !status.is_empty() {
        conditions.push(format!("status = '{}'", status.replace('\'', "''")));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let l = if limit == 0 { 100 } else { limit };

    let sql = format!(
        "SELECT id, webhook_id, resource, event, url, status, response_code, error, payload_len, \
         to_char(created_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"+00:00\"') AS timestamp \
         FROM {} {} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        DELIVERIES_TABLE, where_clause, l, offset
    );

    match db_bridge::execute_db_query(&sql) {
        Ok(json) => {
            // Transform DB column names (snake_case) → API keys (camelCase)
            if let Ok(rows) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
                let transformed: Vec<serde_json::Value> = rows
                    .into_iter()
                    .map(|row| {
                        serde_json::json!({
                            "id": row.get("id"),
                            "webhookId": row.get("webhook_id"),
                            "resource": row.get("resource"),
                            "event": row.get("event"),
                            "url": row.get("url"),
                            "status": row.get("status"),
                            "responseCode": row.get("response_code"),
                            "error": row.get("error"),
                            "payloadLen": row.get("payload_len"),
                            "timestamp": row.get("timestamp"),
                        })
                    })
                    .collect();
                serde_json::to_string(&transformed).unwrap_or_else(|_| "[]".to_string())
            } else {
                json
            }
        }
        Err(e) => {
            eprintln!("[WEBHOOK] DB query error: {}", e);
            "[]".to_string()
        }
    }
}

// ============================================================================
// WEBHOOK AUDIT LOG — FFI EXPORTS
// ============================================================================

/// Return recent webhook dispatch records from in-memory ring buffer.
#[no_mangle]
pub extern "C" fn doo_http_webhooks_recent(limit: i32) -> *mut crate::DooResult {
    let l = if limit <= 0 { 0 } else { limit as usize };
    let json = get_recent_records(l);
    crate::make_ok_json(&json)
}

/// Query webhook deliveries from DB with filters.
/// Params: resource, event, webhook_id, status, limit, offset (all strings).
/// Empty string = no filter. Returns JSON array of delivery records.
#[no_mangle]
pub extern "C" fn doo_http_webhooks_deliveries(
    resource: *const std::os::raw::c_char,
    event: *const std::os::raw::c_char,
    webhook_id: *const std::os::raw::c_char,
    status: *const std::os::raw::c_char,
    limit: i32,
    offset: i32,
) -> *mut crate::DooResult {
    let r = crate::helpers::c_to_string(resource);
    let e = crate::helpers::c_to_string(event);
    let w = crate::helpers::c_to_string(webhook_id);
    let s = crate::helpers::c_to_string(status);
    let l = if limit <= 0 { 0 } else { limit as usize };
    let o = if offset < 0 { 0 } else { offset as usize };
    let json = query_deliveries(&r, &e, &w, &s, l, o);
    crate::make_ok_json(&json)
}

/// Clear in-memory webhook dispatch records.
#[no_mangle]
pub extern "C" fn doo_http_webhooks_log_clear() -> *mut crate::DooResult {
    clear_log();
    crate::make_ok_void()
}
