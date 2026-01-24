//! Database Driver Trait
//!
//! Pluggable database driver interface.

use std::ffi::c_void;
use doo_ffi_core::DooResult;
use crate::error::DbError;

/// Database driver trait - pluggable implementation.
pub trait DatabaseDriver: Send + Sync {
    /// Connect to database.
    fn connect(&mut self, url: &str) -> Result<(), DbError>;
    
    /// Execute a query (no results).
    fn execute(&self, sql: &str, params: &[&str]) -> Result<u64, DbError>;
    
    /// Query with results.
    fn query(&self, sql: &str, params: &[&str]) -> Result<Vec<Vec<String>>, DbError>;
    
    /// Close connection.
    fn close(&mut self);
    
    /// Check if connected.
    fn is_connected(&self) -> bool;
}

/// PostgreSQL driver.
pub struct PostgresDriver {
    connected: bool,
    connection_url: String,
}

impl PostgresDriver {
    /// Create a new PostgreSQL driver.
    pub fn new() -> Self {
        Self {
            connected: false,
            connection_url: String::new(),
        }
    }
}

impl Default for PostgresDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl DatabaseDriver for PostgresDriver {
    fn connect(&mut self, url: &str) -> Result<(), DbError> {
        self.connection_url = url.to_string();
        self.connected = true;
        Ok(())
    }

    fn execute(&self, _sql: &str, _params: &[&str]) -> Result<u64, DbError> {
        if !self.connected {
            return Err(DbError::NotConnected);
        }
        Ok(0)
    }

    fn query(&self, _sql: &str, _params: &[&str]) -> Result<Vec<Vec<String>>, DbError> {
        if !self.connected {
            return Err(DbError::NotConnected);
        }
        Ok(Vec::new())
    }

    fn close(&mut self) {
        self.connected = false;
    }

    fn is_connected(&self) -> bool {
        self.connected
    }
}

// ============================================================================
// FFI Functions
// ============================================================================

static mut DRIVER: Option<Box<dyn DatabaseDriver>> = None;

/// Initialize PostgreSQL driver.
#[no_mangle]
pub extern "C" fn doo_db_init_postgres() {
    unsafe {
        DRIVER = Some(Box::new(PostgresDriver::new()));
    }
}

/// Connect to database.
#[no_mangle]
pub extern "C" fn doo_db_connect(url: *const i8) -> DooResult {
    if url.is_null() {
        return DooResult::err_str(400, "Null URL");
    }
    
    unsafe {
        let url_str = std::ffi::CStr::from_ptr(url)
            .to_str()
            .unwrap_or("");
        
        if let Some(driver) = &mut DRIVER {
            match driver.connect(url_str) {
                Ok(()) => DooResult::ok_empty(),
                Err(e) => DooResult::err_str(500, &format!("{:?}", e)),
            }
        } else {
            DooResult::err_str(500, "Driver not initialized")
        }
    }
}

/// Execute query.
#[no_mangle]
pub extern "C" fn doo_db_execute(sql: *const i8) -> DooResult {
    if sql.is_null() {
        return DooResult::err_str(400, "Null SQL");
    }
    
    unsafe {
        let sql_str = std::ffi::CStr::from_ptr(sql)
            .to_str()
            .unwrap_or("");
        
        if let Some(driver) = &DRIVER {
            match driver.execute(sql_str, &[]) {
                Ok(_) => DooResult::ok_empty(),
                Err(e) => DooResult::err_str(500, &format!("{:?}", e)),
            }
        } else {
            DooResult::err_str(500, "Driver not initialized")
        }
    }
}
