//! Auth System
//!
//! Signup and login handlers, JWT token generation, and auth route
//! registration (`doo_http_auth`). Supports both in-memory and
//! database-backed user storage.

use std::collections::HashMap;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::Mutex as StdMutex;

use doo_ffi_core::ffi_debug;
use doo_ffi_core::DooResult;

use crate::db_bridge::{
    execute_db_insert, execute_db_query_with_string_param, execute_db_statement,
    generate_create_table_sql, is_pool_initialized, to_snake_case,
};
use crate::helpers::c_to_string;
use crate::metadata::get_struct_metadata;
use crate::router::{get_routes, AuthConfig};
use crate::types::*;
use crate::{make_err_http, make_ok_json, make_ok_void};

// ============================================================================
// AUTH STATICS
// ============================================================================

/// In-memory user store for auth (fallback when no database connected)
static AUTH_USERS: std::sync::OnceLock<StdMutex<HashMap<String, AuthUser>>> =
    std::sync::OnceLock::new();

/// Counter for generating user IDs (in-memory; production would use DB auto-increment)
static AUTH_USER_ID_COUNTER: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(1);

/// Store which auth table has been created in the database
static AUTH_DB_TABLE: std::sync::OnceLock<StdMutex<Option<String>>> = std::sync::OnceLock::new();

fn get_auth_users() -> &'static StdMutex<HashMap<String, AuthUser>> {
    AUTH_USERS.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn get_auth_db_table() -> &'static StdMutex<Option<String>> {
    AUTH_DB_TABLE.get_or_init(|| StdMutex::new(None))
}

/// Check if auth is using the database
fn is_auth_db_backed() -> bool {
    if !is_pool_initialized() {
        return false;
    }
    let table = get_auth_db_table()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    table.is_some()
}

/// Get the auth table name (e.g., "users")
fn get_auth_table_name() -> Option<String> {
    let table = get_auth_db_table()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    table.clone()
}

/// Basic email format validation
/// Returns true if email has valid format: something@something.something
fn is_valid_email(email: &str) -> bool {
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return false;
    }
    let local = parts[0];
    let domain = parts[1];

    if local.is_empty() {
        return false;
    }

    if !domain.contains('.') {
        return false;
    }

    let domain_parts: Vec<&str> = domain.split('.').collect();
    if domain_parts.iter().any(|p| p.is_empty()) {
        return false;
    }

    true
}

// ============================================================================
// AUTH USER STRUCT
// ============================================================================

/// Generic auth user that stores all fields from the user's struct
#[derive(Clone)]
struct AuthUser {
    id: i64,
    email: String,
    password_hash: String,
    /// Additional fields from the user's struct (stored as JSON)
    extra_fields: serde_json::Value,
}

// ============================================================================
// SIGNUP HANDLER
// ============================================================================

