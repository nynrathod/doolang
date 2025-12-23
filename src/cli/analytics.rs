//! Anonymous usage analytics for Doo using PostHog
//!
//! Privacy-first analytics that tracks aggregate usage patterns without
//! collecting any private data. All calls are fire-and-forget and never
//! block user operations.
//!
//! PostHog Dashboard: https://app.posthog.com
//! To get your API key: Sign up → Project Settings → Project API Key

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

/// PostHog Cloud endpoint (US region)
const POSTHOG_HOST: &str = "https://us.i.posthog.com";

/// PostHog Project API Key - Get yours from https://app.posthog.com → Project Settings
/// This is a PUBLIC key (safe to embed in code) - it can only send events, not read data
const POSTHOG_API_KEY: &str = "phc_AdLC9AceWDZJEy04XZKnXIGwG3voUoMwyOpXTOebUet";

/// Request timeout in seconds (short to never block user)
const TIMEOUT_SECS: u64 = 2;

/// Event names
pub const EVENT_INSTALL_COMPLETE: &str = "install_complete";
pub const EVENT_PROJECT_CREATED: &str = "project_created";
pub const EVENT_DEPLOY_ATTEMPT: &str = "deploy_attempt";
pub const EVENT_DEPLOY_SUCCESS: &str = "deploy_success";
pub const EVENT_DEPLOY_ERROR: &str = "deploy_error";

/// Get the current OS identifier
pub fn get_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

/// Get the current doo version from Cargo.toml
fn get_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Generate a simple anonymous distinct_id based on machine characteristics
/// This is NOT personally identifiable - just a stable hash for session tracking
fn get_anonymous_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();

    // Hash some stable machine characteristics (not personally identifiable)
    if let Ok(hostname) = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .or_else(|_| std::env::var("HOST"))
    {
        hostname.hash(&mut hasher);
    }

    // Add OS info for additional entropy
    get_os().hash(&mut hasher);

    format!("doo_{:x}", hasher.finish())
}

/// Track an analytics event with properties
///
/// This function is fire-and-forget: it spawns a background thread,
/// never blocks, and silently ignores any errors.
///
/// # Privacy
/// - No personal data is collected
/// - Only aggregate counts matter
/// - Anonymous ID is a hash, not identifiable
pub fn track_event(event: &str, properties: HashMap<&str, String>) {
    // Skip if API key not configured
    if POSTHOG_API_KEY.contains("REPLACE") {
        return;
    }

    let event = event.to_string();
    let os = get_os().to_string();
    let version = get_version().to_string();
    let distinct_id = get_anonymous_id();

    // Clone properties for the thread
    let props: HashMap<String, String> = properties
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

    // Fire-and-forget in background thread
    thread::spawn(move || {
        let _ = send_to_posthog(&event, &distinct_id, &os, &version, &props);
    });
}

/// Track a simple event with just OS and version
pub fn track_simple(event: &str) {
    track_event(event, HashMap::new());
}

/// Track project creation event
pub fn track_project_created(template: &str) {
    let mut props = HashMap::new();
    props.insert("template", template.to_string());
    track_event(EVENT_PROJECT_CREATED, props);
}

/// Track deploy attempt
pub fn track_deploy_attempt(platform: &str) {
    let mut props = HashMap::new();
    props.insert("platform", platform.to_string());
    track_event(EVENT_DEPLOY_ATTEMPT, props);
}

/// Track deploy success
pub fn track_deploy_success(platform: &str, duration_ms: u64) {
    let mut props = HashMap::new();
    props.insert("platform", platform.to_string());
    props.insert("duration_ms", duration_ms.to_string());
    track_event(EVENT_DEPLOY_SUCCESS, props);
}

/// Track deploy error
pub fn track_deploy_error(platform: &str, error_type: &str) {
    let mut props = HashMap::new();
    props.insert("platform", platform.to_string());
    props.insert("error_type", error_type.to_string());
    track_event(EVENT_DEPLOY_ERROR, props);
}

/// Send event to PostHog using their capture API
fn send_to_posthog(
    event: &str,
    distinct_id: &str,
    os: &str,
    version: &str,
    properties: &HashMap<String, String>,
) -> Result<(), ()> {
    // Build properties object
    let mut props = serde_json::json!({
        "$os": os,
        "doo_version": version,
    });

    // Add custom properties
    if let Some(obj) = props.as_object_mut() {
        for (key, value) in properties {
            obj.insert(key.clone(), serde_json::Value::String(value.clone()));
        }
    }

    // Build PostHog capture payload
    // https://posthog.com/docs/api/capture
    let payload = serde_json::json!({
        "api_key": POSTHOG_API_KEY,
        "event": event,
        "distinct_id": distinct_id,
        "properties": props,
        "timestamp": chrono_timestamp(),
    });

    // Send HTTP POST to PostHog
    let url = format!("{}/capture/", POSTHOG_HOST);

    let result = ureq::post(&url)
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .set("Content-Type", "application/json")
        .send_json(&payload);

    match result {
        Ok(_) => Ok(()),
        Err(_) => Err(()),
    }
}

/// Get ISO 8601 timestamp
fn chrono_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let secs = duration.as_secs();
    // Simple ISO format without external crate
    format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        1970 + secs / 31536000,
        ((secs % 31536000) / 2592000) + 1,
        ((secs % 2592000) / 86400) + 1,
        (secs % 86400) / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_os() {
        let os = get_os();
        assert!(["windows", "macos", "linux", "unknown"].contains(&os));
    }

    #[test]
    fn test_get_version() {
        let version = get_version();
        assert!(!version.is_empty());
    }

    #[test]
    fn test_anonymous_id_stable() {
        let id1 = get_anonymous_id();
        let id2 = get_anonymous_id();
        assert_eq!(id1, id2); // Should be stable across calls
        assert!(id1.starts_with("doo_"));
    }
}
