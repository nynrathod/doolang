//! Migration Planner
//!
//! Takes raw schema changes from the diff engine and produces an ordered,
//! risk-assessed migration plan. Handles dependency ordering, rename detection,
//! and destructive change flagging.

use sha2::{Digest, Sha256};

use crate::diff::SchemaChange;

// ============================================================================
// Risk Classification
// ============================================================================

/// Risk level for a migration change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    /// No data loss possible.
    Safe,
    /// Potential data impact but recoverable.
    Risky,
    /// Data loss or irreversible change.
    Destructive,
}

// ============================================================================
// Migration Plan
// ============================================================================

/// Complete migration plan — ordered list of changes with risk assessment.
#[derive(Debug, Clone)]
pub struct MigrationPlan {
    /// Unique migration ID (timestamp-based).
    pub id: String,
    /// Ordered changes to apply.
    pub changes: Vec<PlannedChange>,
    /// SHA-256 checksum of the migration SQL.
    pub checksum: String,
}

/// A single planned change with metadata.
#[derive(Debug, Clone)]
pub struct PlannedChange {
    /// The schema change to apply.
    pub change: SchemaChange,
    /// Risk level.
    pub risk: Risk,
    /// Up (forward) SQL statement.
    pub up_sql: String,
    /// Down (rollback) SQL statement. None = irreversible.
    pub down_sql: Option<String>,
    /// Whether this change requires user approval.
    pub requires_approval: bool,
}

impl PlannedChange {
    /// Human-readable description of this change.
    pub fn description(&self) -> String {
        match &self.change {
            SchemaChange::CreateEnum(e) => {
                format!("Create enum type '{}'", e.name)
            }
            SchemaChange::AddEnumValue { enum_name, value } => {
                format!("Add value '{}' to enum '{}'", value, enum_name)
            }
            SchemaChange::DropEnum { name } => {
                format!("Drop enum type '{}'", name)
            }
            SchemaChange::CreateTable(t) => {
                format!("Create table '{}' ({} columns)", t.name, t.columns.len())
            }
            SchemaChange::DropTable { name } => {
                format!("Drop table '{}'", name)
            }
            SchemaChange::RenameTable { from, to } => {
                format!("Rename table '{}' → '{}'", from, to)
            }
            SchemaChange::AddColumn { table, column } => {
                format!(
                    "Add column '{}.{}' ({})",
                    table,
                    column.name,
                    column.sql_type.to_ddl()
                )
            }
            SchemaChange::DropColumn { table, column } => {
                format!("Drop column '{}.{}'", table, column)
            }
            SchemaChange::RenameColumn { table, from, to } => {
                format!("Rename column '{}.{}' → '{}'", table, from, to)
            }
            SchemaChange::AlterColumnType {
                table,
                column,
                from,
                to,
            } => {
                format!(
                    "Change type '{}.{}' {} → {}",
                    table,
                    column,
                    from.to_ddl(),
                    to.to_ddl()
                )
            }
            SchemaChange::SetNotNull { table, column, .. } => {
                format!("Set NOT NULL on '{}.{}'", table, column)
            }
            SchemaChange::DropNotNull { table, column } => {
                format!("Drop NOT NULL on '{}.{}'", table, column)
            }
            SchemaChange::SetDefault {
                table,
                column,
                default,
            } => {
                format!(
                    "Set default on '{}.{}' = {}",
                    table,
                    column,
                    default.to_sql()
                )
            }
            SchemaChange::DropDefault { table, column } => {
                format!("Drop default on '{}.{}'", table, column)
            }
            SchemaChange::AddPrimaryKey { table, columns, .. } => {
                format!("Add primary key on '{}' ({})", table, columns.join(", "))
            }
            SchemaChange::DropPrimaryKey { table, .. } => {
                format!("Drop primary key on '{}'", table)
            }
            SchemaChange::AddUnique { table, columns, .. } => {
                format!(
                    "Add unique constraint on '{}' ({})",
                    table,
                    columns.join(", ")
                )
            }
            SchemaChange::DropUnique { table, name } => {
                format!("Drop unique constraint '{}' on '{}'", name, table)
            }
            SchemaChange::AddCheck {
                table, expression, ..
            } => {
                format!("Add check constraint on '{}': {}", table, expression)
            }
            SchemaChange::DropCheck { table, name } => {
                format!("Drop check constraint '{}' on '{}'", name, table)
            }
            SchemaChange::CreateIndex { table, index } => {
                format!("Create index on '{}' ({})", table, index.columns.join(", "))
            }
            SchemaChange::DropIndex { name } => {
                format!("Drop index '{}'", name)
            }
            SchemaChange::AddForeignKey { table, fk } => {
                format!(
                    "Add foreign key '{}.{}' → '{}.{}'",
                    table,
                    fk.columns.join(", "),
                    fk.ref_table,
                    fk.ref_columns.join(", ")
                )
            }
            SchemaChange::DropForeignKey { table, name } => {
                format!("Drop foreign key '{}' on '{}'", name, table)
            }
        }
    }

    /// Short preview of the SQL for display.
    pub fn up_sql_preview(&self) -> String {
        if self.up_sql.len() > 80 {
            format!("{}...", &self.up_sql[..77])
        } else {
            self.up_sql.clone()
        }
    }
}

