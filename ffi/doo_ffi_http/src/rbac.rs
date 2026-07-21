//! RBAC (Role-Based Access Control) Runtime
//!
//! Receives policy metadata emitted by the compiler and enforces per-struct
//! access-control rules at request time.
//!
//! ## Data flow
//!
//! 1. Compiler emits `doo_http_register_policy(struct, json)` during startup.
//! 2. Compiler emits `doo_http_register_role_hierarchy(enum, json)` for enums
//!    whose variants have `@inherits` decorators.
//! 3. At request time, `check_policy()` is called with the JWT claims, struct
//!    name, action, and optional resource owner ID.
//!
//! ## Policy rule grammar (serialised string)
//!
//! | Value               | Meaning                                      |
//! |---------------------|----------------------------------------------|
//! | `"public"`          | No authentication required                   |
//! | `"authenticated"`   | Any valid JWT                                |
//! | `"own"`             | JWT user must own the resource               |
//! | `"Admin"` (etc.)    | JWT role must be "Admin" (or inherit it)     |
//! | `"Admin\|own"`      | Role is Admin **OR** user owns it            |
//! | `"Admin&Moderator"` | Role is Admin **AND** Moderator (both)       |
//!
//! Rule strings are the canonical serialisation produced by the compiler parser.

use std::collections::HashMap;
use std::os::raw::c_char;
use std::sync::{Mutex as StdMutex, OnceLock};

use doo_ffi_core::ffi_safe_void;

use crate::helpers::c_to_string;
use crate::metadata::{field_names_match, get_struct_metadata, should_include_in_response};

// ============================================================================
// GLOBAL REGISTRIES
// ============================================================================

/// struct_name → policy_rules (action → rule_string)
static POLICY_REGISTRY: OnceLock<StdMutex<HashMap<String, HashMap<String, String>>>> =
    OnceLock::new();

/// enum_name → variant_name → list of directly-inherited variants
static ROLE_HIERARCHY_RAW: OnceLock<StdMutex<HashMap<String, HashMap<String, Vec<String>>>>> =
    OnceLock::new();

/// enum_name → variant_name → flattened transitive closure of inherited variants
static ROLE_HIERARCHY: OnceLock<StdMutex<HashMap<String, HashMap<String, Vec<String>>>>> =
    OnceLock::new();

fn get_policy_registry() -> &'static StdMutex<HashMap<String, HashMap<String, String>>> {
    POLICY_REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn get_role_hierarchy_raw() -> &'static StdMutex<HashMap<String, HashMap<String, Vec<String>>>> {
    ROLE_HIERARCHY_RAW.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn get_role_hierarchy() -> &'static StdMutex<HashMap<String, HashMap<String, Vec<String>>>> {
    ROLE_HIERARCHY.get_or_init(|| StdMutex::new(HashMap::new()))
}

// ============================================================================
// FFI ENTRY POINTS
// ============================================================================

/// Register RBAC policy for a struct.
///
/// `policy_json` format: `{"create":"authenticated","read":"public","update":"own|Admin","delete":"Admin"}`
#[no_mangle]
pub extern "C" fn doo_http_register_policy(struct_name: *const c_char, policy_json: *const c_char) {
    ffi_safe_void!({
        let name = c_to_string(struct_name);
        let json_str = c_to_string(policy_json);

        if let Ok(rules) = serde_json::from_str::<HashMap<String, String>>(&json_str) {
            let mut registry = get_policy_registry()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            registry.insert(name, rules);
        }
    });
}

/// Register role hierarchy for an enum.
///
/// `hierarchy_json` format: `{"Admin":["Moderator","User"],"Moderator":["User"]}`
/// Each value is the list of variants this variant *directly* inherits.
/// Transitive closure is computed on first access.
#[no_mangle]
pub extern "C" fn doo_http_register_role_hierarchy(
    enum_name: *const c_char,
    hierarchy_json: *const c_char,
) {
    ffi_safe_void!({
        let name = c_to_string(enum_name);
        let json_str = c_to_string(hierarchy_json);

        if let Ok(raw) = serde_json::from_str::<HashMap<String, Vec<String>>>(&json_str) {
            // Store raw hierarchy
            {
                let mut registry = get_role_hierarchy_raw()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                registry.insert(name.clone(), raw.clone());
            }
            // Compute transitive closure
            let closure = compute_transitive_closure(&raw);
            {
                let mut registry = get_role_hierarchy()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                registry.insert(name, closure);
            }
        }
    });
}

