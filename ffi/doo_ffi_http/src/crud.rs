//! CRUD System
//!
//! In-memory and database-backed CRUD handlers for resources.
//! Provides list, create, get, update, and delete handlers, plus
//! automatic table creation via `doo_http_crud`.

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
use crate::metadata::{filter_response_fields, get_struct_metadata};
use crate::router::{get_routes, CrudConfig};
use crate::types::*;
use crate::validation::validate_item_against_schema;
use crate::{make_err_http, make_ok_json, make_ok_void};

// ============================================================================
// CRUD STATICS
// ============================================================================

/// In-memory store fallback for CRUD resources (used when DB not connected)
static CRUD_STORES: std::sync::OnceLock<StdMutex<HashMap<String, CrudStore>>> =
    std::sync::OnceLock::new();

/// Store which resources have been configured for DB-backed CRUD
static CRUD_DB_TABLES: std::sync::OnceLock<StdMutex<HashMap<String, String>>> =
    std::sync::OnceLock::new();

fn get_crud_stores() -> &'static StdMutex<HashMap<String, CrudStore>> {
    CRUD_STORES.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn get_crud_db_tables() -> &'static StdMutex<HashMap<String, String>> {
    CRUD_DB_TABLES.get_or_init(|| StdMutex::new(HashMap::new()))
}

struct CrudStore {
    items: Vec<serde_json::Value>,
    next_id: i64,
}

impl CrudStore {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            next_id: 1,
        }
    }
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

