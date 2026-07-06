//! SQL Generator — SchemaChange → PostgreSQL DDL
//!
//! Generates forward (up) and rollback (down) SQL for each schema change.
//! PostgreSQL-specific. All DDL is generated here — single source of truth.

use crate::diff::SchemaChange;
use crate::plan::MigrationPlan;
use crate::schema::*;

// ============================================================================
// Identifier Quoting — Single Source of Truth
// ============================================================================

/// Quote a PostgreSQL identifier (table, column, constraint, index, enum name).
///
/// Wraps the identifier in double quotes and escapes any embedded double quotes
/// by doubling them. This protects against SQL reserved keywords (e.g. `as`,
/// `user`, `order`, `group`) and special characters in identifiers.
///
/// PostgreSQL quoted identifiers are case-sensitive; unquoted identifiers are
/// folded to lowercase. Since Doo always generates lowercase snake_case names,
/// quoting preserves the lowercase form correctly.
fn quote_ident(ident: &str) -> String {
    let escaped = ident.replace('"', "\"\"");
    format!("\"{}\"", escaped)
}

/// Quote a list of identifiers and join them with a separator.
fn quote_idents(items: &[String], sep: &str) -> String {
    items
        .iter()
        .map(|s| quote_ident(s))
        .collect::<Vec<_>>()
        .join(sep)
}

