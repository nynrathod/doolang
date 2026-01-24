//! Database Errors
//!
//! Centralized database error codes.

use serde::{Deserialize, Serialize};
use doo_ffi_core::Rfc7807Error;

/// Database error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbError {
    /// Not connected
    NotConnected,
    /// Connection failed
    ConnectionFailed,
    /// Query failed
    QueryFailed,
    /// Unique violation (23505)
    UniqueViolation,
    /// Foreign key violation (23503)
    ForeignKeyViolation,
    /// Not null violation (23502)
    NotNullViolation,
    /// Check violation (23514)
    CheckViolation,
    /// Invalid SQL
    InvalidSql,
    /// Table not found
    TableNotFound,
    /// Column not found
    ColumnNotFound,
    /// Data type mismatch
    DataTypeMismatch,
    /// Transaction failed
    TransactionFailed,
    /// Internal error
    InternalError,
}

impl DbError {
    /// Get error code.
    pub fn code(&self) -> u16 {
        match self {
            Self::NotConnected => 5001,
            Self::ConnectionFailed => 5002,
            Self::QueryFailed => 5003,
            Self::UniqueViolation => 5004,
            Self::ForeignKeyViolation => 5005,
            Self::NotNullViolation => 5006,
            Self::CheckViolation => 5007,
            Self::InvalidSql => 5008,
            Self::TableNotFound => 5009,
            Self::ColumnNotFound => 5010,
            Self::DataTypeMismatch => 5011,
            Self::TransactionFailed => 5012,
            Self::InternalError => 5099,
        }
    }

    /// Get error message.
    pub fn message(&self) -> &'static str {
        match self {
            Self::NotConnected => "Not connected to database",
            Self::ConnectionFailed => "Failed to connect to database",
            Self::QueryFailed => "Query execution failed",
            Self::UniqueViolation => "Duplicate key value",
            Self::ForeignKeyViolation => "Foreign key constraint violated",
            Self::NotNullViolation => "Not null constraint violated",
            Self::CheckViolation => "Check constraint violated",
            Self::InvalidSql => "Invalid SQL syntax",
            Self::TableNotFound => "Table not found",
            Self::ColumnNotFound => "Column not found",
            Self::DataTypeMismatch => "Data type mismatch",
            Self::TransactionFailed => "Transaction failed",
            Self::InternalError => "Internal database error",
        }
    }

    /// Convert to RFC 7807 error.
    pub fn to_rfc7807(&self) -> Rfc7807Error {
        Rfc7807Error::internal(self.message())
    }
}
