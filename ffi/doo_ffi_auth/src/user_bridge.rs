//! User DB bridge — runtime DB access for OAuth user upsert and lookup.
//!
//! Resolves doo_db symbols at runtime (same pattern as doo_ffi_http/db_bridge).

use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::OnceLock;

use doo_ffi_core::ffi_debug;
use doo_ffi_core::helpers::string_to_c;

use crate::strategies::oauth::tokens::UserInfo;

struct DbSymbols {
    is_connected: unsafe extern "C" fn() -> bool,
    query_with_params: unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_void,
}

static DB_SYMBOLS: OnceLock<Option<DbSymbols>> = OnceLock::new();

fn find_symbol_in_process(name: &[u8]) -> Option<*mut c_void> {
    let name_str = std::str::from_utf8(&name[..name.len().saturating_sub(1)]).unwrap_or("");

    if let Some(ptr) = doo_ffi_core::ffi_bridge::resolve(name_str) {
        return Some(ptr as *mut c_void);
    }

    #[cfg(unix)]
    {
        let addr = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr() as *const c_char) };
        if !addr.is_null() {
            return Some(addr);
        }
    }

    #[cfg(windows)]
    {
        use std::ffi::c_void as WinVoid;
        extern "system" {
            fn GetProcAddress(hModule: *mut WinVoid, lpProcName: *const i8) -> *mut WinVoid;
            fn GetCurrentProcess() -> *mut WinVoid;
            fn K32EnumProcessModules(
                hProcess: *mut WinVoid,
                lphModule: *mut *mut WinVoid,
                cb: u32,
                lpcbNeeded: *mut u32,
            ) -> i32;
        }
        let sym = name.as_ptr() as *const i8;
        let process = unsafe { GetCurrentProcess() };
        let mut modules: [*mut WinVoid; 512] = [std::ptr::null_mut(); 512];
        let mut needed: u32 = 0;
        let ok = unsafe {
            K32EnumProcessModules(
                process,
                modules.as_mut_ptr(),
                (modules.len() * std::mem::size_of::<*mut WinVoid>()) as u32,
                &mut needed,
            )
        };
        if ok != 0 {
            let count = (needed as usize) / std::mem::size_of::<*mut WinVoid>();
            for i in 0..count.min(modules.len()) {
                if !modules[i].is_null() {
                    let addr = unsafe { GetProcAddress(modules[i], sym) };
                    if !addr.is_null() {
                        return Some(addr);
                    }
                }
            }
        }
    }

    None
}

fn get_db_symbols() -> Option<&'static DbSymbols> {
    DB_SYMBOLS
        .get_or_init(|| {
            unsafe {
                let is_connected = find_symbol_in_process(b"doo_db_is_connected\0")?;
                let query_with_params =
                    find_symbol_in_process(b"doo_db_query_with_params\0")?;
                Some(DbSymbols {
                    is_connected: std::mem::transmute(is_connected),
                    query_with_params: std::mem::transmute(query_with_params),
                })
            }
        })
        .as_ref()
}

fn is_pool_initialized() -> bool {
    let Some(syms) = get_db_symbols() else {
        return false;
    };
    unsafe { (syms.is_connected)() }
}

fn auth_table_name() -> String {
    std::env::var("AUTH_TABLE").unwrap_or_else(|_| "users".to_string())
}

fn query_with_params(sql: &str, params: &[serde_json::Value]) -> Result<String, String> {
    if !is_pool_initialized() {
        return Err("Database not connected".to_string());
    }

    let Some(syms) = get_db_symbols() else {
        return Err("Database symbols not available".to_string());
    };

    let params_json = serde_json::to_string(params).unwrap_or_else(|_| "[]".to_string());
    let sql_c = string_to_c(sql);
    let params_c = string_to_c(&params_json);

    let result = unsafe { (syms.query_with_params)(sql_c, params_c) };

    unsafe {
        libc::free(sql_c as *mut c_void);
        libc::free(params_c as *mut c_void);
    }

    if result.is_null() {
        return Err("Query execution failed".to_string());
    }

    let json = unsafe {
        let c_str = std::ffi::CStr::from_ptr(result as *const c_char);
        let s = c_str.to_string_lossy().into_owned();
        libc::free(result);
        s
    };

    Ok(json)
}

fn row_id(row: &serde_json::Value) -> Option<i64> {
    row.get("id")
        .or_else(|| row.get("Id"))
        .and_then(|v| v.as_i64())
}

/// Look up user ID by email from the auth table.
pub fn lookup_user_id_by_email(email: &str) -> Option<i64> {
    if email.is_empty() {
        return None;
    }

    let table = auth_table_name();
    let sql = format!("SELECT id FROM {} WHERE email = $1 LIMIT 1", table);
    let result_json = query_with_params(&sql, &[serde_json::json!(email)]).ok()?;
    let rows: Vec<serde_json::Value> = serde_json::from_str(&result_json).ok()?;
    rows.first().and_then(row_id)
}

/// Upsert an OAuth user and return the database user ID.
pub fn upsert_oauth_user(user_info: &UserInfo) -> Result<i64, String> {
    if user_info.email.is_empty() {
        return Err("OAuth user info missing email".to_string());
    }

    if !is_pool_initialized() {
        ffi_debug!("AUTH", "DB not available — skipping OAuth user upsert");
        return Ok(0);
    }

    let table = auth_table_name();
    let email = &user_info.email;
    let name = user_info.name.as_deref().unwrap_or("");
    let avatar = user_info.avatar.as_deref().unwrap_or("");
    let provider = &user_info.provider;

    let check_sql = format!(
        "SELECT id FROM {} WHERE email = $1 LIMIT 1",
        table
    );
    let existing = query_with_params(&check_sql, &[serde_json::json!(email)])?;
    let existing_rows: Vec<serde_json::Value> =
        serde_json::from_str(&existing).unwrap_or_default();

    if let Some(row) = existing_rows.first() {
        let user_id = row_id(row).unwrap_or(0);
        let update_sql = format!(
            "UPDATE {} SET name = $1, avatar = $2, provider = $3 WHERE email = $4",
            table
        );
        query_with_params(
            &update_sql,
            &[
                serde_json::json!(name),
                serde_json::json!(avatar),
                serde_json::json!(provider),
                serde_json::json!(email),
            ],
        )?;
        ffi_debug!("AUTH", "Updated OAuth user: email={}, id={}", email, user_id);
        return Ok(user_id);
    }

    let insert_sql = format!(
        "INSERT INTO {} (email, password, name, role, provider, avatar) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        table
    );
    let result_json = query_with_params(
        &insert_sql,
        &[
            serde_json::json!(email),
            serde_json::json!(""),
            serde_json::json!(name),
            serde_json::json!(""),
            serde_json::json!(provider),
            serde_json::json!(avatar),
        ],
    )?;
    let rows: Vec<serde_json::Value> = serde_json::from_str(&result_json).unwrap_or_default();
    let user_id = rows.first().and_then(row_id).unwrap_or(0);
    ffi_debug!(
        "AUTH",
        "Created OAuth user: email={}, id={}, provider={}",
        email,
        user_id,
        provider
    );
    Ok(user_id)
}

/// Resolve user_id for token signing — use provided id or look up by email.
pub fn resolve_user_id(user_id: i64, email: &str) -> i64 {
    if user_id > 0 {
        return user_id;
    }
    lookup_user_id_by_email(email).unwrap_or(0)
}