/// Generate combined UP SQL for the entire migration plan.
pub fn generate_up_sql(plan: &MigrationPlan) -> String {
    plan.changes
        .iter()
        .map(|p| p.up_sql.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate combined DOWN SQL for the entire migration plan (reversed order).
pub fn generate_down_sql(plan: &MigrationPlan) -> String {
    plan.changes
        .iter()
        .rev()
        .filter_map(|p| p.down_sql.as_deref())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate UP SQL for a single schema change.
pub fn change_to_up_sql(change: &SchemaChange) -> String {
    match change {
        // --- Enums ---
        SchemaChange::CreateEnum(e) => {
            let variants = e
                .variants
                .iter()
                .map(|v| format!("'{}'", v))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "CREATE TYPE {} AS ENUM ({});",
                quote_ident(&e.name),
                variants
            )
        }
        SchemaChange::RenameEnum { from, to } => {
            format!(
                "ALTER TYPE {} RENAME TO {};",
                quote_ident(from),
                quote_ident(to)
            )
        }
        SchemaChange::AddEnumValue { enum_name, value } => {
            format!(
                "ALTER TYPE {} ADD VALUE IF NOT EXISTS '{}';",
                quote_ident(enum_name),
                value
            )
        }
        SchemaChange::DropEnum { name } => {
            format!("DROP TYPE IF EXISTS {};", quote_ident(name))
        }

        // --- Tables ---
        SchemaChange::CreateTable(t) => generate_create_table(t),
        SchemaChange::DropTable { name, .. } => {
            format!("DROP TABLE IF EXISTS {} CASCADE;", quote_ident(name))
        }
        SchemaChange::RenameTable { from, to } => {
            format!(
                "ALTER TABLE {} RENAME TO {};",
                quote_ident(from),
                quote_ident(to)
            )
        }

        // --- Columns ---
        SchemaChange::AddColumn { table, column } => {
            let col_type = column.sql_type.to_ddl();
            let q_table = quote_ident(table);
            let q_col = quote_ident(&column.name);
            let mut sql = format!("ALTER TABLE {} ADD COLUMN {} {}", q_table, q_col, col_type);

            // If column should be NOT NULL:
            // - With a default: add NOT NULL inline (safe, existing rows get the default)
            // - Without a default: add as nullable first, then backfill with zero value
            //   and set NOT NULL in a single self-contained statement pair.
            //   This avoids silently creating a nullable column when the schema says NOT NULL.
            if !column.nullable {
                if column.default.is_some() {
                    sql.push_str(" NOT NULL");
                }
                // No default → will add NOT NULL via backfill below
            }
            if let Some(default) = &column.default {
                sql.push_str(&format!(" DEFAULT {}", default.to_sql()));
            }
            sql.push(';');

            // If NOT NULL without a default, add the backfill + SET NOT NULL now.
            // This guarantees the column ends up NOT NULL even for empty tables,
            // and does NOT rely on a separate SetNotNull change (which may be
            // ordered incorrectly or silently skipped by the planner).
            if !column.nullable && column.default.is_none() {
                let zero = column.sql_type.zero_default().to_sql();
                sql.push_str(&format!(
                    "\nUPDATE {} SET {} = {} WHERE {} IS NULL;\nALTER TABLE {} ALTER COLUMN {} SET NOT NULL;",
                    q_table, q_col, zero, q_col, q_table, q_col
                ));
            }
            sql
        }
        SchemaChange::DropColumn { table, column, .. } => {
            format!(
                "ALTER TABLE {} DROP COLUMN IF EXISTS {} CASCADE;",
                quote_ident(table),
                quote_ident(column)
            )
        }
        SchemaChange::RenameColumn { table, from, to } => {
            format!(
                "ALTER TABLE {} RENAME COLUMN {} TO {};",
                quote_ident(table),
                quote_ident(from),
                quote_ident(to)
            )
        }
        SchemaChange::AlterColumnType {
            table, column, to, ..
        } => {
            let q_table = quote_ident(table);
            let q_col = quote_ident(column);
            format!(
                "ALTER TABLE {} ALTER COLUMN {} TYPE {} USING {}::{};",
                q_table,
                q_col,
                to.to_ddl(),
                q_col,
                to.to_ddl()
            )
        }
        SchemaChange::SetNotNull {
            table,
            column,
            default_value,
            sql_type,
        } => {
            // Determine the value to backfill NULL rows with.
            // If the desired schema specifies a default, use that.
            // Otherwise, derive a type-appropriate "zero" value.
            let backfill_value = match default_value {
                Some(dv) => dv.to_sql(),
                None => sql_type.zero_default().to_sql(),
            };
            let q_table = quote_ident(table);
            let q_col = quote_ident(column);
            format!(
                "UPDATE {} SET {} = {} WHERE {} IS NULL;\nALTER TABLE {} ALTER COLUMN {} SET NOT NULL;",
                q_table, q_col, backfill_value, q_col, q_table, q_col
            )
        }
        SchemaChange::DropNotNull { table, column } => {
            format!(
                "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL;",
                quote_ident(table),
                quote_ident(column)
            )
        }
        SchemaChange::SetDefault {
            table,
            column,
            default,
        } => {
            format!(
                "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {};",
                quote_ident(table),
                quote_ident(column),
                default.to_sql()
            )
        }
        SchemaChange::DropDefault { table, column } => {
            format!(
                "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT;",
                quote_ident(table),
                quote_ident(column)
            )
        }

        // --- Constraints ---
        SchemaChange::AddPrimaryKey {
            table,
            name,
            columns,
        } => {
            format!(
                "ALTER TABLE {} ADD CONSTRAINT {} PRIMARY KEY ({});",
                quote_ident(table),
                quote_ident(name),
                quote_idents(columns, ", ")
            )
        }
        SchemaChange::DropPrimaryKey { table, name } => {
            format!(
                "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
                quote_ident(table),
                quote_ident(name)
            )
        }
        SchemaChange::AddUnique {
            table,
            name,
            columns,
        } => {
            format!(
                "ALTER TABLE {} ADD CONSTRAINT {} UNIQUE ({});",
                quote_ident(table),
                quote_ident(name),
                quote_idents(columns, ", ")
            )
        }
        SchemaChange::DropUnique { table, name } => {
            format!(
                "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
                quote_ident(table),
                quote_ident(name)
            )
        }
        SchemaChange::AddCheck {
            table,
            name,
            expression,
        } => {
            format!(
                "ALTER TABLE {} ADD CONSTRAINT {} CHECK ({});",
                quote_ident(table),
                quote_ident(name),
                expression
            )
        }
        SchemaChange::DropCheck { table, name } => {
            format!(
                "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
                quote_ident(table),
                quote_ident(name)
            )
        }

        // --- Indexes ---
        SchemaChange::CreateIndex { table, index } => {
            let unique = if index.unique { "UNIQUE " } else { "" };
            format!(
                "CREATE {}INDEX IF NOT EXISTS {} ON {} ({});",
                unique,
                quote_ident(&index.name),
                quote_ident(table),
                quote_idents(&index.columns, ", ")
            )
        }
        SchemaChange::DropIndex { name, .. } => {
            format!("DROP INDEX IF EXISTS {};", quote_ident(name))
        }

        // --- Foreign Keys ---
        SchemaChange::AddForeignKey { table, fk } => {
            format!(
                "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {} ON UPDATE {};",
                quote_ident(table),
                quote_ident(&fk.name),
                quote_idents(&fk.columns, ", "),
                quote_ident(&fk.ref_table),
                quote_idents(&fk.ref_columns, ", "),
                fk.on_delete.to_sql(),
                fk.on_update.to_sql()
            )
        }
        SchemaChange::DropForeignKey { table, name, .. } => {
            format!(
                "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
                quote_ident(table),
                quote_ident(name)
            )
        }
        SchemaChange::ModifyForeignKey {
            table,
            constraint_name,
            fk,
            previous,
            change_kind,
        } => {
            // Safe approach: DROP old constraint then ADD new one.
            // PostgreSQL doesn't have ALTER CONSTRAINT for FKs, so drop+add is the standard way.
            // Include a comment about what changed for audit trail.
            let q_table = quote_ident(table);
            let drop_sql = format!(
                "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
                q_table,
                quote_ident(&previous.name)
            );
            let add_sql = format!(
                "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {} ON UPDATE {};",
                q_table,
                quote_ident(&fk.name),
                quote_idents(&fk.columns, ", "),
                quote_ident(&fk.ref_table),
                quote_idents(&fk.ref_columns, ", "),
                fk.on_delete.to_sql(),
                fk.on_update.to_sql()
            );
            format!(
                "-- Modify FK {} (changed: {})\n{}",
                quote_ident(constraint_name),
                change_kind,
                [drop_sql.as_str(), add_sql.as_str()].join("\n")
            )
        }
    }
}

/// Generate DOWN (rollback) SQL for a single schema change.
/// Returns None for irreversible changes.
pub fn change_to_down_sql(change: &SchemaChange) -> Option<String> {
    match change {
        SchemaChange::CreateEnum(e) => {
            Some(format!("DROP TYPE IF EXISTS {};", quote_ident(&e.name)))
        }
        SchemaChange::RenameEnum { from, to } => Some(format!(
            "ALTER TYPE {} RENAME TO {};",
            quote_ident(to),
            quote_ident(from)
        )),
        SchemaChange::AddEnumValue { .. } => {
            // PostgreSQL cannot remove enum values — irreversible
            None
        }
        SchemaChange::DropEnum { .. } => {
            // Can't recreate without knowing variants
            None
        }

        SchemaChange::CreateTable(t) => Some(format!(
            "DROP TABLE IF EXISTS {} CASCADE;",
            quote_ident(&t.name)
        )),
        SchemaChange::DropTable { name: _, previous } => {
            // Recreate the table from the full previous definition
            Some(generate_create_table(previous))
        }
        SchemaChange::RenameTable { from, to } => Some(format!(
            "ALTER TABLE {} RENAME TO {};",
            quote_ident(to),
            quote_ident(from)
        )),

        SchemaChange::AddColumn { table, column } => Some(format!(
            "ALTER TABLE {} DROP COLUMN IF EXISTS {};",
            quote_ident(table),
            quote_ident(&column.name)
        )),
        SchemaChange::DropColumn { .. } => {
            // Can't recreate dropped column with data
            None
        }
        SchemaChange::RenameColumn { table, from, to } => Some(format!(
            "ALTER TABLE {} RENAME COLUMN {} TO {};",
            quote_ident(table),
            quote_ident(to),
            quote_ident(from)
        )),
        SchemaChange::AlterColumnType {
            table,
            column,
            from,
            ..
        } => {
            let q_table = quote_ident(table);
            let q_col = quote_ident(column);
            Some(format!(
                "ALTER TABLE {} ALTER COLUMN {} TYPE {} USING {}::{};",
                q_table,
                q_col,
                from.to_ddl(),
                q_col,
                from.to_ddl()
            ))
        }
        SchemaChange::SetNotNull { table, column, .. } => Some(format!(
            "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL;",
            quote_ident(table),
            quote_ident(column)
        )),
        SchemaChange::DropNotNull { table, column } => Some(format!(
            "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL;",
            quote_ident(table),
            quote_ident(column)
        )),
        SchemaChange::SetDefault { table, column, .. } => Some(format!(
            "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT;",
            quote_ident(table),
            quote_ident(column)
        )),
        SchemaChange::DropDefault { .. } => {
            // Can't restore unknown default
            None
        }
        SchemaChange::AddPrimaryKey { table, name, .. } => Some(format!(
            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
            quote_ident(table),
            quote_ident(name)
        )),
        SchemaChange::DropPrimaryKey { .. } => None,
        SchemaChange::AddUnique { table, name, .. } => Some(format!(
            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
            quote_ident(table),
            quote_ident(name)
        )),
        SchemaChange::DropUnique { .. } => None,
        SchemaChange::AddCheck { table, name, .. } => Some(format!(
            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
            quote_ident(table),
            quote_ident(name)
        )),
        SchemaChange::DropCheck { .. } => None,
        SchemaChange::CreateIndex { index, .. } => Some(format!(
            "DROP INDEX IF EXISTS {};",
            quote_ident(&index.name)
        )),
        SchemaChange::DropIndex { .. } => None,
        SchemaChange::AddForeignKey { table, fk } => Some(format!(
            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
            quote_ident(table),
            quote_ident(&fk.name)
        )),
        SchemaChange::DropForeignKey { .. } => None,
        SchemaChange::ModifyForeignKey {
            table,
            fk,
            previous,
            ..
        } => {
            // Rollback: DROP the new constraint, re-ADD the old one
            let q_table = quote_ident(table);
            Some(format!(
                "-- Rollback FK modification\nALTER TABLE {} DROP CONSTRAINT IF EXISTS {};\nALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {} ON UPDATE {};",
                q_table, quote_ident(&fk.name),
                q_table,
                quote_ident(&previous.name),
                quote_idents(&previous.columns, ", "),
                quote_ident(&previous.ref_table),
                quote_idents(&previous.ref_columns, ", "),
                previous.on_delete.to_sql(),
                previous.on_update.to_sql()
            ))
        }
    }
}

/// Generate CREATE TABLE SQL for a complete table definition.
fn generate_create_table(table: &TableDef) -> String {
    let mut parts = Vec::new();

    for col in &table.columns {
        let q_col = quote_ident(&col.name);
        let mut col_sql = format!("  {} {}", q_col, col.sql_type.to_ddl());

        if col.is_auto {
            // Use GENERATED ALWAYS AS IDENTITY for modern PostgreSQL
            col_sql = format!("  {} INTEGER GENERATED ALWAYS AS IDENTITY", q_col);
        }

        if !col.nullable && !col.is_auto {
            col_sql.push_str(" NOT NULL");
        }

        if let Some(default) = &col.default {
            if !col.is_auto {
                col_sql.push_str(&format!(" DEFAULT {}", default.to_sql()));
            }
        }

        parts.push(col_sql);
    }

    // Primary key
    if let Some(pk) = &table.primary_key {
        parts.push(format!(
            "  CONSTRAINT {} PRIMARY KEY ({})",
            quote_ident(&pk.name),
            quote_idents(&pk.columns, ", ")
        ));
    }

    // Unique constraints
    for uq in &table.unique_constraints {
        parts.push(format!(
            "  CONSTRAINT {} UNIQUE ({})",
            quote_ident(&uq.name),
            quote_idents(&uq.columns, ", ")
        ));
    }

    // Check constraints
    for check in &table.check_constraints {
        parts.push(format!(
            "  CONSTRAINT {} CHECK ({})",
            quote_ident(&check.name),
            check.expression
        ));
    }

    // Foreign keys
    for fk in &table.foreign_keys {
        parts.push(format!(
            "  CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {} ON UPDATE {}",
            quote_ident(&fk.name),
            quote_idents(&fk.columns, ", "),
            quote_ident(&fk.ref_table),
            quote_idents(&fk.ref_columns, ", "),
            fk.on_delete.to_sql(),
            fk.on_update.to_sql()
        ));
    }

    let mut sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (\n{}\n);",
        quote_ident(&table.name),
        parts.join(",\n")
    );

    // Indexes (separate statements)
    for idx in &table.indexes {
        let unique = if idx.unique { "UNIQUE " } else { "" };
        sql.push_str(&format!(
            "\nCREATE {}INDEX IF NOT EXISTS {} ON {} ({});",
            unique,
            quote_ident(&idx.name),
            quote_ident(&table.name),
            quote_idents(&idx.columns, ", ")
        ));
    }

    sql
}