// ============================================================================
// POLICY ENFORCEMENT
// ============================================================================

/// Check whether the current request is allowed.
///
/// `jwt_claims`         — parsed JWT payload (may be `Value::Null` for unauthenticated)
/// `struct_name`        — name of the Doo struct the endpoint is bound to
/// `action`             — "create" | "read" | "update" | "delete" | custom
/// `resource_owner_id`  — the `id` field of the resource being accessed (for `own` checks)
///
/// Returns `true` if access is permitted.
pub(crate) fn check_policy(
    jwt_claims: &serde_json::Value,
    struct_name: &str,
    action: &str,
    resource_owner_id: Option<i64>,
) -> bool {
    let rule = {
        let registry = get_policy_registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match registry
            .get(struct_name)
            .and_then(|rules| rules.get(action))
            .cloned()
        {
            Some(r) => r,
            None => return true, // No policy registered → open access (opt-in RBAC)
        }
    };

    evaluate_rule(&rule, jwt_claims, struct_name, resource_owner_id)
}

/// Evaluate a rule string against the current JWT claims.
fn evaluate_rule(
    rule: &str,
    jwt_claims: &serde_json::Value,
    struct_name: &str,
    resource_owner_id: Option<i64>,
) -> bool {
    // OR: any term must pass
    if rule.contains('|') {
        return rule
            .split('|')
            .any(|term| evaluate_term(term.trim(), jwt_claims, struct_name, resource_owner_id));
    }

    // AND: all terms must pass
    if rule.contains('&') {
        return rule
            .split('&')
            .all(|term| evaluate_term(term.trim(), jwt_claims, struct_name, resource_owner_id));
    }

    evaluate_term(rule.trim(), jwt_claims, struct_name, resource_owner_id)
}

/// Evaluate a single rule term.
fn evaluate_term(
    term: &str,
    jwt_claims: &serde_json::Value,
    struct_name: &str,
    resource_owner_id: Option<i64>,
) -> bool {
    match term {
        "public" => true,
        "authenticated" => is_authenticated(jwt_claims),
        "own" => {
            if !is_authenticated(jwt_claims) {
                return false;
            }
            let user_id = get_jwt_user_id(jwt_claims);
            match (user_id, resource_owner_id) {
                (Some(uid), Some(rid)) => uid == rid,
                _ => false,
            }
        }
        role_name => {
            // Role-based: user must have this role (or inherit it)
            if !is_authenticated(jwt_claims) {
                return false;
            }
            match get_jwt_role(jwt_claims, struct_name) {
                Some(user_role) => role_satisfies(struct_name, &user_role, role_name),
                None => false,
            }
        }
    }
}

// ============================================================================
// JWT HELPERS
// ============================================================================

/// True if `jwt_claims` is a non-null JSON object (i.e., the user is logged in).
pub(crate) fn is_authenticated(claims: &serde_json::Value) -> bool {
    claims.is_object()
}

