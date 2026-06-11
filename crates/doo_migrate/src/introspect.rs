//! Database Introspector — PostgreSQL → DatabaseSchema
//!
//! Queries `information_schema` and `pg_catalog` to build a `DatabaseSchema`
//! representing the current live database state.
//!
//! Uses `doo_ffi_db` pool for connection management — single source of truth
//! for TLS, pooling, retry, password encoding, timezone, and sslmode handling.

use deadpool_postgres::Client;

use crate::schema::*;

/// Connect to PostgreSQL using the production-grade `doo_ffi_db` pool.
/// Pool handles TLS config, retry logic, password encoding, timezone, etc.
/// Returns a `deadpool_postgres::Client` which derefs to `tokio_postgres::Client`.
pub async fn connect(
    database_url: &str,
) -> Result<deadpool_postgres::Client, String> {
    // Initialize the global pool (idempotent via OnceLock)
    doo_ffi_db::drivers::postgres::pool::init_pool(database_url)
        .await
        .map_err(|e| format!("Failed to initialize database pool: {}", e))?;

    // Get a client from the pool (sets timezone=UTC automatically)
    doo_ffi_db::drivers::postgres::pool::get_client()
        .await
        .map_err(|e| format!("Failed to get database connection: {}", e))
}

/// Introspect the current database schema.
pub async fn introspect_schema(client: &Client) -> Result<DatabaseSchema, String> {
    let mut schema = DatabaseSchema::default();

    // 1. Get all user tables
    let tables = client
        .query(
            "SELECT table_name FROM information_schema.tables
             WHERE table_schema = 'public'
             AND table_type = 'BASE TABLE'
             AND table_name != 'doo_migrations'
             ORDER BY table_name",
            &[],
        )
        .await
        .map_err(|e| format!("Failed to query tables: {}", e))?;

    for table_row in &tables {
        let table_name: String = table_row.get(0);
        let table_def = introspect_table(client, &table_name).await?;
        schema.tables.push(table_def);
    }

    // 2. Get all user-defined enum types
    let enums = client
        .query(
            "SELECT t.typname, e.enumlabel
             FROM pg_type t
             JOIN pg_enum e ON t.oid = e.enumtypid
             JOIN pg_namespace n ON t.typnamespace = n.oid
             WHERE n.nspname = 'public'
             ORDER BY t.typname, e.enumsortorder",
            &[],
        )
        .await
        .map_err(|e| format!("Failed to query enum types: {}", e))?;

    let mut enum_map: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for row in &enums {
        let type_name: String = row.get(0);
        let variant: String = row.get(1);
        enum_map
            .entry(type_name)
            .or_default()
            .push(variant);
    }

    for (name, variants) in enum_map {
        schema.enums.push(EnumTypeDef {
            name: name.clone(),
            enum_name: name, // When introspecting, enum_name = pg name
            variants,
        });
    }

    Ok(schema)
}

