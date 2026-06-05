//! CRUD System
//!
//! Database-backed CRUD handlers for resources.
//! Provides list, create, get, update, and delete handlers, plus
//! automatic table creation via `doo_http_crud`.
//! No in-memory fallback — database is required.

use std::collections::HashMap;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::Mutex as StdMutex;

use doo_ffi_core::ffi_debug;
use doo_ffi_core::DooResult;

use crate::db_bridge::{
    execute_db_delete_by_id, execute_db_insert, execute_db_query, execute_db_query_by_id,
    execute_db_statement, generate_create_table_sql, is_pool_initialized, to_snake_case,
};
use crate::helpers::c_to_string;
use crate::metadata::{filter_response_fields, get_struct_metadata, json_get_id};
use crate::rbac::{
    check_policy, extract_jwt_claims_from_request, filter_request_fields_rbac,
    filter_response_fields_rbac, get_jwt_role, get_resource_owner_from_row, is_authenticated,
};
use crate::router::{get_routes, CrudConfig};
use crate::types::*;
use crate::validation::{validate_item_against_schema, validate_required_fields};
use crate::{make_err_http, make_ok_json, make_ok_void};

// ============================================================================
// CRUD STATICS
// ============================================================================

/// Store which resources have been configured for DB-backed CRUD
static CRUD_DB_TABLES: std::sync::OnceLock<StdMutex<HashMap<String, String>>> =
    std::sync::OnceLock::new();

fn get_crud_db_tables() -> &'static StdMutex<HashMap<String, String>> {
    CRUD_DB_TABLES.get_or_init(|| StdMutex::new(HashMap::new()))
}

// ============================================================================
// CRUD HELPERS
// ============================================================================

/// Check if a resource is using database-backed CRUD
fn is_db_backed_crud(resource: &str) -> bool {
    if !is_pool_initialized() {
        return false;
    }
    let tables = get_crud_db_tables()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    tables.contains_key(resource)
}

/// Get the struct name for a CRUD resource
fn get_crud_struct_name(resource: &str) -> Option<String> {
    let tables = get_crud_db_tables()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    tables.get(resource).cloned()
}

/// Extract resource name from CRUD path.
/// Examples:
///   "/api/posts" -> "posts"
///   "/api/posts/1" -> "posts"
///   "/posts" -> "posts"
///   "/posts/1" -> "posts"
fn extract_crud_resource(path: &str) -> String {
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();

    for seg in segments.iter().rev() {
        let s = *seg;
        // Skip numeric IDs
        if s.parse::<i64>().is_ok() {
            continue;
        }
        // Skip common API prefixes and empty segments
        if s == "api" || s == "v1" || s == "v2" || s.is_empty() {
            continue;
        }
        return s.to_string();
    }

    String::new()
}

/// Fetch a single DB row by primary key. Returns None when not found or on error.
fn fetch_item_by_id(resource: &str, id: i64) -> Option<serde_json::Value> {
    let sql = format!("SELECT * FROM {} WHERE id = $1", resource);
    match execute_db_query_by_id(&sql, id as i32) {
        Ok(json) => {
            let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap_or_default();
            items.into_iter().next()
        }
        Err(_) => None,
    }
}

/// Check if a struct has at least one field decorated with @primary.
fn struct_has_primary(struct_name: &str) -> bool {
    crate::metadata::get_struct_metadata(struct_name)
        .map(|meta| {
            meta.fields
                .iter()
                .any(|f| f.decorators.iter().any(|d| d == "primary"))
        })
        .unwrap_or(false)
}

/// Create CRUD handler that returns all items
#[allow(unused_variables)]
fn make_crud_list_handler(resource: String) -> DooHandlerFn {
    crud_list_handler
}

// ============================================================================
// CRUD HANDLERS
// ============================================================================