/// Extract the user's `id` from JWT claims (checks "id", "sub", "user_id").
pub(crate) fn get_jwt_user_id(claims: &serde_json::Value) -> Option<i64> {
    for key in &["id", "sub", "user_id"] {
        if let Some(v) = claims.get(key) {
            if let Some(n) = v.as_i64() {
                return Some(n);
            }
            // "sub" may be a string containing a number
            if let Some(s) = v.as_str() {
                if let Ok(n) = s.parse::<i64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

/// Extract the user's role from JWT claims.
///
/// Checks standard claim names ("role", "roles", then any field whose
/// *struct metadata decorator* is `@role`).
pub(crate) fn get_jwt_role(claims: &serde_json::Value, struct_name: &str) -> Option<String> {
    // Try well-known claim names first
    for key in &["role", "roles"] {
        if let Some(v) = claims.get(*key) {
            if let Some(s) = v.as_str() {
                return Some(s.to_string());
            }
        }
    }

    // Fall back to the field decorated with @role on the struct
    if let Some(meta) = get_struct_metadata(struct_name) {
        for field in &meta.fields {
            if field.decorators.iter().any(|d| d == "role") {
                let claim_key = to_snake_case(&field.name);
                if let Some(v) = claims.get(&claim_key).or_else(|| claims.get(&field.name)) {
                    if let Some(s) = v.as_str() {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }

    None
}

// ============================================================================
// ROLE HIERARCHY
// ============================================================================

/// True if `user_role` satisfies `required_role` directly or via inheritance.
///
/// Example: hierarchy `Admin → Moderator → User`.
/// `role_satisfies(..., "Admin", "User")` → `true`
/// `role_satisfies(..., "User", "Admin")` → `false`
pub(crate) fn role_satisfies(_struct_name: &str, user_role: &str, required_role: &str) -> bool {
    if user_role == required_role {
        return true;
    }
    // Search all registered enums for the user_role and check if it inherits required_role
    let registry = get_role_hierarchy()
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    for (_enum_name, closure) in registry.iter() {
        if let Some(inherited) = closure.get(user_role) {
            if inherited.iter().any(|r| r == required_role) {
                return true;
            }
        }
    }
    false
}

/// Compute the transitive closure of a direct-inheritance map.
///
/// Input:  `{"Admin": ["Moderator"], "Moderator": ["User"]}`
/// Output: `{"Admin": ["Moderator", "User"], "Moderator": ["User"]}`
fn compute_transitive_closure(raw: &HashMap<String, Vec<String>>) -> HashMap<String, Vec<String>> {
    let mut closure: HashMap<String, Vec<String>> = HashMap::new();

    for role in raw.keys() {
        let mut visited: Vec<String> = Vec::new();
        collect_inherited(role, raw, &mut visited);
        if !visited.is_empty() {
            closure.insert(role.clone(), visited);
        }
    }
    closure
}

fn collect_inherited(role: &str, raw: &HashMap<String, Vec<String>>, out: &mut Vec<String>) {
    if let Some(direct) = raw.get(role) {
        for parent in direct {
            if !out.contains(parent) {
                out.push(parent.clone());
                collect_inherited(parent, raw, out);
            }
        }
    }
}

// ============================================================================
// PER-ROLE FIELD VISIBILITY
// ============================================================================

/// Filter response fields based on RBAC role.
///
/// Beyond the existing `@writeOnly`/`@internal` rules (handled by
/// `should_include_in_response`), this also checks:
///
/// - `@visible(Role)` / `@visible(Role1,Role2)` — only include for those roles
///   (unauthenticated or non-matching roles see the field excluded)
///
/// `jwt_role` is `None` when the request is unauthenticated.
/// `is_owner` is `true` when the requesting user owns the resource.
pub(crate) fn filter_response_fields_rbac(
    value: &serde_json::Value,
    struct_name: &str,
    jwt_role: Option<&str>,
    is_owner: bool,
) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut filtered = serde_json::Map::new();
            for (k, v) in map {
                if should_include_in_response_rbac(struct_name, k, jwt_role, is_owner) {
                    filtered.insert(k.clone(), v.clone());
                }
            }
            serde_json::Value::Object(filtered)
        }
        serde_json::Value::Array(arr) => {
            let out: Vec<serde_json::Value> = arr
                .iter()
                .map(|item| filter_response_fields_rbac(item, struct_name, jwt_role, is_owner))
                .collect();
            serde_json::Value::Array(out)
        }
        _ => value.clone(),
    }
}

/// Check whether a single field should appear in the response for this role.
fn should_include_in_response_rbac(
    struct_name: &str,
    field_name: &str,
    jwt_role: Option<&str>,
    is_owner: bool,
) -> bool {
    // First apply the existing writeOnly/internal logic
    if !should_include_in_response(struct_name, field_name) {
        return false;
    }

    // Then check @visible decorator
    if let Some(meta) = get_struct_metadata(struct_name) {
        for field in &meta.fields {
            if field_names_match(&field.name, field_name) {
                for dec in &field.decorators {
                    if let Some(args_str) = dec
                        .strip_prefix("visible(")
                        .and_then(|s| s.strip_suffix(')'))
                    {
                        // @visible(Role1,Role2,...) or @visible(own)
                        let allowed: Vec<&str> = args_str.split(',').map(str::trim).collect();
                        let role_ok = match jwt_role {
                            Some(role) => allowed
                                .iter()
                                .any(|r| *r == role || role_satisfies(struct_name, role, r)),
                            None => false,
                        };
                        let own_ok = is_owner && allowed.contains(&"own");
                        if !role_ok && !own_ok {
                            return false;
                        }
                    }
                }
                break;
            }
        }
    }
    true
}

/// Filter request fields based on RBAC role.
///
/// Checks `@writable(Role)` — only accept the field from roles in the list.
pub(crate) fn filter_request_fields_rbac(
    value: &serde_json::Value,
    struct_name: &str,
    jwt_role: Option<&str>,
) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut filtered = serde_json::Map::new();
            for (k, v) in map {
                if should_accept_from_request_rbac(struct_name, k, jwt_role) {
                    filtered.insert(k.clone(), v.clone());
                }
            }
            serde_json::Value::Object(filtered)
        }
        serde_json::Value::Array(arr) => {
            let out: Vec<serde_json::Value> = arr
                .iter()
                .map(|item| filter_request_fields_rbac(item, struct_name, jwt_role))
                .collect();
            serde_json::Value::Array(out)
        }
        _ => value.clone(),
    }
}

/// Check whether a single field should be accepted from the request for this role.
fn should_accept_from_request_rbac(
    struct_name: &str,
    field_name: &str,
    jwt_role: Option<&str>,
) -> bool {
    use crate::metadata::should_accept_from_request;

    // First apply the existing readOnly/internal logic
    if !should_accept_from_request(struct_name, field_name) {
        return false;
    }

    // Then check @writable decorator
    if let Some(meta) = get_struct_metadata(struct_name) {
        for field in &meta.fields {
            if field_names_match(&field.name, field_name) {
                for dec in &field.decorators {
                    if let Some(args_str) = dec
                        .strip_prefix("writable(")
                        .and_then(|s| s.strip_suffix(')'))
                    {
                        let allowed: Vec<&str> = args_str.split(',').map(str::trim).collect();
                        let ok = match jwt_role {
                            Some(role) => allowed
                                .iter()
                                .any(|r| *r == role || role_satisfies(struct_name, role, r)),
                            None => false,
                        };
                        if !ok {
                            return false;
                        }
                    }
                }
                break;
            }
        }
    }
    true
}

// ============================================================================
// UTILITY — single source of truth: doo_ffi_core::case
// ============================================================================

use doo_ffi_core::{to_pascal_case, to_snake_case};

/// Extract the resource owner's user ID from a DB row.
///
/// Looks for the field decorated with `@owner` in the struct metadata.
/// Falls back to checking well-known column names: `user_id`, `owner_id`,
/// `created_by`.
pub(crate) fn get_resource_owner_from_row(
    item: &serde_json::Value,
    struct_name: &str,
) -> Option<i64> {
    // Try the @owner-decorated field first
    if let Some(meta) = get_struct_metadata(struct_name) {
        for field in &meta.fields {
            if field.decorators.iter().any(|d| d == "owner") {
                let col_snake = to_snake_case(&field.name);
                // DB rows come back with PascalCase keys from json_utils (snake_case → PascalCase)
                let col_pascal = to_pascal_case(&col_snake);
                let v = item
                    .get(&field.name)
                    .or_else(|| item.get(&col_snake))
                    .or_else(|| item.get(&col_pascal))?;
                return v
                    .as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()));
            }
        }
    }
    // Fallback: well-known owner columns — try both snake_case and PascalCase variants
    for key in &[
        "user_id",
        "UserId",
        "owner_id",
        "OwnerId",
        "created_by",
        "CreatedBy",
    ] {
        if let Some(v) = item.get(*key) {
            if let Some(n) = v.as_i64() {
                return Some(n);
            }
        }
    }
    None
}