/// Introspect a single table.
async fn introspect_table(client: &deadpool_postgres::Client, table_name: &str) -> Result<TableDef, String> {
    // Columns
    let col_rows = client
        .query(
            "SELECT column_name, data_type, udt_name, is_nullable, column_default,
                    character_maximum_length, is_identity, identity_generation
             FROM information_schema.columns
             WHERE table_schema = 'public' AND table_name = $1
             ORDER BY ordinal_position",
            &[&table_name],
        )
        .await
        .map_err(|e| format!("Failed to query columns for {}: {}", table_name, e))?;

    let mut columns = Vec::new();
    for row in &col_rows {
        let col_name: String = row.get(0);
        let data_type: String = row.get(1);
        let udt_name: String = row.get(2);
        let is_nullable: String = row.get(3);
        let column_default: Option<String> = row.get(4);
        let char_max_len: Option<i32> = row.get(5);
        let is_identity: String = row.get(6);
        let identity_generation: Option<String> = row.get(7);

        let sql_type = if data_type == "USER-DEFINED" {
            SqlType::Enum(udt_name.clone())
        } else if data_type == "character varying" {
            SqlType::Varchar(char_max_len.unwrap_or(255) as u32)
        } else {
            SqlType::from_pg_type(&data_type)
        };

        // Detect SERIAL / auto-increment columns
        let is_auto = is_identity == "YES"
            || identity_generation.is_some()
            || column_default
                .as_ref()
                .map(|d| d.starts_with("nextval("))
                .unwrap_or(false);

        // Parse default value (skip nextval sequences — those are handled by is_auto)
        let default = column_default
            .as_ref()
            .filter(|d| !d.starts_with("nextval("))
            .map(|d| DefaultValue::from_pg_default(d));

        columns.push(ColumnDef {
            name: col_name.clone(),
            field_name: col_name.clone(), // Introspected — no Doo field name available
            sql_type,
            nullable: is_nullable == "YES",
            default,
            is_auto,
            is_primary: false, // Set below from constraints
            is_unique: false,  // Set below from constraints
            is_index: false,   // Set below from indexes
            is_hashed: false,
        });
    }

    // Primary key
    let pk_rows = client
        .query(
            "SELECT kcu.column_name, tc.constraint_name
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
               ON tc.constraint_name = kcu.constraint_name
               AND tc.table_schema = kcu.table_schema
             WHERE tc.table_schema = 'public'
               AND tc.table_name = $1
               AND tc.constraint_type = 'PRIMARY KEY'
             ORDER BY kcu.ordinal_position",
            &[&table_name],
        )
        .await
        .map_err(|e| format!("Failed to query PK for {}: {}", table_name, e))?;

    let primary_key = if !pk_rows.is_empty() {
        let pk_name: String = pk_rows[0].get(1);
        let pk_cols: Vec<String> = pk_rows.iter().map(|r| r.get::<_, String>(0)).collect();
        // Mark columns as primary
        for col in &mut columns {
            if pk_cols.contains(&col.name) {
                col.is_primary = true;
            }
        }
        Some(PrimaryKeyDef {
            name: pk_name,
            columns: pk_cols,
        })
    } else {
        None
    };

    // Unique constraints
    let uq_rows = client
        .query(
            "SELECT tc.constraint_name, kcu.column_name
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
               ON tc.constraint_name = kcu.constraint_name
               AND tc.table_schema = kcu.table_schema
             WHERE tc.table_schema = 'public'
               AND tc.table_name = $1
               AND tc.constraint_type = 'UNIQUE'
             ORDER BY tc.constraint_name, kcu.ordinal_position",
            &[&table_name],
        )
        .await
        .map_err(|e| format!("Failed to query unique constraints for {}: {}", table_name, e))?;

    let mut unique_map: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for row in &uq_rows {
        let name: String = row.get(0);
        let col: String = row.get(1);
        unique_map.entry(name).or_default().push(col);
    }
    let unique_constraints: Vec<UniqueConstraintDef> = unique_map
        .into_iter()
        .map(|(name, cols)| {
            // Mark columns as unique
            for col in &mut columns {
                if cols.contains(&col.name) {
                    col.is_unique = true;
                }
            }
            UniqueConstraintDef {
                name,
                columns: cols,
            }
        })
        .collect();

    // Foreign keys
    let fk_rows = client
        .query(
            "SELECT
                tc.constraint_name,
                kcu.column_name,
                ccu.table_name AS ref_table,
                ccu.column_name AS ref_column,
                rc.delete_rule,
                rc.update_rule
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
               ON tc.constraint_name = kcu.constraint_name
               AND tc.table_schema = kcu.table_schema
             JOIN information_schema.constraint_column_usage ccu
               ON ccu.constraint_name = tc.constraint_name
               AND ccu.table_schema = tc.table_schema
             JOIN information_schema.referential_constraints rc
               ON rc.constraint_name = tc.constraint_name
               AND rc.constraint_schema = tc.table_schema
             WHERE tc.table_schema = 'public'
               AND tc.table_name = $1
               AND tc.constraint_type = 'FOREIGN KEY'",
            &[&table_name],
        )
        .await
        .map_err(|e| format!("Failed to query foreign keys for {}: {}", table_name, e))?;

    let mut fk_map: std::collections::HashMap<String, ForeignKeyDef> =
        std::collections::HashMap::new();
    for row in &fk_rows {
        let name: String = row.get(0);
        let col: String = row.get(1);
        let ref_table: String = row.get(2);
        let ref_col: String = row.get(3);
        let on_delete: String = row.get(4);
        let on_update: String = row.get(5);

        let entry = fk_map.entry(name.clone()).or_insert_with(|| ForeignKeyDef {
            name,
            columns: Vec::new(),
            ref_table,
            ref_columns: Vec::new(),
            on_delete: ForeignKeyAction::from_pg(&on_delete),
            on_update: ForeignKeyAction::from_pg(&on_update),
        });
        if !entry.columns.contains(&col) {
            entry.columns.push(col);
        }
        if !entry.ref_columns.contains(&ref_col) {
            entry.ref_columns.push(ref_col);
        }
    }
    let foreign_keys: Vec<ForeignKeyDef> = fk_map.into_values().collect();

    // Check constraints
    let check_rows = client
        .query(
            "SELECT tc.constraint_name, cc.check_clause
             FROM information_schema.table_constraints tc
             JOIN information_schema.check_constraints cc
               ON tc.constraint_name = cc.constraint_name
               AND tc.constraint_schema = cc.constraint_schema
             WHERE tc.table_schema = 'public'
               AND tc.table_name = $1
               AND tc.constraint_type = 'CHECK'
               AND tc.constraint_name NOT LIKE '%_not_null'",
            &[&table_name],
        )
        .await
        .map_err(|e| format!("Failed to query check constraints for {}: {}", table_name, e))?;

    let check_constraints: Vec<CheckConstraintDef> = check_rows
        .iter()
        .map(|row| CheckConstraintDef {
            name: row.get(0),
            expression: row.get(1),
        })
        .collect();

    // Indexes (non-constraint)
    let idx_rows = client
        .query(
            "SELECT indexname, indexdef
             FROM pg_indexes
             WHERE schemaname = 'public'
               AND tablename = $1
               AND indexname NOT IN (
                   SELECT constraint_name FROM information_schema.table_constraints
                   WHERE table_schema = 'public' AND table_name = $1
               )",
            &[&table_name],
        )
        .await
        .map_err(|e| format!("Failed to query indexes for {}: {}", table_name, e))?;

    let mut indexes = Vec::new();
    for row in &idx_rows {
        let name: String = row.get(0);
        let indexdef: String = row.get(1);
        let unique = indexdef.to_uppercase().contains("UNIQUE");

        // Extract column names from indexdef: CREATE [UNIQUE] INDEX ... ON ... (col1, col2)
        let idx_columns = if let Some(start) = indexdef.rfind('(') {
            if let Some(end) = indexdef.rfind(')') {
                indexdef[start + 1..end]
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // Mark columns as indexed
        for col in &mut columns {
            if idx_columns.contains(&col.name) {
                col.is_index = true;
            }
        }

        indexes.push(IndexDef {
            name,
            columns: idx_columns,
            unique,
        });
    }

    Ok(TableDef {
        name: table_name.to_string(),
        struct_name: String::new(), // Introspected — no Doo struct name
        columns,
        primary_key,
        unique_constraints,
        check_constraints,
        foreign_keys,
        indexes,
        auto_timestamp: false,
    })
}
