//! Schema Types — Single Source of Truth
//!
//! Central schema representation used by both the extractor (desired state from .doo)
//! and the introspector (current state from PostgreSQL). The diff engine compares
//! two `DatabaseSchema` instances to produce migration changes.
//!
//! All types are plain data — no behavior, no database dependencies.

use serde::{Deserialize, Serialize};

// ============================================================================
// Top-Level Schema
// ============================================================================

/// Complete database schema — tables + enum types.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DatabaseSchema {
    pub tables: Vec<TableDef>,
    pub enums: Vec<EnumTypeDef>,
}

// ============================================================================
// Table Definition
// ============================================================================

/// A database table derived from a `@table` struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableDef {
    /// PostgreSQL table name (snake_case, possibly pluralized).
    pub name: String,
    /// Original Doo struct name (PascalCase) — used for rename detection.
    pub struct_name: String,
    /// Columns in definition order.
    pub columns: Vec<ColumnDef>,
    /// Primary key constraint (single or composite).
    pub primary_key: Option<PrimaryKeyDef>,
    /// UNIQUE constraints.
    pub unique_constraints: Vec<UniqueConstraintDef>,
    /// CHECK constraints.
    pub check_constraints: Vec<CheckConstraintDef>,
    /// Foreign key constraints.
    pub foreign_keys: Vec<ForeignKeyDef>,
    /// Indexes (non-constraint).
    pub indexes: Vec<IndexDef>,
    /// Whether `@autoTimestamp` is set on the struct.
    pub auto_timestamp: bool,
}

// ============================================================================
// Column Definition
// ============================================================================

/// A database column derived from a struct field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDef {
    /// PostgreSQL column name (snake_case).
    pub name: String,
    /// Original Doo field name (camelCase/PascalCase).
    pub field_name: String,
    /// SQL data type.
    pub sql_type: SqlType,
    /// Whether the column allows NULL.
    pub nullable: bool,
    /// Default value expression.
    pub default: Option<DefaultValue>,
    /// `@auto` / `@autoIncrement` — GENERATED ALWAYS AS IDENTITY.
    pub is_auto: bool,
    /// `@primary` — part of primary key.
    pub is_primary: bool,
    /// `@unique` — has unique constraint.
    pub is_unique: bool,
    /// `@index` — has index.
    pub is_index: bool,
    /// `@hash` — stored hashed (informational only, doesn't affect DDL).
    pub is_hashed: bool,
}

// ============================================================================
// SQL Types
// ============================================================================

/// PostgreSQL column types mapped from Doo types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SqlType {
    // Integer family
    SmallInt,
    Integer,
    BigInt,
    Serial,
    BigSerial,
    // Floating point
    Real,
    DoublePrecision,
    // Text
    Text,
    Varchar(u32),
    // Boolean
    Boolean,
    // Timestamps
    Timestamp,
    TimestampTz,
    // JSON
    Json,
    Jsonb,
    // Enum (references a PostgreSQL enum type by name)
    Enum(String),
    // Fallback for introspected types we don't directly map
    Custom(String),
}

impl SqlType {
    /// PostgreSQL DDL type name.
    pub fn to_ddl(&self) -> String {
        match self {
            Self::SmallInt => "SMALLINT".to_string(),
            Self::Integer => "INTEGER".to_string(),
            Self::BigInt => "BIGINT".to_string(),
            Self::Serial => "SERIAL".to_string(),
            Self::BigSerial => "BIGSERIAL".to_string(),
            Self::Real => "REAL".to_string(),
            Self::DoublePrecision => "DOUBLE PRECISION".to_string(),
            Self::Text => "TEXT".to_string(),
            Self::Varchar(n) => format!("VARCHAR({})", n),
            Self::Boolean => "BOOLEAN".to_string(),
            Self::Timestamp => "TIMESTAMP".to_string(),
            Self::TimestampTz => "TIMESTAMPTZ".to_string(),
            Self::Json => "JSON".to_string(),
            Self::Jsonb => "JSONB".to_string(),
            Self::Enum(name) => name.clone(),
            Self::Custom(s) => s.clone(),
        }
    }

