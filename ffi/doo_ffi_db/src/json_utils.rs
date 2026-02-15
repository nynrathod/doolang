//! JSON Utilities — Single Source of Truth
//!
//! Convert PostgreSQL rows to JSON.
//!
//! IMPORTANT: Doo uses PascalCase for struct fields (e.g., AuthorId),
//! but PostgreSQL uses snake_case for column names (e.g., author_id).
//! All column name conversion happens here — nowhere else.
//!
//! Performance: Uses direct string buffer writing instead of intermediate
//! serde_json::Value tree. 2-5x faster for large result sets.

use chrono::{DateTime, NaiveDateTime, Utc};
use std::fmt::Write;
use tokio_postgres::Row;

/// Convert snake_case to PascalCase — SINGLE SOURCE OF TRUTH.
/// Special case: "id" stays as "id" (Doo convention).
pub fn to_pascal_case(s: &str) -> String {
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

/// Write PascalCase directly into buffer without allocating a String.
fn write_pascal_case(buf: &mut String, s: &str) {
    if s == "id" {
        buf.push_str("id");
        return;
    }
    for (idx, word) in s.split('_').enumerate() {
        let _ = idx; // suppress warning
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            for c in first.to_uppercase() {
                buf.push(c);
            }
            for c in chars {
                buf.push(c);
            }
        }
    }
}

/// Convert multiple rows to JSON array — fast direct-write path.
/// Writes JSON directly to a String buffer, avoiding intermediate Value tree.
pub fn rows_to_json(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "[]".to_string();
    }

    let mut buf = String::with_capacity(rows.len() * 256);
    buf.push('[');
    for (row_idx, row) in rows.iter().enumerate() {
        if row_idx > 0 {
            buf.push(',');
        }
        buf.push('{');
        for (i, col) in row.columns().iter().enumerate() {
            if i > 0 {
                buf.push(',');
            }
            buf.push('"');
            write_pascal_case(&mut buf, col.name());
            buf.push_str("\":");
            write_column_value(&mut buf, row, i, col);
        }
        buf.push('}');
    }
    buf.push(']');
    buf
}

/// Convert a single row to JSON object string — fast path.
pub fn row_to_json(row: &Row) -> String {
    let mut buf = String::with_capacity(256);
    buf.push('{');
    for (i, col) in row.columns().iter().enumerate() {
        if i > 0 {
            buf.push(',');
        }
        buf.push('"');
        write_pascal_case(&mut buf, col.name());
        buf.push_str("\":");
        write_column_value(&mut buf, row, i, col);
    }
    buf.push('}');
    buf
}

/// Convert a single row to serde_json::Value (needed for some legacy paths).
pub fn row_to_json_value(row: &Row) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for (i, col) in row.columns().iter().enumerate() {
        let name = to_pascal_case(col.name());
        let value = row_col_to_value(row, i);
        obj.insert(name, value);
    }
    serde_json::Value::Object(obj)
}

