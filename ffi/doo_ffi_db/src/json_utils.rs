//! JSON Utilities
//! Convert PostgreSQL rows to JSON.

use serde_json::Value;
use tokio_postgres::Row;
use chrono::{DateTime, NaiveDateTime, Utc};

/// Convert a single row to JSON object
pub fn row_to_json(row: &Row) -> String {
    let mut obj = serde_json::Map::new();
    
    for (i, col) in row.columns().iter().enumerate() {
        let name = col.name().to_string();
        let value = row_col_to_value(row, i);
        obj.insert(name, value);
    }
    
    serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|_| "{}".to_string())
}

/// Convert multiple rows to JSON array
pub fn rows_to_json(rows: &[Row]) -> String {
    let arr: Vec<Value> = rows.iter().map(|row| {
        let mut obj = serde_json::Map::new();
        for (i, col) in row.columns().iter().enumerate() {
            let name = col.name().to_string();
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

