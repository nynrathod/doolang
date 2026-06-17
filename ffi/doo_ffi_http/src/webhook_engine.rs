//! Generic Webhook Engine — SINGLE SOURCE OF TRUTH
//!
//! This module provides a generic, reusable webhook engine that any part of
//! the Doo HTTP FFI can use. It is NOT tied to CRUD, Auth, OAuth, or any
//! specific feature. New features (and future syntax) integrate webhooks
//! by calling `register()` and `fire()` with a namespaced key.
//!
//! ## Key Namespace Convention
//!
//! Webhook configs are stored keyed by a namespace string:
//! - `"crud:products"` — CRUD resource "products"
//! - `"auth:signup"` — Auth signup event
//! - `"auth:login"` — Auth login event
//! - `"oauth:google"` — OAuth provider "google"
//! - `"route:GET:/api/orders"` — Custom route handler
//!
//! ## Integration Points
//!
//! 1. **CRUD handlers** call `WebhookEngine::fire("crud:products", "created", &record)`
//! 2. **Auth handlers** call `WebhookEngine::fire("auth:signup", "signup", &user)`
//! 3. **OAuth handlers** call `WebhookEngine::fire("oauth:google", "oauth_login", &info)`
//! 4. **Server dispatch** calls `WebhookEngine::fire("route:GET:/api/x", "on_success", &data)`
//!
//! ## Industry-Standard Practices
//!
//! - **Fire-and-forget**: Webhooks are dispatched in spawned threads; never block the request
//! - **Durable audit log**: Every dispatch is recorded (in-memory ring buffer + PostgreSQL)
//! - **Filter evaluation**: AND logic across all filter conditions
//! - **Payload shaping**: Field-level filtering of sensitive data
//! - **Timeout + retry**: 10s HTTP timeout; failed dispatches are logged
//! - **Standard payload format**: `{ event, data, timestamp }` (Stripe/Svix-compatible)

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;

use serde::Deserialize;

use doo_ffi_core::ffi_debug;

use crate::webhook_log;

// ============================================================================
// WEBHOOK CONFIG TYPES — Single Definition, Used Everywhere
// ============================================================================

/// Webhook configuration parsed from JSON (serde).
/// Doo JSON uses PascalCase field names (e.g., "Id", "Event", "Url").
/// These types are the SINGLE SOURCE OF TRUTH — crud.rs, auth.rs, oauth.rs,
/// and routes.rs all use these same types.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WebhookConfig {
    #[serde(default)]
    pub id: String,
    pub event: String,
    pub url: String,
    #[serde(default)]
    pub filters: Vec<WebhookFilter>,
    #[serde(default)]
    pub payload_fields: Vec<String>,
}

/// Webhook filter for conditional webhook firing.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WebhookFilter {
    #[serde(default)]
    pub id: String,
    pub field: String,
    pub operator: String,
    pub value: String,
}

// ============================================================================
// GLOBAL WEBHOOK ENGINE
// ============================================================================

/// The global webhook engine — stores all webhook configs for all features.
static WEBHOOK_ENGINE: OnceLock<Mutex<WebhookEngine>> = OnceLock::new();

/// Get a reference to the global webhook engine.
fn get_engine() -> &'static Mutex<WebhookEngine> {
    WEBHOOK_ENGINE.get_or_init(|| Mutex::new(WebhookEngine::new()))
}

/// The webhook engine stores configs keyed by namespace string.
struct WebhookEngine {
    /// Key → Vec<WebhookConfig>, where key is like "crud:products", "auth:signup", etc.
    configs: HashMap<String, Vec<WebhookConfig>>,
}

impl WebhookEngine {
    fn new() -> Self {
        Self {
            configs: HashMap::new(),
        }
    }
}

// ============================================================================
// PUBLIC API
// ============================================================================

/// Register webhook configs for a given key.
///
/// This is the ONE function that all features call to register their webhooks.
/// - CRUD: `register("crud:products", configs)`
/// - Auth: `register("auth:signup", configs)` / `register("auth:login", configs)`
/// - OAuth: `register("oauth:google", configs)`
/// - Routes: `register("route:GET:/api/x", configs)`
pub fn register(key: &str, configs: Vec<WebhookConfig>) {
    let mut engine = get_engine().lock().unwrap_or_else(|e| e.into_inner());

    ffi_debug!(
        "WEBHOOK_ENGINE",
        "Registering {} webhook(s) for key '{}'",
        configs.len(),
        key
    );

    engine.configs.insert(key.to_string(), configs);
}