// ============================================================================
// Plan Builder
// ============================================================================

/// Build a migration plan from raw schema changes.
///
/// Orders changes by dependency (enums before tables, tables before FKs, etc.)
/// and classifies risk levels.
pub fn build_plan(changes: Vec<SchemaChange>) -> MigrationPlan {
    // Include sub-second precision so that sequential migrations within the
    // same second receive unique IDs.  Without this, idempotency incorrectly
    // skips later migrations because they collide with an earlier migration's
    // ID marked as "applied" in `doo_migrations`.
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f").to_string();

    // Classify and generate SQL for each change
    let mut planned: Vec<PlannedChange> = changes
        .into_iter()
        .map(|change| {
            let risk = classify_risk(&change);
            let requires_approval = matches!(risk, Risk::Destructive);
            let up_sql = crate::sql::change_to_up_sql(&change);
            let down_sql = crate::sql::change_to_down_sql(&change);

            PlannedChange {
                change,
                risk,
                up_sql,
                down_sql,
                requires_approval,
            }
        })
        .collect();

    // Sort by dependency order
    sort_by_dependency(&mut planned);

    // Compute checksum
    let all_sql: String = planned
        .iter()
        .map(|p| p.up_sql.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut hasher = Sha256::new();
    hasher.update(all_sql.as_bytes());
    let checksum = format!("{:x}", hasher.finalize());

    MigrationPlan {
        id: timestamp,
        changes: planned,
        checksum,
    }
}

/// Classify risk level for a schema change.
fn classify_risk(change: &SchemaChange) -> Risk {
    match change {
        // Safe operations — no data loss
        SchemaChange::CreateEnum(_)
        | SchemaChange::AddEnumValue { .. }
        | SchemaChange::CreateTable(_)
        | SchemaChange::AddColumn { .. }
        | SchemaChange::RenameTable { .. }
        | SchemaChange::RenameColumn { .. }
        | SchemaChange::DropNotNull { .. }
        | SchemaChange::SetDefault { .. }
        | SchemaChange::AddPrimaryKey { .. }
        | SchemaChange::AddUnique { .. }
        | SchemaChange::AddCheck { .. }
        | SchemaChange::CreateIndex { .. }
        | SchemaChange::AddForeignKey { .. } => Risk::Safe,

        // Risky — might fail on existing data
        SchemaChange::SetNotNull { .. }
        | SchemaChange::DropDefault { .. }
        | SchemaChange::DropPrimaryKey { .. }
        | SchemaChange::DropUnique { .. }
        | SchemaChange::DropCheck { .. }
        | SchemaChange::DropIndex { .. }
        | SchemaChange::DropForeignKey { .. } => Risk::Risky,

        // Type changes — depends on whether the cast is safe
        SchemaChange::AlterColumnType { from, to, .. } => {
            if from.is_safe_cast_to(to) {
                Risk::Risky
            } else {
                Risk::Destructive
            }
        }

        // Destructive — data loss
        SchemaChange::DropTable { .. }
        | SchemaChange::DropColumn { .. }
        | SchemaChange::DropEnum { .. } => Risk::Destructive,
    }
}

/// Sort changes by dependency order.
///
/// Order: enums → create tables → add columns → alter columns →
///        constraints → indexes → foreign keys → drops (reverse order)
fn sort_by_dependency(changes: &mut Vec<PlannedChange>) {
    changes.sort_by_key(|p| change_order(&p.change));
}

/// Dependency ordering key.
fn change_order(change: &SchemaChange) -> u32 {
    match change {
        // Phase 1: Enum types (tables may reference them)
        SchemaChange::CreateEnum(_) => 10,
        SchemaChange::AddEnumValue { .. } => 11,

        // Phase 2: Create tables (before anything references them)
        SchemaChange::CreateTable(_) => 20,
        SchemaChange::RenameTable { .. } => 21,

        // Phase 3: Column changes
        SchemaChange::AddColumn { .. } => 30,
        SchemaChange::RenameColumn { .. } => 31,
        SchemaChange::AlterColumnType { .. } => 32,
        SchemaChange::SetDefault { .. } => 33,
        SchemaChange::SetNotNull { .. } => 34,
        SchemaChange::DropNotNull { .. } => 35,
        SchemaChange::DropDefault { .. } => 36,

        // Phase 4: Constraints
        SchemaChange::AddPrimaryKey { .. } => 40,
        SchemaChange::AddUnique { .. } => 41,
        SchemaChange::AddCheck { .. } => 42,

        // Phase 5: Indexes
        SchemaChange::CreateIndex { .. } => 50,

        // Phase 6: Foreign keys (after all tables/columns exist)
        SchemaChange::AddForeignKey { .. } => 60,

        // Phase 7: Drops (reverse dependency order)
        SchemaChange::DropForeignKey { .. } => 70,
        SchemaChange::DropIndex { .. } => 71,
        SchemaChange::DropCheck { .. } => 72,
        SchemaChange::DropUnique { .. } => 73,
        SchemaChange::DropPrimaryKey { .. } => 74,
        SchemaChange::DropColumn { .. } => 80,
        SchemaChange::DropTable { .. } => 90,
        SchemaChange::DropEnum { .. } => 100,
    }
}