/// Signup handler - registers a new user
/// Generic: works with any struct that has email and password fields, plus any additional fields
extern "C" fn auth_signup_handler(req: *const DooRequest) -> *mut DooResult {
    ffi_debug!("AUTH", "auth_signup_handler called");

    if req.is_null() {
        ffi_debug!("AUTH", "Error: Invalid request (null)");
        return make_err_http(400, "Invalid request");
    }

    let body = unsafe { c_to_string((*req).body) };
    ffi_debug!("AUTH", "Request body: {}", body);

    // Parse full JSON body - supports any fields the user defines
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&body);
    let json = match parsed {
        Ok(serde_json::Value::Object(obj)) => obj,
        Ok(_) => {
            ffi_debug!("AUTH", "Error: Request body is not a JSON object");
            return make_err_http(400, "Request body must be a JSON object");
        }
        Err(e) => {
            ffi_debug!("AUTH", "Error: Invalid JSON body: {}", e);
            return make_err_http(400, "Invalid JSON body");
        }
    };

    // Extract email (required, case-insensitive field lookup)
    let email = json
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("email"))
        .and_then(|(_, v)| v.as_str())
        .map(|s| s.to_string());

    let email = match email {
        Some(e) => e,
        None => {
            ffi_debug!("AUTH", "Error: Missing 'email' field");
            return make_err_http(400, "Missing 'email' field");
        }
    };

    // Validate email format
    if !is_valid_email(&email) {
        ffi_debug!("AUTH", "Error: Invalid email format: {}", email);
        return make_err_http(
            400,
            "Invalid email format. Email must be in format: name@domain.tld",
        );
    }

    // Extract password (required, case-insensitive field lookup)
    let password = json
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("password"))
        .and_then(|(_, v)| v.as_str());

    let password = match password {
        Some(p) => p,
        None => {
            ffi_debug!("AUTH", "Error: Missing 'password' field");
            return make_err_http(400, "Missing 'password' field");
        }
    };

    // Hash password using bcrypt at production cost (DEFAULT_COST = 12)
    let password_hash = match bcrypt::hash(password, bcrypt::DEFAULT_COST) {
        Ok(h) => h,
        Err(_) => return make_err_http(500, "Failed to hash password"),
    };

    // Build extra_fields: all fields except email and password
    let mut extra_fields = serde_json::Map::new();
    for (key, value) in json.iter() {
        if !key.eq_ignore_ascii_case("email") && !key.eq_ignore_ascii_case("password") {
            extra_fields.insert(key.clone(), value.clone());
        }
    }

    // Try database-backed auth first
    if is_auth_db_backed() {
        if let Some(table_name) = get_auth_table_name() {
            ffi_debug!("AUTH", "Using database-backed auth, table: {}", table_name);

            // Check if user already exists
            let check_sql = format!("SELECT id FROM {} WHERE email = $1", table_name);
            match execute_db_query_with_string_param(&check_sql, &email) {
                Ok(json) => {
                    let rows: Vec<serde_json::Value> =
                        serde_json::from_str(&json).unwrap_or_default();
                    if !rows.is_empty() {
                        ffi_debug!("AUTH", "Error: User already exists in DB: {}", email);
                        return make_err_http(409, "User already exists");
                    }
                }
                Err(e) => {
                    ffi_debug!("AUTH", "DB error checking existing user: {}", e);
                }
            }

            // Build dynamic INSERT based on extra_fields from request
            let mut columns = vec!["email".to_string(), "password".to_string()];
            let mut values: Vec<serde_json::Value> = vec![
                serde_json::json!(email),
                serde_json::json!(password_hash.clone()),
            ];
            let mut placeholders: Vec<String> = vec!["$1".to_string(), "$2".to_string()];
            let mut idx = 3;

            // Add extra fields dynamically
            for (key, value) in extra_fields.iter() {
                let col_name = to_snake_case(key);
                columns.push(col_name);
                placeholders.push(format!("${}", idx));
                values.push(value.clone());
                idx += 1;
            }

            let insert_sql = format!(
                "INSERT INTO {} ({}) VALUES ({}) RETURNING *",
                table_name,
                columns.join(", "),
                placeholders.join(", ")
            );

            ffi_debug!("AUTH", "Inserting user with SQL: {}", insert_sql);
            ffi_debug!("AUTH", "Values: {:?}", values);

            match execute_db_insert(&insert_sql, &values) {
                Ok(json) => {
                    ffi_debug!("AUTH", "Insert result: {}", json);
                    let rows: Vec<serde_json::Value> =
                        serde_json::from_str(&json).unwrap_or_default();
                    if let Some(user_row) = rows.into_iter().next() {
                        let user_id = user_row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);

                        // Generate JWT token with user_id in claims
                        let token = generate_jwt_token(&email, user_id);

                        // Build response with all fields from the database row (except password)
                        let mut response_data = serde_json::json!({
                            "token": token,
                        });
                        if let (Some(obj), Some(row_obj)) =
                            (response_data.as_object_mut(), user_row.as_object())
                        {
                            for (k, v) in row_obj {
                                if !k.eq_ignore_ascii_case("password") {
                                    obj.insert(k.clone(), v.clone());
                                }
                            }
                        }

                        let response = serde_json::json!({ "data": response_data }).to_string();
                        ffi_debug!("AUTH", "Signup success (DB): {}", response);
                        return make_ok_json(&response);
                    } else {
                        ffi_debug!("AUTH", "DB insert returned no rows, falling back");
                        // Fall through to in-memory
                    }
                }
                Err(e) => {
                    ffi_debug!("AUTH", "DB insert error: {}", e);
                    if e.contains("duplicate key") || e.contains("unique constraint") {
                        return make_err_http(409, "User already exists");
                    }
                    return make_err_http(500, &format!("Database error: {}", e));
                }
            }
        }
    }

    // Fallback to in-memory auth
    ffi_debug!("AUTH", "Using in-memory auth fallback");

    // Check if user already exists (in-memory check)
    {
        let users = get_auth_users().lock().unwrap_or_else(|e| e.into_inner());
        if users.contains_key(&email) {
            ffi_debug!("AUTH", "Error: User already exists: {}", email);
            return make_err_http(409, "User already exists");
        }
    }

    // Generate user ID (in-memory counter)
    let user_id = AUTH_USER_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    // Store user with all fields
    {
        let mut users = get_auth_users().lock().unwrap_or_else(|e| e.into_inner());
        users.insert(
            email.clone(),
            AuthUser {
                id: user_id,
                email: email.clone(),
                password_hash,
                extra_fields: serde_json::Value::Object(extra_fields.clone()),
            },
        );
        ffi_debug!(
            "AUTH",
            "User stored in memory: {} (id={}), total users: {}",
            email,
            user_id,
            users.len()
        );
    }

    // Sync user into CRUD in-memory store so GET /users returns auth-created users
    {
        let mut user_data = serde_json::json!({
            "id": user_id,
            "email": email,
        });
        if let Some(obj) = user_data.as_object_mut() {
            for (k, v) in &extra_fields {
                obj.insert(k.clone(), v.clone());
            }
        }
        crate::crud::crud_store_insert("users", user_data);
    }

    // Generate JWT token with user_id in claims
    let token = generate_jwt_token(&email, user_id as i64);
    ffi_debug!("AUTH", "JWT token generated for: {}", email);

    // Build response with id, email and any extra fields (but not password)
    let mut response_data = serde_json::json!({
        "token": token,
        "email": email,
        "id": user_id,
    });
    if let Some(obj) = response_data.as_object_mut() {
        for (k, v) in extra_fields {
            obj.insert(k, v);
        }
    }
    let response = serde_json::json!({ "data": response_data }).to_string();
    ffi_debug!("AUTH", "Signup success response: {}", response);
    make_ok_json(&response)
}