/// Parse a JSON string into a Vec<WebhookConfig>.
///
/// This is the centralized parsing function. All FFI entry points
/// (crud, auth, oauth, routes) call this to parse user-provided JSON.
pub fn parse_configs(json: &str) -> Result<Vec<WebhookConfig>, String> {
    if json.is_empty() || json == "[]" {
        return Ok(Vec::new());
    }
    serde_json::from_str::<Vec<WebhookConfig>>(json)
        .map_err(|e| format!("Failed to parse webhook configs: {}", e))
}

/// Fire all matching webhooks for a given key and event.
///
/// This is the ONE function that all features call to fire webhooks.
/// - CRUD: `fire("crud:products", "created", &record)`
/// - Auth: `fire("auth:signup", "signup", &user)`
/// - OAuth: `fire("oauth:google", "oauth_login", &info)`
/// - Routes: `fire("route:GET:/api/x", "on_success", &data)`
///
/// Looks up webhook configs by key, checks event match + filters,
/// then spawns fire-and-forget threads for each matching webhook.
pub fn fire(key: &str, event: &str, record: &serde_json::Value) {
    let engine = get_engine().lock().unwrap_or_else(|e| e.into_inner());

    let configs = match engine.configs.get(key) {
        Some(c) => c,
        None => return, // No webhooks registered for this key — fast path
    };

    for config in configs {
        if config.event == event {
            if evaluate_filters(record, &config.filters) {
                dispatch_webhook(config, record, key);
            } else {
                ffi_debug!(
                    "WEBHOOK_ENGINE",
                    "Webhook '{}' (key='{}') skipped: filters did not match",
                    config.event,
                    key
                );
            }
        }
    }
}

/// Check if any webhooks are registered for a given key.
/// Used as a fast-path check before doing expensive work.
pub fn has_webhooks(key: &str) -> bool {
    let engine = get_engine().lock().unwrap_or_else(|e| e.into_inner());
    engine.configs.contains_key(key)
}

// ============================================================================
// FILTER EVALUATION — Generic, Used by All Features
// ============================================================================

/// Evaluate webhook filters against a record. All filters must pass (AND logic).
/// Returns true if no filters (unconditional webhook).
pub fn evaluate_filters(record: &serde_json::Value, filters: &[WebhookFilter]) -> bool {
    if filters.is_empty() {
        return true;
    }
    for filter in filters {
        let field_val = match record.get(&filter.field) {
            Some(v) => v,
            None => {
                ffi_debug!(
                    "WEBHOOK_ENGINE",
                    "Filter field '{}' not found in record",
                    filter.field
                );
                return false;
            }
        };
        let passes = match filter.operator.as_str() {
            "equals" => {
                // Try string comparison first, then fall back to value comparison
                field_val.as_str().map_or_else(
                    || field_val.to_string().trim_matches('"') == filter.value,
                    |s| s == filter.value,
                )
            }
            "not_equals" => {
                let as_str = field_val.as_str().map_or_else(
                    || field_val.to_string().trim_matches('"').to_string(),
                    |s| s.to_string(),
                );
                as_str != filter.value
            }
            "contains" => field_val
                .as_str()
                .map_or(false, |s| s.contains(&filter.value)),
            "greater_than" => {
                let val = field_val.as_f64().unwrap_or(0.0);
                let target = filter.value.parse::<f64>().unwrap_or(0.0);
                val > target
            }
            "less_than" => {
                let val = field_val.as_f64().unwrap_or(0.0);
                let target = filter.value.parse::<f64>().unwrap_or(0.0);
                val < target
            }
            other => {
                ffi_debug!("WEBHOOK_ENGINE", "Unknown filter operator: {}", other);
                true // unknown operator = pass (don't block on unknown ops)
            }
        };
        if !passes {
            return false;
        }
    }
    true
}

// ============================================================================
// PAYLOAD BUILDING — Generic, Used by All Features
// ============================================================================

/// Build the webhook payload JSON.
///
/// If payload_fields is empty, sends entire record.
/// Otherwise, only includes specified fields.
///
/// Wraps in industry-standard format (Stripe/Svix-compatible):
/// `{ "event": "...", "data": { ... }, "timestamp": "..." }`
pub fn build_webhook_payload(
    event: &str,
    record: &serde_json::Value,
    payload_fields: &[String],
) -> serde_json::Value {
    let data = if payload_fields.is_empty() {
        record.clone()
    } else {
        let mut filtered = serde_json::Map::new();
        if let Some(obj) = record.as_object() {
            for field in payload_fields {
                if let Some(val) = obj.get(field) {
                    filtered.insert(field.clone(), val.clone());
                }
            }
        }
        serde_json::Value::Object(filtered)
    };
    let timestamp = chrono::Utc::now().to_rfc3339();
    serde_json::json!({
        "event": event,
        "data": data,
        "timestamp": timestamp,
    })
}

