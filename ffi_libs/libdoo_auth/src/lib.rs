mod errors;

use bcrypt::{hash, verify, DEFAULT_COST};
use errors::{AuthError, AuthErrorCode};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use doo_runtime::{
    doo_auth_debug, doo_ffi_enter, doo_ffi_exit, doo_mem_alloc, doo_mem_free, dooruntime_malloc,
    ownership::dooruntime_free_rc_string,
    memory::{track_alloc, track_free, is_freed, validate_pointer},
};

/// Thread-safe set to track freed DooResult pointers.
/// This prevents double-free by checking if a pointer was already freed.
/// NOTE: We use actual pointer tracking instead of sentinel values because
/// reading from freed memory (to check sentinel) is undefined behavior.
static FREED_AUTH_RESULTS: OnceCell<Mutex<HashSet<usize>>> = OnceCell::new();

fn get_freed_auth_results() -> &'static Mutex<HashSet<usize>> {
    FREED_AUTH_RESULTS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Check if a pointer was already freed. Returns true if already freed.
fn mark_auth_as_freed(ptr: *mut DooResult) -> bool {
    let addr = ptr as usize;
    let mut set = get_freed_auth_results().lock().unwrap();
    // insert returns false if the value was already present
    !set.insert(addr)
}

fn unmark_auth_as_freed(ptr: *mut DooResult) {
    let addr = ptr as usize;
    if let Ok(mut set) = get_freed_auth_results().lock() {
        let _ = set.remove(&addr);
    }
}

// Result type for FFI returns with ownership tracking
// tag: 0 = Ok, 1 = Err
// owner: 0 = LLVM (RC), 1 = FFI (libc), 2 = Rust (Box)
#[repr(C)]
pub struct DooResult {
    tag: i32,
    value: *mut std::ffi::c_void,
    owner: u8, // Owner enum: 0=LLVM, 1=FFI, 2=Rust
}

/// Owner enum constants for DooResult
pub mod owner {
    pub const LLVM: u8 = 0;
    pub const FFI: u8 = 1;
    pub const RUST: u8 = 2;
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

/// Convert Rust String to an RC-layout C string.
/// Layout: [rc:i32][len:i32][data...][0]
/// Returns pointer to data (base + 8).
fn string_to_c(s: String) -> *mut c_char {
    unsafe {
        let bytes = s.as_bytes();
        let len = bytes.len();

        let total_size = len + 1 + 8;
        let alloc_size = (total_size + 15) & !15;

        let ptr = dooruntime_malloc(alloc_size) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        std::ptr::write_bytes(ptr, 0, alloc_size);

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = len as i32;

        let data_ptr = ptr.add(8);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data_ptr, len);
        *data_ptr.add(len) = 0;

        data_ptr as *mut c_char
    }
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
    unsafe {
        // Allocate DooAuthError using libc::malloc
        let err_size = std::mem::size_of::<DooAuthError>();
        let err = libc::malloc(err_size) as *mut DooAuthError;
        if err.is_null() {
            return std::ptr::null_mut();
        }
        track_alloc(err as *const std::ffi::c_void, "auth_make_err_error");
        (*err).message = string_to_c(msg);

        // Allocate DooResult using libc::malloc
        let result_size = std::mem::size_of::<DooResult>();
        let ptr = libc::malloc(result_size) as *mut DooResult;
        if ptr.is_null() {
            libc::free(err as *mut libc::c_void);
            return std::ptr::null_mut();
        }
        track_alloc(ptr as *const std::ffi::c_void, "auth_make_err_result");
        unmark_auth_as_freed(ptr);
        (*ptr).tag = 1;
        (*ptr).value = err as *mut _;
        (*ptr).owner = owner::FFI;
        ptr
    }
}

fn make_err_from_auth_error(auth_err: AuthError) -> *mut DooResult {
    let err_json = auth_err.to_json_string();
    unsafe {
        // Allocate DooAuthError using libc::malloc
        let err_size = std::mem::size_of::<DooAuthError>();
        let err = libc::malloc(err_size) as *mut DooAuthError;
        if err.is_null() {
            return std::ptr::null_mut();
        }
        track_alloc(err as *const std::ffi::c_void, "auth_make_err_from_auth_error_error");
        (*err).message = string_to_c(err_json);

        // Allocate DooResult using libc::malloc
        let result_size = std::mem::size_of::<DooResult>();
        let ptr = libc::malloc(result_size) as *mut DooResult;
        if ptr.is_null() {
            libc::free(err as *mut libc::c_void);
            return std::ptr::null_mut();
        }
        track_alloc(ptr as *const std::ffi::c_void, "auth_make_err_from_auth_error_result");
        unmark_auth_as_freed(ptr);
        (*ptr).tag = 1;
        (*ptr).value = err as *mut _;
        (*ptr).owner = owner::FFI;
        ptr
    }
}

fn make_ok_string(s: String) -> *mut DooResult {
    unsafe {
        let result_size = std::mem::size_of::<DooResult>();
        let ptr = libc::malloc(result_size) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        track_alloc(ptr as *const std::ffi::c_void, "auth_make_ok_string");
        unmark_auth_as_freed(ptr);
        (*ptr).tag = 0;
        (*ptr).value = string_to_c(s) as *mut _;
        (*ptr).owner = owner::FFI;
        ptr
    }
}

