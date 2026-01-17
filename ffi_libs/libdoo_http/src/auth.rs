//! Auth route handlers - dynamically generated at runtime
//! Implements signup and login handlers based on struct metadata

use crate::{
    alloc_doo_response, c_to_string, make_err, make_ok_ptr, make_ok_void, string_to_c, DooHandlerFn,
    DooRequest, DooResponse, DooResult,
};
use std::collections::HashMap;
use std::ffi::c_char;
use std::sync::{Arc, Mutex};

/// Metadata for auth struct passed from compiler
#[derive(Clone, Debug)]
pub struct AuthStructMetadata {
    pub name: String,
    pub fields: Vec<AuthField>,
    pub table_name: String,
}

#[derive(Clone, Debug)]
pub struct AuthField {
    pub name: String,
    pub field_type: String, // "Str", "Int", etc.
    pub is_primary: bool,
    pub is_auto: bool,
    pub is_unique: bool,
    pub is_hash: bool, // password field
}

/// Register auth routes for a struct
pub fn register_auth_routes(
    signup_path: &str,
    login_path: &str,
    struct_metadata: AuthStructMetadata,
    db_connection: *const std::ffi::c_void,
) -> Result<(), String> {
    crate::doo_debug!("✓ Generating auth routes for {}", struct_metadata.name);
    crate::doo_debug!("  - POST {} (signup)", signup_path);
    crate::doo_debug!("  - POST {} (login)", login_path);

    // Create and register signup handler
    let signup_handler = create_signup_handler(struct_metadata.clone(), db_connection);
    register_handler_fn(signup_path, "POST", signup_handler)?;

    // Create and register login handler
    let login_handler = create_login_handler(struct_metadata, db_connection);
    register_handler_fn(login_path, "POST", login_handler)?;

    Ok(())
}

/// Create signup handler function
fn create_signup_handler(
    metadata: AuthStructMetadata,
    _db: *const std::ffi::c_void,
) -> DooHandlerFn {
    // Return a function pointer that will handle signup requests
    extern "C" fn signup_handler(request: *mut DooRequest) -> *mut DooResult {
        if request.is_null() {
            return make_err("Null request".to_string());
        }

        // TODO: Implement full signup logic:
        // 1. Parse JSON body
        // 2. Validate fields
        // 3. Hash password
        // 4. Insert into database
        // 5. Generate JWT token
        // 6. Return success response with token

        // For now, return a proper JSON response
        let response = DooResponse {
            status: 201,
            body: string_to_c(
                r#"{"success":true,"message":"User created successfully","userId":1}"#,
            ),
            content_type: string_to_c("application/json"),
        };

        let resp_ptr = alloc_doo_response(response.status, response.body, response.content_type);
        make_ok_ptr(resp_ptr as *mut _)
    }

    signup_handler
}

/// Create login handler function
fn create_login_handler(
    metadata: AuthStructMetadata,
    _db: *const std::ffi::c_void,
) -> DooHandlerFn {
    extern "C" fn login_handler(request: *mut DooRequest) -> *mut DooResult {
        if request.is_null() {
            return make_err("Null request".to_string());
        }

        // TODO: Implement full login logic:
        // 1. Parse JSON body (email/username + password)
        // 2. Query database for user
        // 3. Verify password hash
        // 4. Generate JWT token
        // 5. Return success response with token

        // For now, return a proper JSON response with mock JWT
        let response = DooResponse {
            status: 200,
            body: string_to_c(
                r#"{"success":true,"token":"eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.mock","user":{"id":1,"email":"user@example.com"}}"#,
            ),
            content_type: string_to_c("application/json"),
        };

        let resp_ptr = alloc_doo_response(response.status, response.body, response.content_type);
        make_ok_ptr(resp_ptr as *mut _)
    }

    login_handler
}

/// Helper to register a handler function with the HTTP server
fn register_handler_fn(path: &str, method: &str, handler: DooHandlerFn) -> Result<(), String> {
    use crate::get_routes;

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register(method, path, handler);
    Ok(())
}

/// Parse struct metadata from JSON string (called from compiler)
pub fn parse_struct_metadata(json: &str) -> Result<AuthStructMetadata, String> {
    // TODO: Parse JSON into AuthStructMetadata
    // For now, return a dummy
    Ok(AuthStructMetadata {
        name: "User".to_string(),
        fields: vec![],
        table_name: "users".to_string(),
    })
}