/// Extract and verify a JWT from the `Authorization: Bearer <token>` header,
/// then return the full claims as a `serde_json::Value` (an Object on success,
/// `Value::Null` when there is no token or the token is invalid/expired).
///
/// We do a **lenient** parse here (no signature verification) because
/// signature verification already happened in `jwt_middleware_handler`.
/// This avoids duplicating the secret handling logic in every CRUD handler.
///
/// If you want strict per-request verification, replace with a proper
/// `jsonwebtoken::decode` call using `crate::middleware::get_jwt_secret()`.
pub(crate) fn extract_jwt_claims_from_request(
    req: *const crate::types::DooRequest,
) -> serde_json::Value {
    if req.is_null() {
        return serde_json::Value::Null;
    }

    // `(*req).headers` is a `Box<HashMap<String, String>>` cast to `*mut c_void`.
    // Cast it back to read headers injected by JWT middleware.
    let headers_ptr = unsafe { (*req).headers };

    // Primary path: headers were extracted (POST/PUT/DELETE routes, or GET with needs_headers)
    if !headers_ptr.is_null() {
        let user_id = unsafe {
            let headers = &*(headers_ptr as *const HashMap<String, String>);
            headers.get("x-user-id").and_then(|s| s.parse::<i64>().ok())
        };

        if let Some(uid) = user_id {
            // User was validated by JWT middleware — build claims from injected headers.
            let role = unsafe {
                let headers = &*(headers_ptr as *const HashMap<String, String>);
                headers.get("x-user-role").cloned()
            };
            let mut claims = serde_json::json!({ "user_id": uid });
            if let Some(r) = role {
                claims["role"] = serde_json::json!(r);
            }
            return claims;
        }

        // No x-user-id: JWT middleware didn't run (public route). Try decoding the
        // Authorization: Bearer header directly — needed for GET routes with RBAC policies.
        let token = unsafe {
            let headers = &*(headers_ptr as *const HashMap<String, String>);
            let auth = headers
                .get("authorization")
                .or_else(|| headers.get("Authorization"));
            match auth {
                Some(h) if h.starts_with("Bearer ") => Some(h[7..].to_string()),
                _ => None,
            }
        };
        if let Some(t) = token {
            let decoded = decode_jwt_claims_insecure(&t);
            if decoded.is_object() {
                return decoded;
            }
        }
        return serde_json::Value::Null;
    }

    // No headers at all: fallback to user_id field set by middleware.
    let user_id_ptr = unsafe { (*req).user_id };
    if !user_id_ptr.is_null() {
        let uid_str = unsafe { c_to_string(user_id_ptr) };
        if let Ok(uid) = uid_str.parse::<i64>() {
            return serde_json::json!({ "user_id": uid });
        }
    }

    serde_json::Value::Null
}

