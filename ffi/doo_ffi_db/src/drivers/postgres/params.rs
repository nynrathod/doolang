//! PostgreSQL Parameter Conversion
//!
//! Converts `serde_json::Value` parameters to `tokio_postgres::types::ToSql`.
//! This is the SINGLE SOURCE OF TRUTH for Doo→PostgreSQL param type mapping.

use tokio_postgres::types::{ToSql, Type};

/// Convert a single JSON value to a PostgreSQL parameter, using the PG-inferred
/// type to ensure compatibility.
///
/// PostgreSQL infers parameter types from query context (e.g.,
/// `WHERE age > $1` infers INT4 from the `age` column). For bare `SELECT $1`,
/// PostgreSQL returns UNKNOWN. We adapt to whatever PostgreSQL expects.
pub fn json_value_to_typed_param(
    v: &serde_json::Value,
    pg_type: &Type,
) -> Box<dyn ToSql + Sync + Send> {
    // Handle NULL upfront for ALL types — prevents type mismatch when
    // a null JSON value is boxed as String but PG expects a numeric type.
    if matches!(v, serde_json::Value::Null) {
        return match pg_type {
            &Type::INT2 => Box::new(None::<i16>),
            &Type::INT4 | &Type::OID => Box::new(None::<i32>),
            &Type::INT8 => Box::new(None::<i64>),
            &Type::FLOAT4 => Box::new(None::<f32>),
            &Type::FLOAT8 | &Type::NUMERIC => Box::new(None::<f64>),
            &Type::BOOL => Box::new(None::<bool>),
            _ => Box::new(None::<String>),
        };
    }

    match pg_type {
        // Array types
        &Type::INT2_ARRAY => match v {
            serde_json::Value::Array(items) => Box::new(
                items
                    .iter()
                    .map(|x| match x {
                        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) as i16,
                        serde_json::Value::String(s) => s.parse::<i16>().unwrap_or(0),
                        serde_json::Value::Bool(b) => {
                            if *b {
                                1i16
                            } else {
                                0i16
                            }
                        }
                        _ => 0i16,
                    })
                    .collect::<Vec<i16>>(),
            ),
            _ => Box::new(Vec::<i16>::new()),
        },
        &Type::INT4_ARRAY => match v {
            serde_json::Value::Array(items) => Box::new(
                items
                    .iter()
                    .map(|x| match x {
                        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
                        serde_json::Value::String(s) => s.parse::<i32>().unwrap_or(0),
                        serde_json::Value::Bool(b) => {
                            if *b {
                                1i32
                            } else {
                                0i32
                            }
                        }
                        _ => 0i32,
                    })
                    .collect::<Vec<i32>>(),
            ),
            _ => Box::new(Vec::<i32>::new()),
        },
        &Type::INT8_ARRAY => match v {
            serde_json::Value::Array(items) => Box::new(
                items
                    .iter()
                    .map(|x| match x {
                        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
                        serde_json::Value::String(s) => s.parse::<i64>().unwrap_or(0),
                        serde_json::Value::Bool(b) => {
                            if *b {
                                1i64
                            } else {
                                0i64
                            }
                        }
                        _ => 0i64,
                    })
                    .collect::<Vec<i64>>(),
            ),
            _ => Box::new(Vec::<i64>::new()),
        },
        &Type::FLOAT4_ARRAY => match v {
            serde_json::Value::Array(items) => Box::new(
                items
                    .iter()
                    .map(|x| match x {
                        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0) as f32,
                        serde_json::Value::String(s) => s.parse::<f32>().unwrap_or(0.0),
                        serde_json::Value::Bool(b) => {
                            if *b {
                                1.0f32
                            } else {
                                0.0f32
                            }
                        }
                        _ => 0.0f32,
                    })
                    .collect::<Vec<f32>>(),
            ),
            _ => Box::new(Vec::<f32>::new()),
        },
        &Type::FLOAT8_ARRAY => match v {
            serde_json::Value::Array(items) => Box::new(
                items
                    .iter()
                    .map(|x| match x {
                        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
                        serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
                        serde_json::Value::Bool(b) => {
                            if *b {
                                1.0f64
                            } else {
                                0.0f64
                            }
                        }
                        _ => 0.0f64,
                    })
                    .collect::<Vec<f64>>(),
            ),
            _ => Box::new(Vec::<f64>::new()),
        },
        &Type::BOOL_ARRAY => match v {
            serde_json::Value::Array(items) => Box::new(
                items
                    .iter()
                    .map(|x| match x {
                        serde_json::Value::Bool(b) => *b,
                        serde_json::Value::String(s) => s == "true" || s == "1",
                        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
                        _ => false,
                    })
                    .collect::<Vec<bool>>(),
            ),
            _ => Box::new(Vec::<bool>::new()),
        },
        &Type::TEXT_ARRAY | &Type::VARCHAR_ARRAY | &Type::BPCHAR_ARRAY | &Type::NAME_ARRAY => {
            match v {
                serde_json::Value::Array(items) => Box::new(
                    items
                        .iter()
                        .map(|x| match x {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Null => String::new(),
                            _ => x.to_string(),
                        })
                        .collect::<Vec<String>>(),
                ),
                _ => Box::new(Vec::<String>::new()),
            }
        }
        // Integer types
        &Type::INT2 => match v {
            serde_json::Value::Number(n) => Box::new(n.as_i64().unwrap_or(0) as i16),
            serde_json::Value::String(s) => Box::new(s.parse::<i16>().unwrap_or(0)),
            serde_json::Value::Bool(b) => Box::new(if *b { 1i16 } else { 0i16 }),
            _ => Box::new(0i16),
        },
        &Type::INT4 | &Type::OID => match v {
            serde_json::Value::Number(n) => Box::new(n.as_i64().unwrap_or(0) as i32),
            serde_json::Value::String(s) => Box::new(s.parse::<i32>().unwrap_or(0)),
            serde_json::Value::Bool(b) => Box::new(if *b { 1i32 } else { 0i32 }),
            _ => Box::new(0i32),
        },
        &Type::INT8 => match v {
            serde_json::Value::Number(n) => Box::new(n.as_i64().unwrap_or(0)),
            serde_json::Value::String(s) => Box::new(s.parse::<i64>().unwrap_or(0)),
            serde_json::Value::Bool(b) => Box::new(if *b { 1i64 } else { 0i64 }),
            _ => Box::new(0i64),
        },
        // Float types
        &Type::FLOAT4 => match v {
            serde_json::Value::Number(n) => Box::new(n.as_f64().unwrap_or(0.0) as f32),
            serde_json::Value::String(s) => Box::new(s.parse::<f32>().unwrap_or(0.0)),
            serde_json::Value::Bool(b) => Box::new(if *b { 1.0f32 } else { 0.0f32 }),
            _ => Box::new(0.0f32),
        },
        &Type::FLOAT8 | &Type::NUMERIC => match v {
            serde_json::Value::Number(n) => Box::new(n.as_f64().unwrap_or(0.0)),
            serde_json::Value::String(s) => Box::new(s.parse::<f64>().unwrap_or(0.0)),
            serde_json::Value::Bool(b) => Box::new(if *b { 1.0f64 } else { 0.0f64 }),
            _ => Box::new(0.0f64),
        },
        // Boolean
        &Type::BOOL => match v {
            serde_json::Value::Bool(b) => Box::new(*b),
            serde_json::Value::String(s) => Box::new(s == "true" || s == "1"),
            serde_json::Value::Number(n) => Box::new(n.as_i64().unwrap_or(0) != 0),
            _ => Box::new(false),
        },
        // Text types + UNKNOWN — always send as String
        &Type::TEXT | &Type::VARCHAR | &Type::BPCHAR | &Type::NAME => match v {
            serde_json::Value::String(s) => Box::new(s.clone()),
            serde_json::Value::Null => Box::new(None::<String>),
            _ => Box::new(v.to_string()),
        },
        // NULL
        _ if matches!(v, serde_json::Value::Null) => Box::new(None::<String>),
        // UNKNOWN or any unrecognized type — send as String (safe default)
        _ => match v {
            serde_json::Value::String(s) => Box::new(s.clone()),
            serde_json::Value::Null => Box::new(None::<String>),
            _ => Box::new(v.to_string()),
        },
    }
}

