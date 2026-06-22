//! Schema Diff Engine
//!
//! Compares two `DatabaseSchema` instances (current vs desired) and produces
//! a list of `SchemaChange` operations representing the migration.

use std::collections::{HashMap, HashSet};

use crate::schema::*;
use serde::Serialize;

/// Returns the list of affected object keys for a SchemaChange.
/// This is the SINGLE SOURCE OF TRUTH for dependency graph computation.
/// Every SchemaChange variant MUST be covered here.
pub fn affected_objects_for(change: &SchemaChange) -> Vec<String> {
    use SchemaChange::*;
    match change {
        CreateEnum(e) => vec![e.name.clone()],
        RenameEnum { from, to } => vec![from.clone(), to.clone()],
        AddEnumValue { enum_name, .. } => vec![enum_name.clone()],
        DropEnum { name } => vec![name.clone()],

        CreateTable(t) => {
            let mut objects = vec![t.name.clone()];
            // Include FK referenced tables so the dependency graph
            // knows this table depends on those tables existing first.
            for fk in &t.foreign_keys {
                if !objects.contains(&fk.ref_table) {
                    objects.push(fk.ref_table.clone());
                }
            }
            // Include enum types used by columns so the dependency graph
            // knows this table depends on those enums existing first.
            for col in &t.columns {
                if let SqlType::Enum(enum_name) = &col.sql_type {
                    if !objects.contains(enum_name) {
                        objects.push(enum_name.clone());
                    }
                }
            }
            // Include transitive enum refs from non-table struct fields.
            // These are enums found through struct references (e.g., a field
            // of type Project where Project has a field of enum Category).
            for enum_name in &t.transitive_enum_refs {
                if !objects.contains(enum_name) {
                    objects.push(enum_name.clone());
                }
            }
            // Include struct_refs names that look like table names
            // (from @foreign decorators on non-table struct fields).
            // Pure non-table struct names (no matching creator) are harmless
            // — they simply won't match any creator in the dependency graph.
            for sr in &t.struct_refs {
                if !objects.contains(sr) {
                    objects.push(sr.clone());
                }
            }
            objects
        }
        DropTable { name } => vec![name.clone()],
        RenameTable { from, to } => vec![from.clone(), to.clone()],

        AddColumn { table, column } => {
            let mut objects = vec![table.clone(), format!("{}.{}", table, column.name)];
            // If column type is an enum, include the enum name so deps chain correctly
            if let SqlType::Enum(enum_name) = &column.sql_type {
                objects.push(enum_name.clone());
            }
            objects
        }
        DropColumn { table, column } => vec![table.clone(), format!("{}.{}", table, column)],
        RenameColumn { table, from, to } => vec![
            table.clone(),
            format!("{}.{}", table, from),
            format!("{}.{}", table, to),
        ],
        AlterColumnType {
            table, column, to, ..
        } => {
            let mut objects = vec![table.clone(), format!("{}.{}", table, column)];
            // If the target type is an enum, include the enum type name
            // so the dependency graph connects type changes to enum creation/dropping.
            if let SqlType::Enum(enum_name) = to {
                objects.push(enum_name.clone());
            }
            objects
        }
        SetNotNull { table, column, .. } => vec![table.clone(), format!("{}.{}", table, column)],
        DropNotNull { table, column } => vec![table.clone(), format!("{}.{}", table, column)],
        SetDefault { table, column, .. } => vec![table.clone(), format!("{}.{}", table, column)],
        DropDefault { table, column } => vec![table.clone(), format!("{}.{}", table, column)],

        AddPrimaryKey { table, .. } => vec![table.clone()],
        DropPrimaryKey { table, .. } => vec![table.clone()],
        AddUnique { table, .. } => vec![table.clone()],
        DropUnique { table, .. } => vec![table.clone()],
        AddCheck { table, .. } => vec![table.clone()],
        DropCheck { table, .. } => vec![table.clone()],

        CreateIndex { table, .. } => vec![table.clone()],
        DropIndex { table, name } => vec![table.clone(), name.clone()],

        AddForeignKey { table, fk } => vec![
            table.clone(),
            fk.ref_table.clone(),
            format!("{}.fk.{}", table, fk.name),
        ],
        DropForeignKey { table, name } => vec![table.clone(), format!("{}.fk.{}", table, name)],
        ModifyForeignKey {
            table,
            fk,
            previous,
            ..
        } => {
            let mut objects = vec![
                table.clone(),
                fk.ref_table.clone(),
                format!("{}.fk.{}", table, fk.name),
            ];
            // Include previous ref_table too in case it changed
            if previous.ref_table != fk.ref_table {
                objects.push(previous.ref_table.clone());
            }
            objects
        }
    }
}