extern "C" fn crud_list_handler(req: *const DooRequest) -> *mut DooResult {
    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let path = unsafe { c_to_string((*req).path) };
    let resource = extract_crud_resource(&path);

    // --- RBAC: check read policy ---
    let jwt_claims = extract_jwt_claims_from_request(req);
    let struct_name_for_rbac = get_crud_struct_name(&resource).unwrap_or_else(|| resource.clone());
    if !check_policy(&jwt_claims, &struct_name_for_rbac, "read", None) {
        if !is_authenticated(&jwt_claims) {
            return make_err_http(401, "Unauthorized");
        }
        return make_err_http(403, "Access denied");
    }
    let jwt_role = get_jwt_role(&jwt_claims, &struct_name_for_rbac);

    // Try database-backed CRUD first
    if is_db_backed_crud(&resource) {
        let sql = format!("SELECT * FROM {}", resource);
        match execute_db_query(&sql) {
            Ok(json) => {
                let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap_or_default();
                let struct_name = get_crud_struct_name(&resource).unwrap_or_default();
                // Apply RBAC field filtering (extends base writeOnly/internal filtering)
                let filtered = filter_response_fields_rbac(
                    &serde_json::json!(items),
                    &struct_name,
                    jwt_role.as_deref(),
                    false, // ownership is per-item; use false for list
                );
                let response = serde_json::to_string(&serde_json::json!({ "data": filtered }))
                    .unwrap_or_else(|_| r#"{"data":[]}"#.to_string());
                return make_ok_json(&response);
            }
            Err(e) => {
                ffi_debug!("CRUD", "DB list error: {}", e);
                return make_err_http(500, &format!("Query failed: {}", e));
            }
        }
    }

    // No in-memory fallback — database is required
    ffi_debug!(
        "CRUD",
        "ERROR: Database not available for resource: {}",
        resource
    );
    make_err_http(503, "Database not available")
}

extern "C" fn crud_create_handler(req: *const DooRequest) -> *mut DooResult {
    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let path = unsafe { c_to_string((*req).path) };
    let body = unsafe { c_to_string((*req).body) };

    ffi_debug!(
        "CRUD",
        "POST {} - body length: {}, body: {:?}",
        path,
        body.len(),
        &body[..body.len().min(200)]
    );

    let resource = extract_crud_resource(&path);

    // --- RBAC: check create policy ---
    let jwt_claims = extract_jwt_claims_from_request(req);
    let struct_name_for_rbac = get_crud_struct_name(&resource).unwrap_or_else(|| resource.clone());
    if !check_policy(&jwt_claims, &struct_name_for_rbac, "create", None) {
        if !is_authenticated(&jwt_claims) {
            return make_err_http(401, "Unauthorized");
        }
        return make_err_http(403, "Access denied");
    }
    let jwt_role = get_jwt_role(&jwt_claims, &struct_name_for_rbac);

    // Parse body JSON
    let mut item: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            ffi_debug!("CRUD", "JSON parse error: {:?}", e);
            return make_err_http(400, "Invalid JSON body");
        }
    };

    // Validate fields using centralized struct/enum metadata
    if let Err(validation_error) = validate_item_against_schema(&item, &resource, &path) {
        return make_err_http(422, &validation_error);
    }

    // Validate required fields — returns 400 if any non-auto/non-owner field is absent
    if let Err(missing) = validate_required_fields(&item, &resource, &path) {
        return make_err_http(400, &missing);
    }

    // Auto-fill @owner field with the JWT user_id so users cannot forge ownership.
    // Only applied when the user is authenticated and the struct has an @owner field.
    if let Some(user_id) = crate::rbac::get_jwt_user_id(&jwt_claims) {
        if let Some(meta) = get_struct_metadata(&struct_name_for_rbac) {
            for field in &meta.fields {
                if field.decorators.iter().any(|d| d == "owner") {
                    let col = to_snake_case(&field.name);
                    if let Some(obj) = item.as_object_mut() {
                        obj.insert(col, serde_json::json!(user_id));
                    }
                    break;
                }
            }
        }
    }

    // Try database-backed CRUD first
    if is_db_backed_crud(&resource) {
        if let Some(obj) = item.as_object() {
            let struct_name = get_crud_struct_name(&resource);
            if let Some(meta) = struct_name.and_then(|n| get_struct_metadata(&n)) {
                let mut columns = Vec::new();
                let mut values = Vec::new();
                let mut placeholders = Vec::new();
                let mut idx = 1;

                for field in &meta.fields {
                    if field.name.to_lowercase() == "id" {
                        continue;
                    }
                    let col_name = to_snake_case(&field.name);
                    if let Some(val) = obj.get(&field.name).or_else(|| obj.get(&col_name)) {
                        columns.push(col_name);
                        placeholders.push(format!("${}", idx));
                        values.push(val.clone());
                        idx += 1;
                    }
                }

                if !columns.is_empty() {
                    let sql = format!(
                        "INSERT INTO {} ({}) VALUES ({}) RETURNING *",
                        resource,
                        columns.join(", "),
                        placeholders.join(", ")
                    );
                    ffi_debug!("CRUD", "DB INSERT SQL: {}", sql);
                    ffi_debug!("CRUD", "DB INSERT values: {:?}", values);

                    match execute_db_insert(&sql, &values) {
                        Ok(json) => {
                            let items: Vec<serde_json::Value> =
                                serde_json::from_str(&json).unwrap_or_default();
                            let created = items.into_iter().next().unwrap_or(serde_json::json!({}));
                            let sn = get_crud_struct_name(&resource).unwrap_or_default();
                            let filtered = filter_response_fields_rbac(
                                &created,
                                &sn,
                                jwt_role.as_deref(),
                                false,
                            );
                            let response =
                                serde_json::to_string(&serde_json::json!({ "data": filtered }))
                                    .unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
                            return make_ok_json(&response);
                        }
                        Err(e) => {
                            ffi_debug!("CRUD", "DB insert error: {}", e);
                            return make_err_http(500, &format!("Insert failed: {}", e));
                        }
                    }
                }
            }
        }
    }

    // No in-memory fallback — database is required
    ffi_debug!(
        "CRUD",
        "ERROR: Database not available for resource: {}",
        resource
    );
    make_err_http(503, "Database not available")
}