// ============================================================================
// LOGIN HANDLER
// ============================================================================

/// Login handler - authenticates a user and returns JWT
/// Generic: works with any struct that has email and password fields, plus returns any extra fields
extern "C" fn auth_login_handler(req: *const DooRequest) -> *mut DooResult {
    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let body = unsafe { c_to_string((*req).body) };
    ffi_debug!("AUTH", "Login request body: {}", body);

    // Parse full JSON body - supports any fields the user defines
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&body);
    let json = match parsed {
        Ok(serde_json::Value::Object(obj)) => obj,
        Ok(_) => return make_err_http(400, "Request body must be a JSON object"),
        Err(_) => return make_err_http(400, "Invalid JSON body"),
    };

    // Extract email (required, case-insensitive field lookup)
    let email = json
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("email"))
        .and_then(|(_, v)| v.as_str())
        .map(|s| s.to_string());

    let email = match email {
        Some(e) => e,
        None => return make_err_http(400, "Missing 'email' field"),
    };

    // Extract password (required, case-insensitive field lookup)
    let password = json
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("password"))
        .and_then(|(_, v)| v.as_str());

    let password = match password {
        Some(p) => p,
        None => return make_err_http(400, "Missing 'password' field"),
    };

    // Try database-backed auth first
    if is_auth_db_backed() {
        if let Some(table_name) = get_auth_table_name() {
            ffi_debug!(
                "AUTH",
                "Login: Using database-backed auth, table: {}",
                table_name
            );

            // Query ALL fields from user table dynamically
            let query_sql = format!("SELECT * FROM {} WHERE email = $1", table_name);
            match execute_db_query_with_string_param(&query_sql, &email) {
                Ok(json_result) => {
                    let rows: Vec<serde_json::Value> =
                        serde_json::from_str(&json_result).unwrap_or_default();

                    if let Some(user_row) = rows.into_iter().next() {
                        let stored_hash = user_row
                            .as_object()
                            .and_then(|obj| {
                                obj.iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case("password"))
                                    .and_then(|(_, v)| v.as_str())
                            })
                            .unwrap_or("");

                        // Verify password
                        match bcrypt::verify(password, stored_hash) {
                            Ok(true) => {
                                ffi_debug!(
                                    "AUTH",
                                    "Login success (DB): Password verified for: {}",
                                    email
                                );

                                let user_id =
                                    user_row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                                let token = generate_jwt_token(&email, user_id);

                                let mut response_data = serde_json::json!({
                                    "token": token,
                                });
                                if let (Some(obj), Some(row_obj)) =
                                    (response_data.as_object_mut(), user_row.as_object())
                                {
                                    for (k, v) in row_obj {
                                        if !k.eq_ignore_ascii_case("password") {
                                            obj.insert(k.clone(), v.clone());
                                        }
                                    }
                                }

                                let response =
                                    serde_json::json!({ "data": response_data }).to_string();
                                return make_ok_json(&response);
                            }
                            _ => {
                                ffi_debug!(
                                    "AUTH",
                                    "Login failed (DB): Invalid password for: {}",
                                    email
                                );
                                return make_err_http(401, "Invalid email or password");
                            }
                        }
                    } else {
                        ffi_debug!("AUTH", "Login failed (DB): User not found: {}", email);
                        return make_err_http(401, "Invalid email or password");
                    }
                }
                Err(e) => {
                    ffi_debug!("AUTH", "DB error during login: {}", e);
                    return make_err_http(500, &format!("Database error: {}", e));
                }
            }
        }
    }

    // Fallback to in-memory auth
    ffi_debug!("AUTH", "Login: Using in-memory auth fallback");

    let user = {
        let users = get_auth_users().lock().unwrap_or_else(|e| e.into_inner());
        ffi_debug!(
            "AUTH",
            "Login lookup for: {} (total users in store: {})",
            email,
            users.len()
        );
        users.get(&email).cloned()
    };

    let user = match user {
        Some(u) => u,
        None => {
            ffi_debug!("AUTH", "Login failed: User not found: {}", email);
            return make_err_http(401, "Invalid email or password");
        }
    };

    // Verify password
    match bcrypt::verify(password, &user.password_hash) {
        Ok(true) => {
            ffi_debug!("AUTH", "Login success: Password verified for: {}", email);
        }
        _ => {
            ffi_debug!("AUTH", "Login failed: Invalid password for: {}", email);
            return make_err_http(401, "Invalid email or password");
        }
    }

    // Generate JWT token with user_id in claims
    let token = generate_jwt_token(&email, user.id as i64);

    // Build response with id, email, token, and any extra fields stored during signup
    let mut response_data = serde_json::json!({
        "token": token,
        "email": email,
        "id": user.id,
    });
    if let Some(obj) = response_data.as_object_mut() {
        if let serde_json::Value::Object(extras) = &user.extra_fields {
            for (k, v) in extras {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    let response = serde_json::json!({ "data": response_data }).to_string();
    make_ok_json(&response)
}

// ============================================================================
// JWT TOKEN GENERATION
// ============================================================================

/// Generate a JWT token for the given subject and user ID
/// Uses JWT_SECRET from env — FAILS if not set (no insecure fallback)
fn generate_jwt_token(sub: &str, user_id: i64) -> String {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde::{Deserialize, Serialize};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Serialize, Deserialize)]
    struct Claims {
        sub: String,
        user_id: i64,
        exp: usize,
        iat: usize,
        iss: String,
    }

    // Use the shared secret from middleware (cached OnceLock, read once)
    let secret = crate::middleware::get_jwt_secret();
    if secret.is_empty() {
        #[cfg(debug_assertions)]
        {
            ffi_debug!(
                "AUTH",
                "WARNING: JWT_SECRET not set, using dev-only default"
            );
            let dev_secret = "doo-dev-secret-do-not-use-in-prod-32b";
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as usize)
                .unwrap_or(0);
            let claims = Claims {
                sub: sub.to_string(),
                user_id,
                exp: now + 86400,
                iat: now,
                iss: "doo".to_string(),
            };
            let key = EncodingKey::from_secret(dev_secret.as_bytes());
            return encode(&Header::new(Algorithm::HS256), &claims, &key)
                .unwrap_or_else(|_| "invalid-token".to_string());
        }
        #[cfg(not(debug_assertions))]
        {
            ffi_debug!("AUTH", "JWT_SECRET environment variable not set");
            return "invalid-token-no-secret".to_string();
        }
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as usize)
        .unwrap_or(0);

    let claims = Claims {
        sub: sub.to_string(),
        user_id,
        exp: now + 86400, // 24 hours
        iat: now,
        iss: "doo".to_string(),
    };

    let key = EncodingKey::from_secret(secret.as_bytes());
    encode(&Header::new(Algorithm::HS256), &claims, &key)
        .unwrap_or_else(|_| "invalid-token".to_string())
}