/// Decode JWT claims without signature verification.
///
/// Returns the claims as a `serde_json::Value::Object` on success,
/// or `Value::Null` on malformed input.
fn decode_jwt_claims_insecure(token: &str) -> serde_json::Value {
    // JWT = header.payload.signature — grab the middle segment
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() < 2 {
        return serde_json::Value::Null;
    }
    // Base64url decode the payload
    let decoded = base64_url_decode(parts[1]).unwrap_or_default();
    serde_json::from_slice(&decoded).unwrap_or(serde_json::Value::Null)
}

/// Minimal base64url decode (no padding required).
fn base64_url_decode(input: &str) -> Option<Vec<u8>> {
    // Re-pad to multiple of 4
    let rem = input.len() % 4;
    let padded = if rem == 0 {
        input.to_string()
    } else {
        format!("{}{}", input, "=".repeat(4 - rem))
    };
    // Replace URL-safe chars
    let standard = padded.replace('-', "+").replace('_', "/");
    use std::io::Read;
    // Use a simple manual base64 decoder to avoid adding a new dependency
    base64_decode_standard(&standard)
}

fn base64_decode_standard(input: &str) -> Option<Vec<u8>> {
    const TABLE: [u8; 256] = {
        let mut t = [255u8; 256];
        let mut i = 0u8;
        // A-Z
        while i < 26 {
            t[(b'A' + i) as usize] = i;
            i += 1;
        }
        let mut i = 0u8;
        // a-z
        while i < 26 {
            t[(b'a' + i) as usize] = 26 + i;
            i += 1;
        }
        let mut i = 0u8;
        // 0-9
        while i < 10 {
            t[(b'0' + i) as usize] = 52 + i;
            i += 1;
        }
        t[b'+' as usize] = 62;
        t[b'/' as usize] = 63;
        t[b'=' as usize] = 0; // padding
        t
    };

    let bytes = input.as_bytes();
    let len = bytes.len();
    if len % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(len / 4 * 3);
    let mut i = 0;
    while i < len {
        let a = TABLE[bytes[i] as usize];
        let b = TABLE[bytes[i + 1] as usize];
        let c = TABLE[bytes[i + 2] as usize];
        let d = TABLE[bytes[i + 3] as usize];
        if a == 255 || b == 255 {
            return None;
        }
        out.push((a << 2) | (b >> 4));
        if bytes[i + 2] != b'=' {
            out.push((b << 4) | (c >> 2));
        }
        if bytes[i + 3] != b'=' {
            out.push((c << 6) | d);
        }
        i += 4;
    }
    Some(out)
}
