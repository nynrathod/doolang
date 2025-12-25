mod errors;

use bcrypt::{hash, verify, DEFAULT_COST};
use errors::{AuthError, AuthErrorCode};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::env;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[repr(C)]
pub struct DooResult {
    tag: i32,                     // 0 = Ok, 1 = Err
    value: *mut std::ffi::c_void, // pointer to data or error struct
}

#[repr(C)]
pub struct DooAuthError {
    message: *mut c_char,
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
    iat: usize,
    // Generic map payload is avoided to keep ABI simple. Caller can embed JSON string.
    data: Option<String>,
}

static ENCODING: OnceCell<EncodingKey> = OnceCell::new();
static DECODING: OnceCell<DecodingKey> = OnceCell::new();

fn string_to_c(s: String) -> *mut c_char {
    CString::new(s)
        .map(|c| c.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn c_to_string(s: *const c_char) -> Result<String, String> {
    if s.is_null() {
        return Err("Null pointer".to_string());
    }
    unsafe {
        CStr::from_ptr(s)
            .to_str()
            .map(|s| s.to_string())
            .map_err(|e| format!("Invalid UTF-8: {e}"))
    }
}

fn make_err(msg: String) -> *mut DooResult {
    let err = Box::new(DooAuthError {
        message: string_to_c(msg),
    });
    Box::into_raw(Box::new(DooResult {
        tag: 1,
        value: Box::into_raw(err) as *mut _,
    }))
}

fn make_err_from_auth_error(auth_err: AuthError) -> *mut DooResult {
    let err_json = auth_err.to_json_string();
    let err = Box::new(DooAuthError {
        message: string_to_c(err_json),
    });
    Box::into_raw(Box::new(DooResult {
        tag: 1,
        value: Box::into_raw(err) as *mut _,
    }))
}

fn make_ok_string(s: String) -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: string_to_c(s) as *mut _,
    }))
}

fn make_ok_void() -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: std::ptr::null_mut(),
    }))
}

fn ensure_keys() -> Result<(EncodingKey, DecodingKey), String> {
    let secret = env::var("JWT_SECRET").map_err(|_| "JWT_SECRET not set".to_string())?;
    let enc = ENCODING
        .get_or_init(|| EncodingKey::from_secret(secret.as_bytes()))
        .clone();
    let dec = DECODING
        .get_or_init(|| DecodingKey::from_secret(secret.as_bytes()))
        .clone();
    Ok((enc, dec))
}

#[no_mangle]
pub extern "C" fn doo_auth_hash_password(password: *const c_char) -> *mut DooResult {
    let pwd = match c_to_string(password) {
        Ok(s) => s,
        Err(e) => return make_err(e),
    };
    match hash(pwd, DEFAULT_COST - 4) {
        Ok(h) => make_ok_string(h),
        Err(e) => make_err_from_auth_error(errors::internal_error(&format!("Hash failed: {}", e))),
    }
}

#[no_mangle]
pub extern "C" fn doo_auth_verify_password(
    password: *const c_char,
    hashed: *const c_char,
) -> *mut DooResult {
    let pwd = match c_to_string(password) {
        Ok(s) => s,
        Err(e) => return make_err(e),
    };
    let hashed = match c_to_string(hashed) {
        Ok(s) => s,
        Err(e) => return make_err(e),
    };
    match verify(pwd, &hashed) {
        Ok(ok) => Box::into_raw(Box::new(DooResult {
            tag: 0,
            value: (ok as i32) as *mut _,
        })),
        Err(e) => {
            make_err_from_auth_error(errors::internal_error(&format!("Verify failed: {}", e)))
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_auth_sign(
    sub: *const c_char,
    data_json: *const c_char,
    expires_seconds: i64,
) -> *mut DooResult {
    let (enc, _) = match ensure_keys() {
        Ok(v) => v,
        Err(_) => return make_err_from_auth_error(errors::jwt_secret_missing()),
    };
    let sub = match c_to_string(sub) {
        Ok(s) => s,
        Err(e) => return make_err(e),
    };
    let data = if data_json.is_null() {
        None
    } else {
        match c_to_string(data_json) {
            Ok(s) => Some(s),
            Err(e) => return make_err_from_auth_error(errors::jwt_secret_missing()),
        }
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as usize)
        .unwrap_or(0);
    let exp = now.saturating_add(expires_seconds.max(1) as usize);
    let claims = Claims {
        sub,
        exp,
        iat: now,
        data,
    };
    match encode(&Header::new(Algorithm::HS256), &claims, &enc) {
        Ok(token) => make_ok_string(token),
        Err(e) => {
            make_err_from_auth_error(errors::internal_error(&format!("JWT sign failed: {}", e)))
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_auth_verify(token: *const c_char) -> *mut DooResult {
    let (_, dec) = match ensure_keys() {
        Ok(v) => v,
        Err(_) => return make_err_from_auth_error(errors::jwt_secret_missing()),
    };
    let tok = match c_to_string(token) {
        Ok(s) => s,
        Err(e) => return make_err(e),
    };
    let validation = Validation::new(Algorithm::HS256);
    match decode::<Claims>(&tok, &dec, &validation) {
        Ok(data) => {
            let json = serde_json::to_string(&data.claims).unwrap_or_else(|_| "{}".to_string());
            make_ok_string(json)
        }
        Err(e) => {
            let err_str = e.to_string().to_lowercase();
            let auth_err = if err_str.contains("expired") {
                errors::jwt_expired()
            } else {
                errors::jwt_invalid()
            };
            make_err_from_auth_error(auth_err)
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_auth_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = CString::from_raw(ptr);
    }
}

#[no_mangle]
pub extern "C" fn doo_auth_free_result(ptr: *mut DooResult) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let res = Box::from_raw(ptr);
        if res.tag != 0 && !res.value.is_null() {
            let _ = Box::from_raw(res.value as *mut DooAuthError);
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_auth_is_error(ptr: *mut DooResult) -> i32 {
    if ptr.is_null() {
        return 1; // Treat null as error
    }
    unsafe {
        let res = &*ptr;
        res.tag
    }
}

#[no_mangle]
pub extern "C" fn doo_auth_get_error_message(ptr: *mut DooResult) -> *const c_char {
    if ptr.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let res = &*ptr;
        if res.tag != 0 && !res.value.is_null() {
            let err = &*(res.value as *const DooAuthError);
            return err.message;
        }
        std::ptr::null()
    }
}