fn make_ok_void() -> *mut DooResult {
    unsafe {
        let result_size = std::mem::size_of::<DooResult>();
        let ptr = libc::malloc(result_size) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        track_alloc(ptr as *const std::ffi::c_void, "auth_make_ok_void");
        unmark_auth_as_freed(ptr);
        (*ptr).tag = 0;
        (*ptr).value = std::ptr::null_mut();
        (*ptr).owner = owner::FFI;
        ptr
    }
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
    doo_ffi_enter!("doo_auth_hash_password");
    let pwd = match c_to_string(password) {
        Ok(s) => s,
        Err(e) => return make_err(e),
    };
    let result = match hash(pwd, DEFAULT_COST - 4) {
        Ok(h) => make_ok_string(h),
        Err(e) => make_err_from_auth_error(errors::internal_error(&format!("Hash failed: {}", e))),
    };
    doo_ffi_exit!("doo_auth_hash_password", "result={:p}", result);
    result
}

#[no_mangle]
pub extern "C" fn doo_auth_verify_password(
    password: *const c_char,
    hashed: *const c_char,
) -> *mut DooResult {
    doo_ffi_enter!("doo_auth_verify_password");
    let pwd = match c_to_string(password) {
        Ok(s) => s,
        Err(e) => return make_err(e),
    };
    let hashed = match c_to_string(hashed) {
        Ok(s) => s,
        Err(e) => return make_err(e),
    };
    match verify(pwd, &hashed) {
        Ok(ok) => unsafe {
            let result_size = std::mem::size_of::<DooResult>();
            let ptr = libc::malloc(result_size) as *mut DooResult;
            if ptr.is_null() {
                return std::ptr::null_mut();
            }
            track_alloc(ptr as *const std::ffi::c_void, "auth_verify_password_ok_result");
            unmark_auth_as_freed(ptr);
            (*ptr).tag = 0;
            (*ptr).value = (ok as i32) as *mut _;
            (*ptr).owner = owner::FFI;
            ptr
        },
        Err(e) => {
            make_err_from_auth_error(errors::internal_error(&format!("Verify failed: {}", e)))
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_auth_sign(
    sub: *const c_char,
    data_json: *const c_char,
    expires_seconds: i32,
) -> *mut DooResult {
    doo_ffi_enter!("doo_auth_sign", "expires_seconds={}", expires_seconds);
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
    let expires_seconds_usize = (expires_seconds as i64).max(1) as usize;
    let exp = now.saturating_add(expires_seconds_usize);
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
    doo_ffi_enter!("doo_auth_verify");
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
        dooruntime_free_rc_string(ptr as *const c_char);
    }
}

#[no_mangle]
pub extern "C" fn doo_auth_free_result(ptr: *mut DooResult) {
    if doo_mem_free!(ptr, "doo_auth_free_result_entry") {
        return;
    }
    if ptr.is_null() {
        doo_auth_debug!("doo_auth_free_result: null ptr, skip");
        return;
    }

    // CRITICAL FIX: Use thread-safe HashSet to track freed pointers instead of
    // sentinel values. The old approach wrote to freed memory (UB) and read from
    // freed memory on subsequent calls (also UB), causing heap corruption.
    if mark_auth_as_freed(ptr) {
        // Already freed - this is a double-free attempt, skip it
        doo_auth_debug!("doo_auth_free_result: DOUBLE-FREE PREVENTED ptr={:p}", ptr);
        return;
    }

    unsafe {
        let res = &*ptr; // Read-only reference, don't modify freed memory
        let owner = res.owner;
        let tag = res.tag;
        let value = res.value;

        match owner {
            owner::LLVM => {
                // LLVM allocated - RC handles cleanup, don't free value
                libc::free(ptr as *mut libc::c_void);
                return;
            }
            owner::FFI => {
                // FFI allocated the DooResult wrapper and value.
                // Key insight (same as doo_db_result_free):
                // - Error values (tag != 0): These are COPIED into the HTTP error response,
                //   so we MUST free them here to prevent leaks.
                // - OK values (tag == 0): The compiler EXTRACTS the value pointer directly
                //   and stores it in LLVM-managed memory. The value is still in use,
                //   so we must NOT free it here - LLVM RC will free it later.
                if tag != 0 && !value.is_null() {
                    // Error value - DooAuthError - FREE IT (it was copied to response)
                    let err_ptr = value as *mut DooAuthError;
                    if !err_ptr.is_null() {
                        if !(*err_ptr).message.is_null() {
                            dooruntime_free_rc_string((*err_ptr).message as *const c_char);
                        }
                        libc::free(err_ptr as *mut libc::c_void);
                    }
                }
                // NOTE: OK values (tag == 0) are NOT freed here - compiler owns them via LLVM RC
                // Free ONLY the result wrapper
                libc::free(ptr as *mut libc::c_void);
            }
            owner::RUST => {
                // Rust Box allocated - shouldn't happen in normal flow
                libc::free(ptr as *mut libc::c_void);
            }
            _ => {
                // Unknown owner - default to FFI behavior
                libc::free(ptr as *mut libc::c_void);
            }
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
