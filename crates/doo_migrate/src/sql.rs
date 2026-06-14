//! SQL Generator — SchemaChange → PostgreSQL DDL
//!
//! Generates forward (up) and rollback (down) SQL for each schema change.
//! PostgreSQL-specific. All DDL is generated here — single source of truth.

use crate::diff::SchemaChange;
use crate::plan::MigrationPlan;
use crate::schema::*;

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
            format!("CREATE TYPE {} AS ENUM ({});", e.name, variants)
        }
        SchemaChange::AddEnumValue { enum_name, value } => {
            format!(
                "ALTER TYPE {} ADD VALUE IF NOT EXISTS '{}';",
                enum_name, value
            )
        }
        SchemaChange::DropEnum { name } => {
            format!("DROP TYPE IF EXISTS {};", name)
        }

        // --- Tables ---
        SchemaChange::CreateTable(t) => generate_create_table(t),
        SchemaChange::DropTable { name } => {
            format!("DROP TABLE IF EXISTS {} CASCADE;", name)
        }
        SchemaChange::RenameTable { from, to } => {
            format!("ALTER TABLE {} RENAME TO {};", from, to)
        }

        // --- Columns ---
        SchemaChange::AddColumn { table, column } => {
            let col_type = column.sql_type.to_ddl();
            let mut sql = format!(
                "ALTER TABLE {} ADD COLUMN {} {}",
                table, column.name, col_type
            );

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
                    table, column.name, zero, column.name, table, column.name
                ));
            }
            sql
        }
        SchemaChange::DropColumn { table, column } => {
            format!(
                "ALTER TABLE {} DROP COLUMN IF EXISTS {} CASCADE;",
                table, column
            )
        }
        SchemaChange::RenameColumn { table, from, to } => {
            format!("ALTER TABLE {} RENAME COLUMN {} TO {};", table, from, to)
        }
        SchemaChange::AlterColumnType {
            table, column, to, ..
        } => {
            format!(
                "ALTER TABLE {} ALTER COLUMN {} TYPE {} USING {}::{};",
                table,
                column,
                to.to_ddl(),
                column,
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
            format!(
                "UPDATE {} SET {} = {} WHERE {} IS NULL;\nALTER TABLE {} ALTER COLUMN {} SET NOT NULL;",
                table, column, backfill_value, column, table, column
            )
        }
        SchemaChange::DropNotNull { table, column } => {
            format!(
                "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL;",
                table, column
            )
        }
        SchemaChange::SetDefault {
            table,
            column,
            default,
        } => {
            format!(
                "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {};",
                table,
                column,
                default.to_sql()
            )
        }
        SchemaChange::DropDefault { table, column } => {
            format!(
                "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT;",
                table, column
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
                table,
                name,
                columns.join(", ")
            )
        }
        SchemaChange::DropPrimaryKey { table, name } => {
            format!("ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};", table, name)
        }
        SchemaChange::AddUnique {
            table,
            name,
            columns,
        } => {
            format!(
                "ALTER TABLE {} ADD CONSTRAINT {} UNIQUE ({});",
                table,
                name,
                columns.join(", ")
            )
        }
        SchemaChange::DropUnique { table, name } => {
            format!("ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};", table, name)
        }
        SchemaChange::AddCheck {
            table,
            name,
            expression,
        } => {
            format!(
                "ALTER TABLE {} ADD CONSTRAINT {} CHECK ({});",
                table, name, expression
            )
        }
        SchemaChange::DropCheck { table, name } => {
            format!("ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};", table, name)
        }

        // --- Indexes ---
        SchemaChange::CreateIndex { table, index } => {
            let unique = if index.unique { "UNIQUE " } else { "" };
            format!(
                "CREATE {}INDEX IF NOT EXISTS {} ON {} ({});",
                unique,
                index.name,
                table,
                index.columns.join(", ")
            )
        }
        SchemaChange::DropIndex { name, .. } => {
            format!("DROP INDEX IF EXISTS {};", name)
        }

        // --- Foreign Keys ---
        SchemaChange::AddForeignKey { table, fk } => {
            format!(
                "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {} ON UPDATE {};",
                table,
                fk.name,
                fk.columns.join(", "),
                fk.ref_table,
                fk.ref_columns.join(", "),
                fk.on_delete.to_sql(),
                fk.on_update.to_sql()
            )
        }
        SchemaChange::DropForeignKey { table, name } => {
            format!("ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};", table, name)
        }
    }
}