extern "C" fn crud_get_handler(req: *const DooRequest) -> *mut DooResult {
    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let path = unsafe { c_to_string((*req).path) };
    let params_json = unsafe { (*req).params as *const c_char };
    let params: serde_json::Value = if !params_json.is_null() {
        let s = unsafe { std::ffi::CStr::from_ptr(params_json).to_string_lossy() };
        serde_json::from_str(&s).unwrap_or_default()
    } else {
        serde_json::json!({})
    };

    let resource = extract_crud_resource(&path);

    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let id: i64 = params
        .get("id")
        .and_then(|v: &serde_json::Value| v.as_str())
        .and_then(|s: &str| s.parse().ok())
        .unwrap_or_else(|| parts.iter().rev().find_map(|s| s.parse().ok()).unwrap_or(0));

    // --- RBAC: check read policy (ownership check happens after DB fetch) ---
    let jwt_claims = extract_jwt_claims_from_request(req);
    let struct_name_for_rbac = get_crud_struct_name(&resource).unwrap_or_else(|| resource.clone());
    // Pre-check: if the rule is purely "Admin" etc. we can reject early.
    // For "own" rules we need the resource; check_policy with id=None will allow it
    // and we do a second check post-fetch.
    if !check_policy(&jwt_claims, &struct_name_for_rbac, "read", Some(id)) {
        if !is_authenticated(&jwt_claims) {
            return make_err_http(401, "Unauthorized");
        }
        return make_err_http(403, "Access denied");
    }

    // Check if struct has a @primary field — individual resource ops require it
    if !struct_has_primary(&struct_name_for_rbac) {
        return make_err_http(400, "Resource has no primary key — cannot fetch by ID");
    }
    let jwt_role = get_jwt_role(&jwt_claims, &struct_name_for_rbac);

    // Try database-backed CRUD first
    if is_db_backed_crud(&resource) {
        let sql = format!("SELECT * FROM {} WHERE id = $1", resource);
        match execute_db_query_by_id(&sql, id as i32) {
            Ok(json) => {
                let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap_or_default();
                if let Some(item) = items.into_iter().next() {
                    let struct_name = get_crud_struct_name(&resource).unwrap_or_default();
                    // Determine ownership for field-level visibility
                    let owner_id = get_resource_owner_from_row(&item, &struct_name);
                    let is_owner = is_authenticated(&jwt_claims)
                        && owner_id == crate::rbac::get_jwt_user_id(&jwt_claims);
                    let filtered = filter_response_fields_rbac(
                        &item,
                        &struct_name,
                        jwt_role.as_deref(),
                        is_owner,
                    );
                    let response = serde_json::to_string(&serde_json::json!({ "data": filtered }))
                        .unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
                    return make_ok_json(&response);
                } else {
                    return make_err_http(404, "Resource not found");
                }
            }
            Err(e) => {
                ffi_debug!("CRUD", "DB get error: {}", e);
                return make_err_http(500, &format!("Query failed: {}", e));
            }
        }
    }

    // No in-memory fallback — database is required
    ffi_debug!(
        "CRUD",
        "ERROR: Database not available for resource: {}",
        resource
    );
    make_err_http(503, "Database not available")
}