    /// Parse a PostgreSQL type string from information_schema into SqlType.
    pub fn from_pg_type(pg_type: &str) -> Self {
        match pg_type.to_lowercase().as_str() {
            "smallint" | "int2" => Self::SmallInt,
            "integer" | "int" | "int4" => Self::Integer,
            "bigint" | "int8" => Self::BigInt,
            "serial" => Self::Serial,
            "bigserial" => Self::BigSerial,
            "real" | "float4" => Self::Real,
            "double precision" | "float8" => Self::DoublePrecision,
            "text" => Self::Text,
            "boolean" | "bool" => Self::Boolean,
            "timestamp without time zone" | "timestamp" => Self::Timestamp,
            "timestamp with time zone" | "timestamptz" => Self::TimestampTz,
            "json" => Self::Json,
            "jsonb" => Self::Jsonb,
            other => {
                // Check for varchar(n)
                if let Some(rest) = other.strip_prefix("character varying") {
                    if let Some(inner) = rest
                        .trim()
                        .strip_prefix('(')
                        .and_then(|s| s.strip_suffix(')'))
                    {
                        if let Ok(n) = inner.trim().parse::<u32>() {
                            return Self::Varchar(n);
                        }
                    }
                    return Self::Varchar(255);
                }
                // USER-DEFINED = likely an enum
                if other == "user-defined" {
                    return Self::Custom("USER-DEFINED".to_string());
                }
                Self::Custom(other.to_string())
            }
        }
    }

    /// Return a type-appropriate "zero default" value for backfilling NULL rows
    /// when a column has no explicit default defined.
    pub fn zero_default(&self) -> DefaultValue {
        use SqlType::*;
        match self {
            SmallInt | Integer | BigInt | Serial | BigSerial => DefaultValue::Integer(0),
            Real | DoublePrecision => DefaultValue::Float(0.0),
            Text | Varchar(_) => DefaultValue::String(String::new()),
            Boolean => DefaultValue::Boolean(false),
            Timestamp | TimestampTz => DefaultValue::Expression("NOW()".to_string()),
            Json | Jsonb => DefaultValue::Expression("'{}'::jsonb".to_string()),
            Enum(_) => DefaultValue::String(String::new()),
            Custom(_) => DefaultValue::String(String::new()),
        }
    }

    /// Check if this type can be safely cast to another type.
    pub fn is_safe_cast_to(&self, target: &SqlType) -> bool {
        use SqlType::*;
        matches!(
            (self, target),
            // Widening integer casts
            (SmallInt, Integer) | (SmallInt, BigInt) | (Integer, BigInt) |
            // Int to float (safe, no data loss for reasonable values)
            (SmallInt, Real) | (SmallInt, DoublePrecision) |
            (Integer, DoublePrecision) |
            // Float widening
            (Real, DoublePrecision) |
            // Varchar widening
            (Varchar(_), Text) |
            // Timestamp precision
            (Timestamp, TimestampTz) |
            // JSON upgrade
            (Json, Jsonb) |
            // Anything to Text (safe for display)
            (_, Text)
        )
    }
}

impl std::fmt::Display for SqlType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_ddl())
    }
}

// ============================================================================
// Default Values
// ============================================================================

/// Column default value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DefaultValue {
    /// Integer literal: `DEFAULT 0`
    Integer(i64),
    /// Float literal: `DEFAULT 0.0`
    Float(f64),
    /// Boolean literal: `DEFAULT true`
    Boolean(bool),
    /// String literal: `DEFAULT 'value'`
    String(String),
    /// SQL expression: `DEFAULT NOW()`, `DEFAULT gen_random_uuid()`
    Expression(String),
}