/// Generate DOWN (rollback) SQL for a single schema change.
/// Returns None for irreversible changes.
pub fn change_to_down_sql(change: &SchemaChange) -> Option<String> {
    match change {
        SchemaChange::CreateEnum(e) => Some(format!("DROP TYPE IF EXISTS {};", e.name)),
        SchemaChange::AddEnumValue { .. } => {
            // PostgreSQL cannot remove enum values — irreversible
            None
        }
        SchemaChange::DropEnum { .. } => {
            // Can't recreate without knowing variants
            None
        }

        SchemaChange::CreateTable(t) => Some(format!("DROP TABLE IF EXISTS {} CASCADE;", t.name)),
        SchemaChange::DropTable { .. } => {
            // Can't recreate without knowing columns
            None
        }
        SchemaChange::RenameTable { from, to } => {
            Some(format!("ALTER TABLE {} RENAME TO {};", to, from))
        }

        SchemaChange::AddColumn { table, column } => Some(format!(
            "ALTER TABLE {} DROP COLUMN IF EXISTS {};",
            table, column.name
        )),
        SchemaChange::DropColumn { .. } => {
            // Can't recreate dropped column with data
            None
        }
        SchemaChange::RenameColumn { table, from, to } => Some(format!(
            "ALTER TABLE {} RENAME COLUMN {} TO {};",
            table, to, from
        )),
        SchemaChange::AlterColumnType {
            table,
            column,
            from,
            ..
        } => Some(format!(
            "ALTER TABLE {} ALTER COLUMN {} TYPE {} USING {}::{};",
            table,
            column,
            from.to_ddl(),
            column,
            from.to_ddl()
        )),
        SchemaChange::SetNotNull { table, column, .. } => Some(format!(
            "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL;",
            table, column
        )),
        SchemaChange::DropNotNull { table, column } => Some(format!(
            "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL;",
            table, column
        )),
        SchemaChange::SetDefault { table, column, .. } => Some(format!(
            "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT;",
            table, column
        )),
        SchemaChange::DropDefault { .. } => {
            // Can't restore unknown default
            None
        }
        SchemaChange::AddPrimaryKey { table, name, .. } => Some(format!(
            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
            table, name
        )),
        SchemaChange::DropPrimaryKey { .. } => None,
        SchemaChange::AddUnique { table, name, .. } => Some(format!(
            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
            table, name
        )),
        SchemaChange::DropUnique { .. } => None,
        SchemaChange::AddCheck { table, name, .. } => Some(format!(
            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
            table, name
        )),
        SchemaChange::DropCheck { .. } => None,
        SchemaChange::CreateIndex { index, .. } => {
            Some(format!("DROP INDEX IF EXISTS {};", index.name))
        }
        SchemaChange::DropIndex { .. } => None,
        SchemaChange::AddForeignKey { table, fk } => Some(format!(
            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};",
            table, fk.name
        )),
        SchemaChange::DropForeignKey { .. } => None,
    }
}

/// Generate CREATE TABLE SQL for a complete table definition.
fn generate_create_table(table: &TableDef) -> String {
    let mut parts = Vec::new();

    for col in &table.columns {
        let mut col_sql = format!("  {} {}", col.name, col.sql_type.to_ddl());

        if col.is_auto {
            // Use GENERATED ALWAYS AS IDENTITY for modern PostgreSQL
            col_sql = format!("  {} INTEGER GENERATED ALWAYS AS IDENTITY", col.name);
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
            pk.name,
            pk.columns.join(", ")
        ));
    }

    // Unique constraints
    for uq in &table.unique_constraints {
        parts.push(format!(
            "  CONSTRAINT {} UNIQUE ({})",
            uq.name,
            uq.columns.join(", ")
        ));
    }

    // Check constraints
    for check in &table.check_constraints {
        parts.push(format!(
            "  CONSTRAINT {} CHECK ({})",
            check.name, check.expression
        ));
    }

    // Foreign keys
    for fk in &table.foreign_keys {
        parts.push(format!(
            "  CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {} ON UPDATE {}",
            fk.name,
            fk.columns.join(", "),
            fk.ref_table,
            fk.ref_columns.join(", "),
            fk.on_delete.to_sql(),
            fk.on_update.to_sql()
        ));
    }

    let mut sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (\n{}\n);",
        table.name,
        parts.join(",\n")
    );

    // Indexes (separate statements)
    for idx in &table.indexes {
        let unique = if idx.unique { "UNIQUE " } else { "" };
        sql.push_str(&format!(
            "\nCREATE {}INDEX IF NOT EXISTS {} ON {} ({});",
            unique,
            idx.name,
            table.name,
            idx.columns.join(", ")
        ));
    }

    sql
}