extern "C" fn crud_update_handler(req: *const DooRequest) -> *mut DooResult {
    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let path = unsafe { c_to_string((*req).path) };
    let body = unsafe { c_to_string((*req).body) };
    let params_json = unsafe { (*req).params as *const c_char };
    let params: serde_json::Value = if !params_json.is_null() {
        let s = unsafe { std::ffi::CStr::from_ptr(params_json).to_string_lossy() };
        serde_json::from_str(&s).unwrap_or_default()
    } else {
        serde_json::json!({})
    };

    let resource = extract_crud_resource(&path);

    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let id: i64 = params
        .get("id")
        .and_then(|v: &serde_json::Value| v.as_str())
        .and_then(|s: &str| s.parse().ok())
        .unwrap_or_else(|| parts.iter().rev().find_map(|s| s.parse().ok()).unwrap_or(0));

    let updates: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return make_err_http(400, "Invalid JSON body"),
    };

    // Extract JWT claims — header reading is safe here because write routes
    // always have JWT middleware (needs_headers=true).
    let jwt_claims = extract_jwt_claims_from_request(req);
    let struct_name_for_rbac = get_crud_struct_name(&resource).unwrap_or_else(|| resource.clone());
    let jwt_role = get_jwt_role(&jwt_claims, &struct_name_for_rbac);

    // --- RBAC: check update policy with real owner ID ---
    // Fetch the item first so we can compare the @owner field against the JWT user_id.
    if !struct_has_primary(&struct_name_for_rbac) {
        return make_err_http(400, "Resource has no primary key — cannot update by ID");
    }
    if is_db_backed_crud(&resource) {
        let existing = match fetch_item_by_id(&resource, id) {
            Some(item) => item,
            None => return make_err_http(404, "Resource not found"),
        };
        let resource_owner_id = get_resource_owner_from_row(&existing, &struct_name_for_rbac);
        if !check_policy(
            &jwt_claims,
            &struct_name_for_rbac,
            "update",
            resource_owner_id,
        ) {
            if !is_authenticated(&jwt_claims) {
                return make_err_http(401, "Unauthorized");
            }
            return make_err_http(403, "Access denied");
        }
    } else {
        return make_err_http(503, "Database not available");
    }
    // Filter request body by role-writable fields
    let updates = filter_request_fields_rbac(&updates, &struct_name_for_rbac, jwt_role.as_deref());

    // Apply the update (we already confirmed is_db_backed_crud above)
    if let Some(obj) = updates.as_object() {
        let struct_name = get_crud_struct_name(&resource);
        if let Some(meta) = struct_name.and_then(|n| get_struct_metadata(&n)) {
            let mut set_clauses = Vec::new();
            let mut values: Vec<serde_json::Value> = Vec::new();
            let mut idx = 1;

            for field in &meta.fields {
                if field.name.to_lowercase() == "id" {
                    continue;
                }
                let col_name = to_snake_case(&field.name);
                if let Some(val) = obj.get(&field.name).or_else(|| obj.get(&col_name)) {
                    set_clauses.push(format!("{} = ${}", col_name, idx));
                    values.push(val.clone());
                    idx += 1;
                }
            }

            if !set_clauses.is_empty() {
                values.push(serde_json::json!(id));
                let sql = format!(
                    "UPDATE {} SET {} WHERE id = ${} RETURNING *",
                    resource,
                    set_clauses.join(", "),
                    idx
                );
                ffi_debug!("CRUD", "DB UPDATE SQL: {}", sql);

                match execute_db_insert(&sql, &values) {
                    Ok(json) => {
                        let items: Vec<serde_json::Value> =
                            serde_json::from_str(&json).unwrap_or_default();
                        if let Some(updated) = items.into_iter().next() {
                            let sn = get_crud_struct_name(&resource).unwrap_or_default();
                            let owner_id = get_resource_owner_from_row(&updated, &sn);
                            let is_owner = is_authenticated(&jwt_claims)
                                && owner_id == crate::rbac::get_jwt_user_id(&jwt_claims);
                            let filtered = filter_response_fields_rbac(
                                &updated,
                                &sn,
                                jwt_role.as_deref(),
                                is_owner,
                            );
                            let response =
                                serde_json::to_string(&serde_json::json!({ "data": filtered }))
                                    .unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
                            return make_ok_json(&response);
                        } else {
                            return make_err_http(404, "Resource not found");
                        }
                    }
                    Err(e) => {
                        ffi_debug!("CRUD", "DB update error: {}", e);
                        return make_err_http(500, &format!("Update failed: {}", e));
                    }
                }
            }
        }
    }

    make_err_http(400, "No fields to update")
}

