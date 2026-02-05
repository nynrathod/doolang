//! JSON Utilities
//! Convert PostgreSQL rows to JSON.
//! 
//! IMPORTANT: Doo uses PascalCase for struct fields (e.g., AuthorId),
//! but PostgreSQL uses snake_case for column names (e.g., author_id).
//! This module provides both snake_case output (for SQL queries) and
//! PascalCase output (for Doo struct deserialization).

use serde_json::Value;
use tokio_postgres::Row;
use chrono::{DateTime, NaiveDateTime, Utc};

/// Convert snake_case to PascalCase
/// Examples: "author_id" -> "AuthorId", "published" -> "Published"
/// Special case: "id" stays as "id" (Doo convention)
fn to_pascal_case(s: &str) -> String {
    // Special case: "id" should stay lowercase
    if s == "id" {
        return "id".to_string();
    }
    
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

/// Convert a single row to JSON object
/// Column names are converted to PascalCase to match Doo struct fields
pub fn row_to_json(row: &Row) -> String {
    let mut obj = serde_json::Map::new();
    
    for (i, col) in row.columns().iter().enumerate() {
        // Convert PostgreSQL snake_case column name to Doo PascalCase field name
        let name = to_pascal_case(col.name());
        let value = row_col_to_value(row, i);
        obj.insert(name, value);
    }
    
    serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|_| "{}".to_string())
}

/// Convert multiple rows to JSON array
/// Column names are converted to PascalCase to match Doo struct fields
pub fn rows_to_json(rows: &[Row]) -> String {
    let arr: Vec<Value> = rows.iter().map(|row| {
        let mut obj = serde_json::Map::new();
        for (i, col) in row.columns().iter().enumerate() {
            // Convert PostgreSQL snake_case column name to Doo PascalCase field name
            let name = to_pascal_case(col.name());
            let value = row_col_to_value(row, i);
            obj.insert(name, value);
        }
        Value::Object(obj)
    }).collect();
    
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string())
}

/// Convert a column value to JSON value
fn row_col_to_value(row: &Row, i: usize) -> Value {
    let col = &row.columns()[i];
    
    match col.type_().name() {
        "int2" => row.try_get::<usize, Option<i16>>(i)
            .ok().flatten().map(|v| Value::from(v)).unwrap_or(Value::Null),
        "int4" => row.try_get::<usize, Option<i32>>(i)
            .ok().flatten().map(|v| Value::from(v)).unwrap_or(Value::Null),
        "int8" => row.try_get::<usize, Option<i64>>(i)
            .ok().flatten().map(|v| Value::from(v)).unwrap_or(Value::Null),
        "float4" => row.try_get::<usize, Option<f32>>(i)
            .ok().flatten().map(|v| Value::from(v as f64)).unwrap_or(Value::Null),
        "float8" => row.try_get::<usize, Option<f64>>(i)
            .ok().flatten().map(|v| Value::from(v)).unwrap_or(Value::Null),
        "bool" => row.try_get::<usize, Option<bool>>(i)
            .ok().flatten().map(|v| Value::from(v)).unwrap_or(Value::Null),
        "timestamptz" => row.try_get::<usize, Option<DateTime<Utc>>>(i)
            .ok().flatten().map(|dt| Value::String(dt.to_rfc3339())).unwrap_or(Value::Null),
        "timestamp" => row.try_get::<usize, Option<NaiveDateTime>>(i)
            .ok().flatten().map(|dt| Value::String(dt.and_utc().to_rfc3339())).unwrap_or(Value::Null),
        // JSON/JSONB: Get as string and re-parse
        "json" | "jsonb" => {
            row.try_get::<usize, Option<String>>(i)
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(Value::Null)
        }
        // UUID and other types as strings
        _ => row.try_get::<usize, Option<String>>(i)
            .ok().flatten().map(Value::from).unwrap_or(Value::Null),
    }
}

