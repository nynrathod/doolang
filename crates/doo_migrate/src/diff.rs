//! Schema Diff Engine
//!
//! Compares two `DatabaseSchema` instances (current vs desired) and produces
//! a list of `SchemaChange` operations representing the migration.

use std::collections::{HashMap, HashSet};

use crate::schema::*;
use serde::Serialize;

// ============================================================================
// Schema Change Types
// ============================================================================

/// A single schema change operation.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SchemaChange {
    // --- Enum Types ---
    CreateEnum(EnumTypeDef),
    AddEnumValue {
        enum_name: String,
        value: String,
    },
    DropEnum {
        name: String,
    },

    // --- Tables ---
    CreateTable(TableDef),
    DropTable {
        name: String,
    },
    RenameTable {
        from: String,
        to: String,
    },

    // --- Columns ---
    AddColumn {
        table: String,
        column: ColumnDef,
    },
    DropColumn {
        table: String,
        column: String,
    },
    RenameColumn {
        table: String,
        from: String,
        to: String,
    },
    AlterColumnType {
        table: String,
        column: String,
        from: SqlType,
        to: SqlType,
    },
    SetNotNull {
        table: String,
        column: String,
        /// The column's default value from the desired schema (if any).
        /// Used to backfill NULL rows before applying the constraint.
        default_value: Option<DefaultValue>,
        /// The column's SQL type — used to derive a zero-value default
        /// when no explicit default exists.
        sql_type: SqlType,
    },
    DropNotNull {
        table: String,
        column: String,
    },
    SetDefault {
        table: String,
        column: String,
        default: DefaultValue,
    },
    DropDefault {
        table: String,
        column: String,
    },

    // --- Constraints ---
    AddPrimaryKey {
        table: String,
        name: String,
        columns: Vec<String>,
    },
    DropPrimaryKey {
        table: String,
        name: String,
    },
    AddUnique {
        table: String,
        name: String,
        columns: Vec<String>,
    },
    DropUnique {
        table: String,
        name: String,
    },
    AddCheck {
        table: String,
        name: String,
        expression: String,
    },
    DropCheck {
        table: String,
        name: String,
    },

    // --- Indexes ---
    CreateIndex {
        table: String,
        index: IndexDef,
    },
    DropIndex {
        name: String,
    },

    // --- Foreign Keys ---
    AddForeignKey {
        table: String,
        fk: ForeignKeyDef,
    },
    DropForeignKey {
        table: String,
        name: String,
    },
}