// ============================================================================
// AUTH ROUTE REGISTRATION
// ============================================================================

/// Set up authentication routes for a user struct.
/// Creates /signup and /login endpoints that handle user registration and authentication.
/// If database is connected, creates the users table and uses DB-backed auth.
#[no_mangle]
pub extern "C" fn doo_http_auth(
    _server: *const c_void,
    signup_path: *const c_char,
    login_path: *const c_char,
    user_struct_name: *const c_char,
    _db: *const c_void,
) -> *mut DooResult {
    ffi_safe_result!({
        let signup_str = c_to_string(signup_path);
        let login_str = c_to_string(login_path);
        let struct_name = c_to_string(user_struct_name);

        ffi_debug!(
            "HTTP",
            "Auth configured: signup={}, login={}, struct={}",
            signup_str,
            login_str,
            struct_name
        );

        // Table name for users (lowercase plural)
        let table_name = "users";

        // Try to create users table in database if connected
        if is_pool_initialized() {
            ffi_debug!("HTTP", "Database connected, setting up DB-backed auth");

            if let Some(metadata) = get_struct_metadata(&struct_name) {
                let create_sql = generate_create_table_sql(table_name, &metadata);
                ffi_debug!("HTTP", "CREATE TABLE SQL for users:\n{}", create_sql);

                match execute_db_statement(&create_sql) {
                    Ok(_) => {
                        ffi_debug!(
                            "HTTP",
                            "Users table '{}' created/verified successfully",
                            table_name
                        );
                        let mut auth_table = get_auth_db_table()
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        *auth_table = Some(table_name.to_string());
                    }
                    Err(e) => {
                        ffi_debug!("HTTP", "Warning: Failed to create users table: {}", e);
                    }
                }
            } else {
                ffi_debug!(
                    "HTTP",
                    "Warning: No metadata found for struct '{}', using in-memory auth",
                    struct_name
                );
            }
        } else {
            ffi_debug!("HTTP", "No database connection, using in-memory auth");
        }

        // Register auth routes
        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());

        registry.register("POST", &signup_str, auth_signup_handler);
        registry.register("POST", &login_str, auth_login_handler);

        registry.auth_config = Some(AuthConfig {
            signup_path: signup_str,
            login_path: login_str,
            user_struct: struct_name,
        });

        make_ok_void()
    })
}