/// Insert an item into the CRUD in-memory store for a given resource.
/// Used by auth to sync signed-up users into the CRUD store so that
/// GET /users returns users created via signup when no DB is available.
pub(crate) fn crud_store_insert(resource: &str, item: serde_json::Value) {
    let mut stores = get_crud_stores().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(store) = stores.get_mut(resource) {
        store.items.push(item);
    }
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

    // Try database-backed CRUD first
    if is_db_backed_crud(&resource) {
        let sql = format!("SELECT * FROM {}", resource);
        match execute_db_query(&sql) {
            Ok(json) => {
                let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap_or_default();
                let struct_name = get_crud_struct_name(&resource).unwrap_or_default();
                let filtered = filter_response_fields(&serde_json::json!(items), &struct_name);
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

    // Fallback to in-memory store
    let stores = get_crud_stores().lock().unwrap_or_else(|e| e.into_inner());
    let items = match stores.get(&resource) {
        Some(store) => store.items.clone(),
        None => Vec::new(),
    };

    let struct_name = get_crud_struct_name(&resource).unwrap_or_default();
    let filtered = filter_response_fields(&serde_json::json!(items), &struct_name);
    let response = serde_json::to_string(&serde_json::json!({ "data": filtered }))
        .unwrap_or_else(|_| r#"{"data":[]}"#.to_string());
    make_ok_json(&response)
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

    // Parse body JSON
    let item: serde_json::Value = match serde_json::from_str(&body) {
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
                            let filtered = filter_response_fields(&created, &sn);
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

    // Fallback to in-memory store
    let mut item = item;
    let mut stores = get_crud_stores().lock().unwrap_or_else(|e| e.into_inner());
    let store = stores
        .entry(resource.clone())
        .or_insert_with(CrudStore::new);

    if let Some(obj) = item.as_object_mut() {
        obj.insert("id".to_string(), serde_json::json!(store.next_id));
    }
    store.next_id += 1;
    store.items.push(item.clone());

    let struct_name = get_crud_struct_name(&resource).unwrap_or_default();
    let filtered = filter_response_fields(&item, &struct_name);
    let response = serde_json::to_string(&serde_json::json!({ "data": filtered }))
        .unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
    make_ok_json(&response)
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
        .unwrap_or_else(|| {
            parts.iter().rev().find_map(|s| s.parse().ok()).unwrap_or(0)
        });

    // Try database-backed CRUD first
    if is_db_backed_crud(&resource) {
        let sql = format!("SELECT * FROM {} WHERE id = $1", resource);
        match execute_db_query_by_id(&sql, id as i32) {
            Ok(json) => {
                let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap_or_default();
                if let Some(item) = items.into_iter().next() {
                    let struct_name = get_crud_struct_name(&resource).unwrap_or_default();
                    let filtered = filter_response_fields(&item, &struct_name);
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

    // Fallback to in-memory store
    let stores = get_crud_stores().lock().unwrap_or_else(|e| e.into_inner());
    let item = stores
        .get(&resource)
        .and_then(|store| {
            store
                .items
                .iter()
                .find(|i| i.get("id").and_then(|v| v.as_i64()) == Some(id))
        })
        .cloned();

    match item {
        Some(i) => {
            let struct_name = get_crud_struct_name(&resource).unwrap_or_default();
            let filtered = filter_response_fields(&i, &struct_name);
            let response = serde_json::to_string(&serde_json::json!({ "data": filtered }))
                .unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
            make_ok_json(&response)
        }
        None => make_err_http(404, "Resource not found"),
    }
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
        .unwrap_or_else(|| {
            parts.iter().rev().find_map(|s| s.parse().ok()).unwrap_or(0)
        });

    let updates: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return make_err_http(400, "Invalid JSON body"),
    };

    // Try database-backed CRUD first
    if is_db_backed_crud(&resource) {
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
                                let response =
                                    serde_json::to_string(&serde_json::json!({ "data": updated }))
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
    }

    // Fallback to in-memory store
    let mut stores = get_crud_stores().lock().unwrap_or_else(|e| e.into_inner());
    let item = stores.get_mut(&resource).and_then(|store| {
        store
            .items
            .iter_mut()
            .find(|i| i.get("id").and_then(|v| v.as_i64()) == Some(id))
    });

    match item {
        Some(i) => {
            if let (Some(existing), Some(new)) = (i.as_object_mut(), updates.as_object()) {
                for (k, v) in new {
                    existing.insert(k.clone(), v.clone());
                }
            }
            let response = serde_json::to_string(&serde_json::json!({ "data": i }))
                .unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
            make_ok_json(&response)
        }
        None => make_err_http(404, "Resource not found"),
    }
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
        .unwrap_or_else(|| {
            parts.iter().rev().find_map(|s| s.parse().ok()).unwrap_or(0)
        });

    // Try database-backed CRUD first
    if is_db_backed_crud(&resource) {
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

    // Fallback to in-memory store
    let mut stores = get_crud_stores().lock().unwrap_or_else(|e| e.into_inner());
    let removed = stores
        .get_mut(&resource)
        .map(|store| {
            let before = store.items.len();
            store
                .items
                .retain(|i| i.get("id").and_then(|v| v.as_i64()) != Some(id));
            before != store.items.len()
        })
        .unwrap_or(false);

    if removed {
        make_ok_json(r#"{"data":{"deleted":true}}"#)
    } else {
        make_err_http(404, "Resource not found")
    }
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
                    "Warning: No metadata found for struct '{}', using in-memory store",
                    struct_name
                );
            }
        } else {
            ffi_debug!("HTTP", "No database connection, using in-memory CRUD store");
        }

        // Initialize in-memory store as fallback
        {
            let mut stores = get_crud_stores().lock().unwrap_or_else(|e| e.into_inner());
            stores
                .entry(resource_key.clone())
                .or_insert_with(CrudStore::new);
        }

        // Register CRUD routes
        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());

        registry.register("GET", &base_str, crud_list_handler);
        registry.register("POST", &base_str, crud_create_handler);

        let get_one_path = format!("{}/{{id}}", base_str);
        registry.register("GET", &get_one_path, crud_get_handler);
        registry.register("PUT", &get_one_path, crud_update_handler);
        registry.register("DELETE", &get_one_path, crud_delete_handler);

        registry.crud_configs.push(CrudConfig {
            base_path: base_str,
            resource_struct: struct_name,
        });

        make_ok_void()
    })
}