/// Convert JSON values to PostgreSQL parameters with PG type inference.
pub fn json_values_to_pg_params_typed(
    values: &[serde_json::Value],
    pg_types: &[Type],
) -> Vec<Box<dyn ToSql + Sync + Send>> {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if i < pg_types.len() {
                json_value_to_typed_param(v, &pg_types[i])
            } else {
                json_value_to_untyped_param(v)
            }
        })
        .collect()
}

/// Convert JSON values to PostgreSQL parameter types WITHOUT type info (fallback).
pub fn json_values_to_pg_params(values: &[serde_json::Value]) -> Vec<Box<dyn ToSql + Sync + Send>> {
    values.iter().map(json_value_to_untyped_param).collect()
}

/// Convert a single JSON value to a PG parameter without type context.
pub fn json_value_to_untyped_param(v: &serde_json::Value) -> Box<dyn ToSql + Sync + Send> {
    match v {
        serde_json::Value::Array(items) => Box::new(
            items
                .iter()
                .map(|x| match x {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Null => String::new(),
                    _ => x.to_string(),
                })
                .collect::<Vec<String>>(),
        ),
        serde_json::Value::String(s) => Box::new(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                    Box::new(i as i32)
                } else {
                    Box::new(i)
                }
            } else if let Some(f) = n.as_f64() {
                Box::new(f)
            } else {
                Box::new(n.to_string())
            }
        }
        serde_json::Value::Bool(b) => Box::new(*b),
        serde_json::Value::Null => Box::new(None::<String>),
        _ => Box::new(v.to_string()),
    }
}

/// Build a refs slice from boxed params.
pub fn params_as_refs(boxed: &[Box<dyn ToSql + Sync + Send>]) -> Vec<&(dyn ToSql + Sync)> {
    boxed
        .iter()
        .map(|b| b.as_ref() as &(dyn ToSql + Sync))
        .collect()
}