/// Compute the diff between current and desired schemas.
pub fn compute_diff(current: &DatabaseSchema, desired: &DatabaseSchema) -> Vec<SchemaChange> {
    let mut changes = Vec::new();

    // Index tables by name for fast lookup
    let current_tables: HashMap<&str, &TableDef> = current
        .tables
        .iter()
        .map(|t| (t.name.as_str(), t))
        .collect();
    let desired_tables: HashMap<&str, &TableDef> = desired
        .tables
        .iter()
        .map(|t| (t.name.as_str(), t))
        .collect();

    let current_enums: HashMap<&str, &EnumTypeDef> =
        current.enums.iter().map(|e| (e.name.as_str(), e)).collect();
    let desired_enums: HashMap<&str, &EnumTypeDef> =
        desired.enums.iter().map(|e| (e.name.as_str(), e)).collect();

    // --- Enum changes (must come first — tables may reference enum types) ---
    diff_enums(&current_enums, &desired_enums, &mut changes);

    // --- Table changes ---

    // 0. Table rename detection: match dropped tables with new tables by structure
    let mut renamed_tables: HashSet<&str> = HashSet::new();
    for new_table in &desired.tables {
        if current_tables.contains_key(new_table.name.as_str()) {
            continue;
        }
        // Look for a dropped table (in current but not desired) with same columns
        for old_table in &current.tables {
            if desired_tables.contains_key(old_table.name.as_str()) {
                continue; // Not dropped
            }
            if renamed_tables.contains(old_table.name.as_str()) {
                continue; // Already matched
            }
            if tables_match_for_rename(old_table, new_table) {
                changes.push(SchemaChange::RenameTable {
                    from: old_table.name.clone(),
                    to: new_table.name.clone(),
                });
                renamed_tables.insert(old_table.name.as_str());
                renamed_tables.insert(new_table.name.as_str());
                break;
            }
        }
    }

    // 1. New tables (in desired but not in current, excluding renames)
    for desired_table in &desired.tables {
        if !current_tables.contains_key(desired_table.name.as_str())
            && !renamed_tables.contains(desired_table.name.as_str())
        {
            changes.push(SchemaChange::CreateTable(desired_table.clone()));
        }
    }

    // 2. Dropped tables (in current but not in desired, excluding renames)
    for current_table in &current.tables {
        if !desired_tables.contains_key(current_table.name.as_str())
            && !renamed_tables.contains(current_table.name.as_str())
        {
            changes.push(SchemaChange::DropTable {
                name: current_table.name.clone(),
            });
        }
    }

    // 3. Modified tables (in both or renamed — diff columns, constraints, indexes)
    for desired_table in &desired.tables {
        // For renamed tables, find the current table by its original name
        let current_table = if let Some(ct) = current_tables.get(desired_table.name.as_str()) {
            ct
        } else {
            // Check if this table was renamed FROM another name
            let old_name = changes.iter().find_map(|c| {
                if let SchemaChange::RenameTable { from, to } = c {
                    if to == desired_table.name.as_str() {
                        Some(from.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                }
            });
            match old_name.and_then(|n| current_tables.get(n)) {
                Some(ct) => ct,
                None => continue,
            }
        };
        diff_table(current_table, desired_table, &mut changes);
    }

    changes
}

/// Check if two tables have the same column structure (for rename detection).
fn tables_match_for_rename(current: &TableDef, desired: &TableDef) -> bool {
    if current.columns.len() != desired.columns.len() {
        return false;
    }
    for (cur_col, des_col) in current.columns.iter().zip(desired.columns.iter()) {
        // Handle Serial/Integer equivalence (auto-increment columns)
        let types_eq = cur_col.sql_type == des_col.sql_type
            || (cur_col.is_auto || des_col.is_auto)
                && matches!(
                    (&cur_col.sql_type, &des_col.sql_type),
                    (SqlType::Serial, SqlType::Integer) | (SqlType::Integer, SqlType::Serial)
                );
        if !types_eq {
            return false;
        }
    }
    true
}

/// Diff enum types.
fn diff_enums(
    current: &HashMap<&str, &EnumTypeDef>,
    desired: &HashMap<&str, &EnumTypeDef>,
    changes: &mut Vec<SchemaChange>,
) {
    // New enums
    for (name, def) in desired {
        if !current.contains_key(name) {
            changes.push(SchemaChange::CreateEnum((*def).clone()));
        }
    }

    // Modified enums — can only add values (PostgreSQL limitation)
    for (name, desired_def) in desired {
        if let Some(current_def) = current.get(name) {
            let current_variants: HashSet<&str> =
                current_def.variants.iter().map(|v| v.as_str()).collect();
            for variant in &desired_def.variants {
                if !current_variants.contains(variant.as_str()) {
                    changes.push(SchemaChange::AddEnumValue {
                        enum_name: name.to_string(),
                        value: variant.clone(),
                    });
                }
            }
        }
    }

    // Dropped enums
    for name in current.keys() {
        if !desired.contains_key(name) {
            changes.push(SchemaChange::DropEnum {
                name: name.to_string(),
            });
        }
    }
}

/// Diff two versions of the same table.
fn diff_table(current: &TableDef, desired: &TableDef, changes: &mut Vec<SchemaChange>) {
    let table = &desired.name;

    let current_cols: HashMap<&str, &ColumnDef> = current
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();
    let desired_cols: HashMap<&str, &ColumnDef> = desired
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();

    // --- Column changes ---

    // First pass: detect renames by position + type match
    let mut renamed_from: HashSet<&str> = HashSet::new();
    let mut renamed_to: HashSet<&str> = HashSet::new();

    for (pos, desired_col) in desired.columns.iter().enumerate() {
        // If this column doesn't exist in current by name
        if !current_cols.contains_key(desired_col.name.as_str()) {
            // Check if there's a column at the same position in current
            // that also doesn't exist in desired by name, with same type
            if pos < current.columns.len() {
                let cur_at_pos = &current.columns[pos];
                if !desired_cols.contains_key(cur_at_pos.name.as_str())
                    && cur_at_pos.sql_type == desired_col.sql_type
                    && !renamed_from.contains(cur_at_pos.name.as_str())
                {
                    // Same position + same type + both unmatched = rename
                    changes.push(SchemaChange::RenameColumn {
                        table: table.clone(),
                        from: cur_at_pos.name.clone(),
                        to: desired_col.name.clone(),
                    });
                    renamed_from.insert(cur_at_pos.name.as_str());
                    renamed_to.insert(desired_col.name.as_str());
                }
            }
        }
    }

    // New columns (excluding detected renames)
    for desired_col in &desired.columns {
        if !current_cols.contains_key(desired_col.name.as_str())
            && !renamed_to.contains(desired_col.name.as_str())
        {
            changes.push(SchemaChange::AddColumn {
                table: table.clone(),
                column: desired_col.clone(),
            });
        }
    }

    // Dropped columns (excluding detected renames)
    for current_col in &current.columns {
        if !desired_cols.contains_key(current_col.name.as_str())
            && !renamed_from.contains(current_col.name.as_str())
        {
            changes.push(SchemaChange::DropColumn {
                table: table.clone(),
                column: current_col.name.clone(),
            });
        }
    }

    // Modified columns
    for desired_col in &desired.columns {
        if let Some(current_col) = current_cols.get(desired_col.name.as_str()) {
            diff_column(table, current_col, desired_col, changes);
        }
    }

    // --- Primary key changes ---
    diff_primary_key(table, &current.primary_key, &desired.primary_key, changes);

    // --- Unique constraint changes ---
    diff_unique_constraints(
        table,
        &current.unique_constraints,
        &desired.unique_constraints,
        changes,
    );

    // --- Foreign key changes ---
    diff_foreign_keys(table, &current.foreign_keys, &desired.foreign_keys, changes);

    // --- Index changes ---
    diff_indexes(table, &current.indexes, &desired.indexes, changes);

    // --- Check constraint changes ---
    diff_check_constraints(
        table,
        &current.check_constraints,
        &desired.check_constraints,
        changes,
    );
}

/// Diff two versions of the same column.
fn diff_column(
    table: &str,
    current: &ColumnDef,
    desired: &ColumnDef,
    changes: &mut Vec<SchemaChange>,
) {
    // Type change
    if current.sql_type != desired.sql_type {
        // Skip if current is Serial and desired is Integer with is_auto
        // (they represent the same thing)
        let is_serial_equiv = matches!(
            (&current.sql_type, &desired.sql_type),
            (SqlType::Serial, SqlType::Integer) | (SqlType::Integer, SqlType::Serial)
        ) && (current.is_auto || desired.is_auto);

        if !is_serial_equiv {
            changes.push(SchemaChange::AlterColumnType {
                table: table.to_string(),
                column: desired.name.clone(),
                from: current.sql_type.clone(),
                to: desired.sql_type.clone(),
            });
        }
    }

    // Nullable change
    if current.nullable != desired.nullable {
        if desired.nullable {
            changes.push(SchemaChange::DropNotNull {
                table: table.to_string(),
                column: desired.name.clone(),
            });
        } else {
            changes.push(SchemaChange::SetNotNull {
                table: table.to_string(),
                column: desired.name.clone(),
                default_value: desired.default.clone(),
                sql_type: desired.sql_type.clone(),
            });
        }
    }

    // Default value change
    // Compare by SQL representation (not Rust enum) to avoid spurious
    // diffs when PostgreSQL introspects e.g. `0` as Integer(0) while
    // the .doo source says `@default(0.0)` which is Float(0.0) — both
    // produce the same SQL `DEFAULT 0`.
    match (&current.default, &desired.default) {
        (None, Some(new_default)) => {
            changes.push(SchemaChange::SetDefault {
                table: table.to_string(),
                column: desired.name.clone(),
                default: new_default.clone(),
            });
        }
        (Some(_), None) => {
            // Only drop default if column is not auto-increment
            if !desired.is_auto {
                changes.push(SchemaChange::DropDefault {
                    table: table.to_string(),
                    column: desired.name.clone(),
                });
            }
        }
        (Some(old), Some(new)) => {
            if old.to_sql() != new.to_sql() {
                changes.push(SchemaChange::SetDefault {
                    table: table.to_string(),
                    column: desired.name.clone(),
                    default: new.clone(),
                });
            }
        }
        (None, None) => {}
    }
}

/// Diff primary keys.
fn diff_primary_key(
    table: &str,
    current: &Option<PrimaryKeyDef>,
    desired: &Option<PrimaryKeyDef>,
    changes: &mut Vec<SchemaChange>,
) {
    match (current, desired) {
        (None, Some(pk)) => {
            changes.push(SchemaChange::AddPrimaryKey {
                table: table.to_string(),
                name: pk.name.clone(),
                columns: pk.columns.clone(),
            });
        }
        (Some(pk), None) => {
            changes.push(SchemaChange::DropPrimaryKey {
                table: table.to_string(),
                name: pk.name.clone(),
            });
        }
        (Some(old), Some(new)) => {
            if old.columns != new.columns {
                changes.push(SchemaChange::DropPrimaryKey {
                    table: table.to_string(),
                    name: old.name.clone(),
                });
                changes.push(SchemaChange::AddPrimaryKey {
                    table: table.to_string(),
                    name: new.name.clone(),
                    columns: new.columns.clone(),
                });
            }
        }
        (None, None) => {}
    }
}

/// Diff unique constraints.
fn diff_unique_constraints(
    table: &str,
    current: &[UniqueConstraintDef],
    desired: &[UniqueConstraintDef],
    changes: &mut Vec<SchemaChange>,
) {
    let current_by_cols: HashMap<Vec<String>, &UniqueConstraintDef> = current
        .iter()
        .map(|u| {
            let mut cols = u.columns.clone();
            cols.sort();
            (cols, u)
        })
        .collect();

    let desired_by_cols: HashMap<Vec<String>, &UniqueConstraintDef> = desired
        .iter()
        .map(|u| {
            let mut cols = u.columns.clone();
            cols.sort();
            (cols, u)
        })
        .collect();

    for (cols, uq) in &desired_by_cols {
        if !current_by_cols.contains_key(cols) {
            changes.push(SchemaChange::AddUnique {
                table: table.to_string(),
                name: uq.name.clone(),
                columns: uq.columns.clone(),
            });
        }
    }

    for (cols, uq) in &current_by_cols {
        if !desired_by_cols.contains_key(cols) {
            changes.push(SchemaChange::DropUnique {
                table: table.to_string(),
                name: uq.name.clone(),
            });
        }
    }
}

/// Diff foreign keys.
fn diff_foreign_keys(
    table: &str,
    current: &[ForeignKeyDef],
    desired: &[ForeignKeyDef],
    changes: &mut Vec<SchemaChange>,
) {
    let current_by_cols: HashMap<(&str, Vec<String>), &ForeignKeyDef> = current
        .iter()
        .map(|fk| {
            let mut cols = fk.columns.clone();
            cols.sort();
            ((fk.ref_table.as_str(), cols), fk)
        })
        .collect();

    let desired_by_cols: HashMap<(&str, Vec<String>), &ForeignKeyDef> = desired
        .iter()
        .map(|fk| {
            let mut cols = fk.columns.clone();
            cols.sort();
            ((fk.ref_table.as_str(), cols), fk)
        })
        .collect();

    for (key, fk) in &desired_by_cols {
        if !current_by_cols.contains_key(key) {
            changes.push(SchemaChange::AddForeignKey {
                table: table.to_string(),
                fk: (*fk).clone(),
            });
        }
    }

    for (key, fk) in &current_by_cols {
        if !desired_by_cols.contains_key(key) {
            changes.push(SchemaChange::DropForeignKey {
                table: table.to_string(),
                name: fk.name.clone(),
            });
        }
    }
}

/// Diff indexes.
fn diff_indexes(
    table: &str,
    current: &[IndexDef],
    desired: &[IndexDef],
    changes: &mut Vec<SchemaChange>,
) {
    let current_by_cols: HashMap<Vec<String>, &IndexDef> = current
        .iter()
        .map(|idx| {
            let mut cols = idx.columns.clone();
            cols.sort();
            (cols, idx)
        })
        .collect();

    let desired_by_cols: HashMap<Vec<String>, &IndexDef> = desired
        .iter()
        .map(|idx| {
            let mut cols = idx.columns.clone();
            cols.sort();
            (cols, idx)
        })
        .collect();

    for (cols, idx) in &desired_by_cols {
        if !current_by_cols.contains_key(cols) {
            changes.push(SchemaChange::CreateIndex {
                table: table.to_string(),
                index: (*idx).clone(),
            });
        }
    }

    for (cols, idx) in &current_by_cols {
        if !desired_by_cols.contains_key(cols) {
            changes.push(SchemaChange::DropIndex {
                name: idx.name.clone(),
            });
        }
    }
}

/// Diff check constraints.
fn diff_check_constraints(
    table: &str,
    current: &[CheckConstraintDef],
    desired: &[CheckConstraintDef],
    changes: &mut Vec<SchemaChange>,
) {
    let current_by_name: HashMap<&str, &CheckConstraintDef> =
        current.iter().map(|c| (c.name.as_str(), c)).collect();
    let desired_by_name: HashMap<&str, &CheckConstraintDef> =
        desired.iter().map(|c| (c.name.as_str(), c)).collect();

    for (name, check) in &desired_by_name {
        if !current_by_name.contains_key(name) {
            changes.push(SchemaChange::AddCheck {
                table: table.to_string(),
                name: name.to_string(),
                expression: check.expression.clone(),
            });
        }
    }

    for (name, _) in &current_by_name {
        if !desired_by_name.contains_key(name) {
            changes.push(SchemaChange::DropCheck {
                table: table.to_string(),
                name: name.to_string(),
            });
        }
    }
}