impl DefaultValue {
    /// Convert to SQL expression string.
    pub fn to_sql(&self) -> String {
        match self {
            Self::Integer(n) => n.to_string(),
            Self::Float(f) => format!("{}", f),
            Self::Boolean(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Self::String(s) => format!("'{}'", s.replace('\'', "''")),
            Self::Expression(e) => e.clone(),
        }
    }

    /// Parse a PostgreSQL default expression into DefaultValue.
    pub fn from_pg_default(expr: &str) -> Self {
        let expr = expr.trim();

        // Strip type casts: 'value'::text → 'value'
        let expr = if let Some(pos) = expr.find("::") {
            expr[..pos].trim()
        } else {
            expr
        };

        // Boolean
        if expr.eq_ignore_ascii_case("true") {
            return Self::Boolean(true);
        }
        if expr.eq_ignore_ascii_case("false") {
            return Self::Boolean(false);
        }

        // Integer
        if let Ok(n) = expr.parse::<i64>() {
            return Self::Integer(n);
        }

        // Float
        if let Ok(f) = expr.parse::<f64>() {
            return Self::Float(f);
        }

        // String literal: 'value' — but first try to parse as number
        // (PostgreSQL wraps numeric defaults in quotes for some column types,
        // e.g. DOUBLE PRECISION stores `DEFAULT 0` as `'0'::numeric`)
        if expr.starts_with('\'') && expr.ends_with('\'') && expr.len() >= 2 {
            let inner = expr[1..expr.len() - 1].trim();
            // Try integer
            if let Ok(n) = inner.parse::<i64>() {
                return Self::Integer(n);
            }
            // Try float
            if let Ok(f) = inner.parse::<f64>() {
                return Self::Float(f);
            }
            // Try boolean
            if inner.eq_ignore_ascii_case("true") {
                return Self::Boolean(true);
            }
            if inner.eq_ignore_ascii_case("false") {
                return Self::Boolean(false);
            }
            return Self::String(inner.replace("''", "'"));
        }

        // Everything else is an expression (NOW(), gen_random_uuid(), etc.)
        // Normalize to uppercase so `now()` == `NOW()` in diffs
        Self::Expression(expr.to_uppercase())
    }
}

// ============================================================================
// Constraints
// ============================================================================

/// Primary key constraint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrimaryKeyDef {
    /// Constraint name (auto-generated: `{table}_pkey`).
    pub name: String,
    /// Columns in the primary key.
    pub columns: Vec<String>,
}

/// UNIQUE constraint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniqueConstraintDef {
    /// Constraint name (auto-generated: `{table}_{col}_key`).
    pub name: String,
    /// Columns in the unique constraint.
    pub columns: Vec<String>,
}

/// CHECK constraint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckConstraintDef {
    /// Constraint name.
    pub name: String,
    /// SQL expression for the check.
    pub expression: String,
}

/// Foreign key constraint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyDef {
    /// Constraint name (auto-generated: `{table}_{col}_fkey`).
    pub name: String,
    /// Local column(s).
    pub columns: Vec<String>,
    /// Referenced table.
    pub ref_table: String,
    /// Referenced column(s).
    pub ref_columns: Vec<String>,
    /// ON DELETE action.
    pub on_delete: ForeignKeyAction,
    /// ON UPDATE action.
    pub on_update: ForeignKeyAction,
}

/// Foreign key referential action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForeignKeyAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ForeignKeyAction {
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::NoAction => "NO ACTION",
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
        }
    }

    pub fn from_pg(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "CASCADE" | "c" => Self::Cascade,
            "RESTRICT" | "r" => Self::Restrict,
            "SET NULL" | "n" => Self::SetNull,
            "SET DEFAULT" | "d" => Self::SetDefault,
            _ => Self::NoAction,
        }
    }
}

// ============================================================================
// Index Definition
// ============================================================================

/// Database index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDef {
    /// Index name (auto-generated: `idx_{table}_{col}`).
    pub name: String,
    /// Columns in the index.
    pub columns: Vec<String>,
    /// Whether this is a unique index.
    pub unique: bool,
}

// ============================================================================
// Enum Type Definition
// ============================================================================

/// PostgreSQL enum type (CREATE TYPE ... AS ENUM).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumTypeDef {
    /// PostgreSQL type name (snake_case of Doo enum name).
    pub name: String,
    /// Original Doo enum name.
    pub enum_name: String,
    /// Variant values as strings.
    pub variants: Vec<String>,
}