extern "C" fn crud_delete_handler(req: *const DooRequest) -> *mut DooResult {
    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let path = unsafe { c_to_string((*req).path) };
    let params_json = unsafe { (*req).params as *const c_char };
    let params: serde_json::Value = if !params_json.is_null() {
        let s = unsafe { std::ffi::CStr::from_ptr(params_json).to_string_lossy() };
        serde_json::from_str(&s).unwrap_or_default()
    } else {
        serde_json::json!({})
    };

    let resource = extract_crud_resource(&path);

    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let id: i64 = params
        .get("id")
        .and_then(|v: &serde_json::Value| v.as_str())
        .and_then(|s: &str| s.parse().ok())
        .unwrap_or_else(|| parts.iter().rev().find_map(|s| s.parse().ok()).unwrap_or(0));

    // --- RBAC: check delete policy with real owner ID ---
    let jwt_claims = extract_jwt_claims_from_request(req);
    let struct_name_for_rbac = get_crud_struct_name(&resource).unwrap_or_else(|| resource.clone());

    if !struct_has_primary(&struct_name_for_rbac) {
        return make_err_http(400, "Resource has no primary key — cannot delete by ID");
    }

    if is_db_backed_crud(&resource) {
        let existing = match fetch_item_by_id(&resource, id) {
            Some(item) => item,
            None => return make_err_http(404, "Resource not found"),
        };
        let resource_owner_id = get_resource_owner_from_row(&existing, &struct_name_for_rbac);
        if !check_policy(
            &jwt_claims,
            &struct_name_for_rbac,
            "delete",
            resource_owner_id,
        ) {
            if !is_authenticated(&jwt_claims) {
                return make_err_http(401, "Unauthorized");
            }
            return make_err_http(403, "Access denied");
        }

        let sql = format!("DELETE FROM {} WHERE id = $1", resource);
        match execute_db_delete_by_id(&sql, id as i32) {
            Ok(affected) => {
                if affected > 0 {
                    return make_ok_json(r#"{"data":{"deleted":true}}"#);
                } else {
                    return make_err_http(404, "Resource not found");
                }
            }
            Err(e) => {
                ffi_debug!("CRUD", "DB delete error: {}", e);
                return make_err_http(500, &format!("Delete failed: {}", e));
            }
        }
    }

    // No in-memory fallback — database is required
    ffi_debug!(
        "CRUD",
        "ERROR: Database not available for resource: {}",
        resource
    );
    make_err_http(503, "Database not available")
}

// ============================================================================
// CRUD ROUTE REGISTRATION
// ============================================================================

/// Set up CRUD routes for a resource struct.
/// Creates GET, POST, PUT, DELETE endpoints for the resource.
/// If database is connected, creates the table and uses DB-backed CRUD.
#[no_mangle]
pub extern "C" fn doo_http_crud(
    _server: *const c_void,
    base_path: *const c_char,
    resource_struct_name: *const c_char,
    _db: *const c_void,
) -> *mut DooResult {
    ffi_safe_result!({
        let base_str = c_to_string(base_path);
        let struct_name = c_to_string(resource_struct_name);

        ffi_debug!(
            "HTTP",
            "CRUD configured: base={}, struct={}",
            base_str,
            struct_name
        );

        let resource_key = extract_crud_resource(&base_str);
        ffi_debug!("HTTP", "CRUD resource key: {}", resource_key);

        // Try to create table in database if connected
        if is_pool_initialized() {
            ffi_debug!(
                "HTTP",
                "Database connected, setting up DB-backed CRUD for {}",
                resource_key
            );

            if let Some(metadata) = get_struct_metadata(&struct_name) {
                let create_sql = generate_create_table_sql(&resource_key, &metadata);
                ffi_debug!("HTTP", "CREATE TABLE SQL:\n{}", create_sql);

                match execute_db_statement(&create_sql) {
                    Ok(_) => {
                        ffi_debug!(
                            "HTTP",
                            "Table '{}' created/verified successfully",
                            resource_key
                        );
                        let mut tables = get_crud_db_tables()
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        tables.insert(resource_key.clone(), struct_name.clone());
                    }
                    Err(e) => {
                        ffi_debug!(
                            "HTTP",
                            "Warning: Failed to create table '{}': {}",
                            resource_key,
                            e
                        );
                    }
                }
            } else {
                ffi_debug!(
                    "HTTP",
                    "Warning: No metadata found for struct '{}'",
                    struct_name
                );
            }
        } else {
            ffi_debug!(
                "HTTP",
                "WARNING: No database connection for CRUD resource '{}'",
                resource_key
            );
        }

        // Register CRUD routes
        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());

        // Read routes extract headers so RBAC can enforce read policies with auth context.
        registry.register_needs_headers("GET", &base_str, crud_list_handler);
        let get_one_path = format!("{}/{{id}}", base_str);
        registry.register_needs_headers("GET", &get_one_path, crud_get_handler);

        // Write routes: auto-protect with JWT when app.auth() has been configured.
        // This is generic — any CRUD endpoint auto-protects writes when auth is present.
        let auth_configured = registry.auth_config.is_some();
        if auth_configured {
            ffi_debug!(
                "HTTP",
                "Auth configured — CRUD write routes for {} will require JWT",
                base_str
            );
            let jwt_mw: Vec<DooMiddlewareFn> = vec![crate::middleware::jwt_middleware_handler];
            registry.register_with_middleware(
                "POST",
                &base_str,
                crud_create_handler,
                jwt_mw.clone(),
            );
            registry.register_with_middleware(
                "PUT",
                &get_one_path,
                crud_update_handler,
                jwt_mw.clone(),
            );
            registry.register_with_middleware("DELETE", &get_one_path, crud_delete_handler, jwt_mw);
        } else {
            registry.register("POST", &base_str, crud_create_handler);
            registry.register("PUT", &get_one_path, crud_update_handler);
            registry.register("DELETE", &get_one_path, crud_delete_handler);
        }

        registry.crud_configs.push(CrudConfig {
            base_path: base_str,
            resource_struct: struct_name,
        });

        make_ok_void()
    })
}
