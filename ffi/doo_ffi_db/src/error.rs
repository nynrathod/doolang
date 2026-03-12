//! Database Errors
//!
//! Centralized database error codes — single source of truth.

use doo_ffi_core::Rfc7807Error;
use serde::{Deserialize, Serialize};

/// Database error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbError {
    NotConnected,
    ConnectionFailed,
    QueryFailed,
    UniqueViolation,
    ForeignKeyViolation,
    NotNullViolation,
    CheckViolation,
    InvalidSql,
    TableNotFound,
    ColumnNotFound,
    DataTypeMismatch,
    TransactionFailed,
    QueryTimeout,
    DatabaseOverloaded,
    RowLimitExceeded,
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
            Self::QueryTimeout => 5013,
            Self::DatabaseOverloaded => 5014,
            Self::RowLimitExceeded => 5015,
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
            Self::QueryTimeout => "Query timed out",
            Self::DatabaseOverloaded => "Database overloaded",
            Self::RowLimitExceeded => "Query returned too many rows",
            Self::InternalError => "Internal database error",
        }
    }

    /// Map HTTP status code for this error.
    pub fn http_status(&self) -> u16 {
        match self {
            Self::NotConnected | Self::ConnectionFailed => 503,
            Self::QueryFailed | Self::InvalidSql => 400,
            Self::UniqueViolation => 409,
            Self::ForeignKeyViolation => 409,
            Self::NotNullViolation | Self::CheckViolation => 400,
            Self::TableNotFound | Self::ColumnNotFound => 404,
            Self::DataTypeMismatch => 400,
            Self::TransactionFailed => 500,
            Self::QueryTimeout => 504,
            Self::DatabaseOverloaded => 503,
            Self::RowLimitExceeded => 400,
            Self::InternalError => 500,
        }
    }

    /// Map PostgreSQL SQLSTATE error code to DbError.
    pub fn from_pg_code(code: &str) -> Self {
        match code {
            "23505" => Self::UniqueViolation,
            "23503" => Self::ForeignKeyViolation,
            "23502" => Self::NotNullViolation,
            "23514" => Self::CheckViolation,
            "42601" | "42000" => Self::InvalidSql,
            "42P01" => Self::TableNotFound,
            "42703" => Self::ColumnNotFound,
            "42804" | "42846" => Self::DataTypeMismatch,
            "40001" | "40P01" => Self::TransactionFailed,
            "57014" => Self::QueryTimeout,
            "08000" | "08003" | "08006" => Self::ConnectionFailed,
            _ => Self::QueryFailed,
        }
    }

    /// Convert to RFC 7807 error.
    pub fn to_rfc7807(&self) -> Rfc7807Error {
        Rfc7807Error::new(self.http_status(), self.message())
    }
}