// ============================================================================
// Schema Change Types
// ============================================================================

/// A single schema change operation.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SchemaChange {
    // --- Enum Types ---
    CreateEnum(EnumTypeDef),
    RenameEnum {
        from: String,
        to: String,
    },
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
        table: String,
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
    /// Modify an existing foreign key — column, target table, or actions changed.
    /// The from/to fields capture what changed so the UI can show a clear diff.
    ModifyForeignKey {
        table: String,
        /// Name of the FK constraint (may stay the same or change).
        constraint_name: String,
        fk: ForeignKeyDef,
        /// Previous FK definition (for rollback).
        previous: ForeignKeyDef,
        /// What changed: "target", "columns", "on_delete", "on_update", "multi"
        change_kind: String,
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

    // 2. Dropped tables (in current DB but NOT in desired schema, excluding renames).
    // These are tables that were previously managed by doo_migrate but have been
    // removed from the project's .doo source files. In DooCloud, each project has
    // its own database, so all user tables are owned by the current project.
    for current_table in &current.tables {
        if !desired_tables.contains_key(current_table.name.as_str())
            && !renamed_tables.contains(current_table.name.as_str())
        {
            // Skip tables that look like system/internal tables
            // (pg_*, sql_*, information_schema, and any crate::SYSTEM_TABLES)
            let lower = current_table.name.to_lowercase();
            if lower.starts_with("pg_")
                || lower.starts_with("sql_")
                || crate::is_system_table(&lower)
            {
                continue;
            }
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
    // Count how many column names match (by name, order-independent).
    // A genuine table rename keeps the same columns — just the table name changes.
    // Require ALL column names to match to avoid false rename detection when
    // a completely different new table happens to have the same column types.
    let desired_names: std::collections::HashSet<&str> =
        desired.columns.iter().map(|c| c.name.as_str()).collect();
    let current_names: std::collections::HashSet<&str> =
        current.columns.iter().map(|c| c.name.as_str()).collect();
    let matching_names = desired_names.intersection(&current_names).count();
    let total_names = desired_names.len().max(current_names.len());
    // Require ALL column names to match for a rename.
    // If ANY column name differs, this is a different table — not a rename.
    if matching_names != total_names {
        return false;
    }
    for (cur_col, des_col) in current.columns.iter().zip(desired.columns.iter()) {
        // Column names must also match by position (same order)
        if cur_col.name != des_col.name {
            return false;
        }
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
    // Track which enums have been matched (rename or direct match) to avoid
    // double-processing them.
    let mut matched_desired: HashSet<&str> = HashSet::new();
    let mut matched_current: HashSet<&str> = HashSet::new();

    // --- Rename detection: match unmatched desired enums with unmatched current enums ---
    // An enum rename is detected when:
    // 1. The desired name does NOT exist in current DB
    // 2. The current name does NOT exist in desired schema
    // 3. Both have the same variants (same values, same order)
    for (desired_name, desired_def) in desired {
        if current.contains_key(desired_name) {
            continue; // Already exists — handled below as modified
        }
        // Find a current enum NOT in desired that has identical variants
        for (current_name, current_def) in current {
            if desired.contains_key(current_name) {
                continue; // Still exists — not a rename source
            }
            if matched_current.contains(current_name) {
                continue; // Already matched to another rename
            }
            if enum_variants_match(current_def, desired_def) {
                changes.push(SchemaChange::RenameEnum {
                    from: current_name.to_string(),
                    to: desired_name.to_string(),
                });
                matched_desired.insert(desired_name);
                matched_current.insert(current_name);
                break;
            }
        }
    }

    // --- New enums (in desired but not current, excluding renames) ---
    for (name, def) in desired {
        if !current.contains_key(name) && !matched_desired.contains(name) {
            changes.push(SchemaChange::CreateEnum((*def).clone()));
        }
    }

    // --- Modified enums — can only add values (PostgreSQL limitation) ---
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

    // Enums in the database but NOT in our desired schema: SKIP.
    // These are foreign enums from other projects sharing the database,
    // OR enums that were renamed (handled above via RenameEnum).
    // We intentionally do NOT generate DropEnum for them.
}

/// Check if two enum type definitions have identical variants (same values, same order).
/// Used for rename detection — if variants match exactly, it's likely a rename.
fn enum_variants_match(a: &EnumTypeDef, b: &EnumTypeDef) -> bool {
    a.variants.len() == b.variants.len()
        && a.variants
            .iter()
            .zip(b.variants.iter())
            .all(|(va, vb)| va == vb)
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

        // Skip if both are Enum types and a RenameEnum covers this change.
        // When an enum is renamed, columns referencing it show a type change
        // (Enum("old") → Enum("new")), but PostgreSQL handles this automatically
        // via ALTER TYPE RENAME — no ALTER COLUMN needed.
        let is_enum_rename = match (&current.sql_type, &desired.sql_type) {
            (SqlType::Enum(cur_enum), SqlType::Enum(des_enum)) => changes.iter().any(|c| {
                matches!(c, SchemaChange::RenameEnum { from, to }
                    if from == cur_enum && to == des_enum)
            }),
            _ => false,
        };

        if !is_serial_equiv && !is_enum_rename {
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

/// Diff foreign keys — detects adds, drops, and modifications.
///
/// Modifications include:
/// - Target table changed (ref_table)
/// - Referenced columns changed (ref_columns)
/// - ON DELETE action changed
/// - ON UPDATE action changed
/// - Local columns changed (columns)
fn diff_foreign_keys(
    table: &str,
    current: &[ForeignKeyDef],
    desired: &[ForeignKeyDef],
    changes: &mut Vec<SchemaChange>,
) {
    // Match by name when possible (stable identity), fall back to (ref_table, columns) key.
    // Name-based matching is more robust for detecting modifications because
    // the user may change ref_table or columns but keep the same FK name.
    let current_by_name: HashMap<&str, &ForeignKeyDef> =
        current.iter().map(|fk| (fk.name.as_str(), fk)).collect();
    let desired_by_name: HashMap<&str, &ForeignKeyDef> =
        desired.iter().map(|fk| (fk.name.as_str(), fk)).collect();

    // Track which current FKs have been matched (by name or by key)
    let mut matched_current: HashSet<&str> = HashSet::new();
    let mut matched_desired: HashSet<&str> = HashSet::new();

    // Phase 1: Match by name — detect modifications
    for (name, desired_fk) in &desired_by_name {
        if let Some(current_fk) = current_by_name.get(name) {
            matched_current.insert(name);
            matched_desired.insert(name);

            // Check what changed
            let mut changed_parts: Vec<&str> = Vec::new();

            if current_fk.ref_table != desired_fk.ref_table {
                changed_parts.push("target");
            }
            if current_fk.columns != desired_fk.columns {
                changed_parts.push("columns");
            }
            if current_fk.ref_columns != desired_fk.ref_columns {
                changed_parts.push("ref_columns");
            }
            if current_fk.on_delete != desired_fk.on_delete {
                changed_parts.push("on_delete");
            }
            if current_fk.on_update != desired_fk.on_update {
                changed_parts.push("on_update");
            }

            if !changed_parts.is_empty() {
                let change_kind = if changed_parts.len() > 1 {
                    "multi".to_string()
                } else {
                    changed_parts[0].to_string()
                };

                changes.push(SchemaChange::ModifyForeignKey {
                    table: table.to_string(),
                    constraint_name: name.to_string(),
                    fk: (*desired_fk).clone(),
                    previous: (*current_fk).clone(),
                    change_kind,
                });
            }
        }
    }

    // Phase 2: Match unmatched by (ref_table, columns) key — fallback for unnamed FKs
    // Build key-based maps for unmatched FKs only
    let current_by_key: HashMap<(&str, Vec<String>), &ForeignKeyDef> = current
        .iter()
        .filter(|fk| !matched_current.contains(fk.name.as_str()))
        .map(|fk| {
            let mut cols = fk.columns.clone();
            cols.sort();
            ((fk.ref_table.as_str(), cols), fk)
        })
        .collect();

    let desired_by_key: HashMap<(&str, Vec<String>), &ForeignKeyDef> = desired
        .iter()
        .filter(|fk| !matched_desired.contains(fk.name.as_str()))
        .map(|fk| {
            let mut cols = fk.columns.clone();
            cols.sort();
            ((fk.ref_table.as_str(), cols), fk)
        })
        .collect();

    // New FKs (in desired but not in current by key)
    for (key, fk) in &desired_by_key {
        if !current_by_key.contains_key(key) {
            // Check if there's a current FK with different ref_table but same columns
            // (target table change detected via key mismatch)
            let current_same_cols: Vec<&ForeignKeyDef> = current
                .iter()
                .filter(|cfk| {
                    !matched_current.contains(cfk.name.as_str()) && {
                        let mut c_cols = cfk.columns.clone();
                        c_cols.sort();
                        c_cols == key.1
                    }
                })
                .collect();

            if let Some(current_fk) = current_same_cols.first() {
                // Same columns, different ref_table → modification (target changed)
                matched_current.insert(current_fk.name.as_str());
                matched_desired.insert(fk.name.as_str());

                let mut changed_parts = vec!["target"];
                if current_fk.on_delete != fk.on_delete {
                    changed_parts.push("on_delete");
                }
                if current_fk.on_update != fk.on_update {
                    changed_parts.push("on_update");
                }
                let change_kind = if changed_parts.len() > 1 {
                    "multi".to_string()
                } else {
                    changed_parts[0].to_string()
                };

                changes.push(SchemaChange::ModifyForeignKey {
                    table: table.to_string(),
                    constraint_name: fk.name.clone(),
                    fk: (*fk).clone(),
                    previous: (*current_fk).clone(),
                    change_kind,
                });
            } else {
                changes.push(SchemaChange::AddForeignKey {
                    table: table.to_string(),
                    fk: (*fk).clone(),
                });
            }
        }
    }

    // Dropped FKs (in current but not in desired by key, and not matched)
    for (key, fk) in &current_by_key {
        if !desired_by_key.contains_key(key) {
            // Check if columns still exist but ref_table changed → already handled above
            let desired_same_cols: Vec<&ForeignKeyDef> = desired
                .iter()
                .filter(|dfk| {
                    !matched_desired.contains(dfk.name.as_str()) && {
                        let mut d_cols = dfk.columns.clone();
                        d_cols.sort();
                        d_cols == key.1
                    }
                })
                .collect();

            if desired_same_cols.is_empty() {
                changes.push(SchemaChange::DropForeignKey {
                    table: table.to_string(),
                    name: fk.name.clone(),
                });
            }
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
                table: table.to_string(),
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