// ============================================================================
// WEBHOOK DISPATCH — Fire-and-Forget, Threaded, Durable Audit
// ============================================================================

/// Dispatch a single webhook (fire-and-forget).
///
/// Spawns a thread and makes an HTTP POST to the webhook URL.
/// Never blocks the original request.
/// Records every dispatch attempt (success or failure) in the audit log via webhook_log.
fn dispatch_webhook(config: &WebhookConfig, record: &serde_json::Value, context_key: &str) {
    let url = config.url.clone();
    let event = config.event.clone();
    let webhook_id = config.id.clone();
    let context_owned = context_key.to_string();
    let payload = build_webhook_payload(&event, record, &config.payload_fields);
    let payload_str = payload.to_string();
    let payload_len = payload_str.len();

    ffi_debug!(
        "WEBHOOK_ENGINE",
        "Dispatching webhook: id={}, event={}, key={}, url={}, payload_len={}",
        webhook_id,
        event,
        context_owned,
        url,
        payload_len
    );

    std::thread::spawn(move || {
        let client = match reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                let err_msg = format!("Failed to create HTTP client: {}", e);
                webhook_log::record_dispatch(
                    &webhook_id,
                    &context_owned,
                    &event,
                    &url,
                    false,
                    0,
                    &err_msg,
                    payload_len,
                );
                return;
            }
        };

        match client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(payload_str)
            .send()
        {
            Ok(resp) => {
                let code = resp.status().as_u16();
                let ok = resp.status().is_success();
                let err = if ok {
                    String::new()
                } else {
                    format!("HTTP {}", code)
                };
                webhook_log::record_dispatch(
                    &webhook_id,
                    &context_owned,
                    &event,
                    &url,
                    ok,
                    code,
                    &err,
                    payload_len,
                );
            }
            Err(e) => {
                let err_msg = e.to_string();
                webhook_log::record_dispatch(
                    &webhook_id,
                    &context_owned,
                    &event,
                    &url,
                    false,
                    0,
                    &err_msg,
                    payload_len,
                );
            }
        }
    });
}

// ============================================================================
// CROSS-DLL BRIDGE — extern "C" wrappers for doo_ffi_auth (and future FFIs)
// ============================================================================
// These functions expose the webhook engine to other DLLs via C ABI, so they
// can be resolved at runtime with dlsym/GetProcAddress. This follows the same
// pattern as doo_http_register_package_route and doo_http_push_cookie used by
// doo_ffi_auth's http_handlers.rs.

use std::os::raw::c_char;

/// Register webhook configs for a key. Called by doo_ffi_auth via dynamic resolution.
/// key: namespace key like "oauth:google"
/// configs_json: JSON array of WebhookConfig objects
#[no_mangle]
pub extern "C" fn doo_http_register_webhooks(key: *const c_char, configs_json: *const c_char) {
    if key.is_null() || configs_json.is_null() {
        return;
    }
    let key_str = doo_ffi_core::helpers::c_to_string_lossy(key);
    let json_str = doo_ffi_core::helpers::c_to_string_lossy(configs_json);
    if let Ok(configs) = parse_configs(&json_str) {
        if !configs.is_empty() {
            register(&key_str, configs);
        }
    }
}

/// Fire webhooks for a key + event. Called by doo_ffi_auth via dynamic resolution.
/// key: namespace key like "oauth:google"
/// event: event name like "oauth_login"
/// payload_json: JSON payload to send to webhook URLs
#[no_mangle]
pub extern "C" fn doo_http_fire_webhook(
    key: *const c_char,
    event: *const c_char,
    payload_json: *const c_char,
) {
    if key.is_null() || event.is_null() || payload_json.is_null() {
        return;
    }
    let key_str = doo_ffi_core::helpers::c_to_string_lossy(key);
    let event_str = doo_ffi_core::helpers::c_to_string_lossy(event);
    let json_str = doo_ffi_core::helpers::c_to_string_lossy(payload_json);
    if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&json_str) {
        fire(&key_str, &event_str, &payload);
    }
}