/// Write a column value directly into the JSON buffer — no intermediate Value.
fn write_column_value(
    buf: &mut String,
    row: &Row,
    i: usize,
    col: &tokio_postgres::Column,
) {
    match col.type_().name() {
        "int2" => match row.try_get::<usize, Option<i16>>(i) {
            Ok(Some(v)) => {
                let _ = write!(buf, "{}", v);
            }
            _ => buf.push_str("null"),
        },
        "int4" => match row.try_get::<usize, Option<i32>>(i) {
            Ok(Some(v)) => {
                let _ = write!(buf, "{}", v);
            }
            _ => buf.push_str("null"),
        },
        "int8" => match row.try_get::<usize, Option<i64>>(i) {
            Ok(Some(v)) => {
                let _ = write!(buf, "{}", v);
            }
            _ => buf.push_str("null"),
        },
        "float4" => match row.try_get::<usize, Option<f32>>(i) {
            Ok(Some(v)) => {
                if v.is_finite() {
                    let _ = write!(buf, "{}", v as f64);
                } else {
                    buf.push_str("null");
                }
            }
            _ => buf.push_str("null"),
        },
        "float8" => match row.try_get::<usize, Option<f64>>(i) {
            Ok(Some(v)) => {
                if v.is_finite() {
                    let _ = write!(buf, "{}", v);
                } else {
                    buf.push_str("null");
                }
            }
            _ => buf.push_str("null"),
        },
        "bool" => match row.try_get::<usize, Option<bool>>(i) {
            Ok(Some(v)) => buf.push_str(if v { "true" } else { "false" }),
            _ => buf.push_str("null"),
        },
        "timestamptz" => match row.try_get::<usize, Option<DateTime<Utc>>>(i) {
            Ok(Some(dt)) => {
                buf.push('"');
                buf.push_str(&dt.to_rfc3339());
                buf.push('"');
            }
            _ => buf.push_str("null"),
        },
        "timestamp" => match row.try_get::<usize, Option<NaiveDateTime>>(i) {
            Ok(Some(dt)) => {
                buf.push('"');
                buf.push_str(&dt.and_utc().to_rfc3339());
                buf.push('"');
            }
            _ => buf.push_str("null"),
        },
        "json" | "jsonb" => match row.try_get::<usize, Option<String>>(i) {
            Ok(Some(s)) => {
                // JSON values are already valid JSON — write directly
                buf.push_str(&s);
            }
            _ => buf.push_str("null"),
        },
        _ => match row.try_get::<usize, Option<String>>(i) {
            Ok(Some(v)) => write_json_string(buf, &v),
            _ => buf.push_str("null"),
        },
    }
}

/// Write a JSON-escaped string into the buffer.
fn write_json_string(buf: &mut String, s: &str) {
    buf.push('"');
    for c in s.chars() {
        match c {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(buf, "\\u{:04x}", c as u32);
            }
            c => buf.push(c),
        }
    }
    buf.push('"');
}

/// Convert a column value to serde_json::Value (for legacy code paths).
fn row_col_to_value(row: &Row, i: usize) -> serde_json::Value {
    let col = &row.columns()[i];

    match col.type_().name() {
        "int2" => row
            .try_get::<usize, Option<i16>>(i)
            .ok()
            .flatten()
            .map(|v| serde_json::Value::from(v))
            .unwrap_or(serde_json::Value::Null),
        "int4" => row
            .try_get::<usize, Option<i32>>(i)
            .ok()
            .flatten()
            .map(|v| serde_json::Value::from(v))
            .unwrap_or(serde_json::Value::Null),
        "int8" => row
            .try_get::<usize, Option<i64>>(i)
            .ok()
            .flatten()
            .map(|v| serde_json::Value::from(v))
            .unwrap_or(serde_json::Value::Null),
        "float4" => row
            .try_get::<usize, Option<f32>>(i)
            .ok()
            .flatten()
            .map(|v| serde_json::Value::from(v as f64))
            .unwrap_or(serde_json::Value::Null),
        "float8" => row
            .try_get::<usize, Option<f64>>(i)
            .ok()
            .flatten()
            .map(|v| serde_json::Value::from(v))
            .unwrap_or(serde_json::Value::Null),
        "bool" => row
            .try_get::<usize, Option<bool>>(i)
            .ok()
            .flatten()
            .map(|v| serde_json::Value::from(v))
            .unwrap_or(serde_json::Value::Null),
        "timestamptz" => row
            .try_get::<usize, Option<DateTime<Utc>>>(i)
            .ok()
            .flatten()
            .map(|dt| serde_json::Value::String(dt.to_rfc3339()))
            .unwrap_or(serde_json::Value::Null),
        "timestamp" => row
            .try_get::<usize, Option<NaiveDateTime>>(i)
            .ok()
            .flatten()
            .map(|dt| serde_json::Value::String(dt.and_utc().to_rfc3339()))
            .unwrap_or(serde_json::Value::Null),
        "json" | "jsonb" => row
            .try_get::<usize, Option<String>>(i)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(serde_json::Value::Null),
        _ => row
            .try_get::<usize, Option<String>>(i)
            .ok()
            .flatten()
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
    }
}
