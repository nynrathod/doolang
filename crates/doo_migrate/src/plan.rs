//! Migration Planner
//!
//! Takes raw schema changes from the diff engine and produces an ordered,
//! risk-assessed migration plan. Handles dependency ordering, rename detection,
//! and destructive change flagging.

use sha2::{Digest, Sha256};

use crate::diff::SchemaChange;
use serde::Serialize;

// ============================================================================
// Risk Classification
// ============================================================================

/// Risk level for a migration change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, Serialize)]
pub struct MigrationPlan {
    /// Unique migration ID (timestamp-based).
    pub id: String,
    /// Ordered changes to apply.
    pub changes: Vec<PlannedChange>,
    /// SHA-256 checksum of the migration SQL.
    pub checksum: String,
}

/// A single planned change with metadata.
#[derive(Debug, Clone, Serialize)]
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
    // e.g., "change_1"
    pub change_id: String,
    // "schema", "enum", "constraint", "index", "foreign_key"
    pub category: String,
    // "safe", "risky", "destructive"
    pub severity: String,
    // human explanation
    pub reason: String,
    // e.g., ["users", "users.email"]
    pub affected_objects: Vec<String>,
    pub requires_backup: bool,
    pub can_auto_rollback: bool,
}

impl PlannedChange {
    /// Human-readable description of this change.
    pub fn description(&self) -> String {
        self.reason.clone()
    }
    pub fn from_change(change: SchemaChange, index: usize, migration_id: &str) -> Self {
        let risk = classify_risk(&change);
        let requires_approval = matches!(risk, Risk::Destructive);
        let up_sql = crate::sql::change_to_up_sql(&change);
        let down_sql = crate::sql::change_to_down_sql(&change);

        let change_id = format!("{}_{}", migration_id, index);
        let (category, reason, affected_objects) = Self::metadata(&change);
        let severity = match risk {
            Risk::Safe => "safe",
            Risk::Risky => "risky",
            Risk::Destructive => "destructive",
        }
        .to_string();
        let requires_backup = matches!(risk, Risk::Destructive);
        let can_auto_rollback = down_sql.is_some();

        PlannedChange {
            change,
            risk,
            up_sql,
            down_sql,
            requires_approval,
            change_id,
            category,
            severity,
            reason,
            affected_objects,
            requires_backup,
            can_auto_rollback,
        }
    }

    fn metadata(change: &SchemaChange) -> (String, String, Vec<String>) {
        use SchemaChange::*;
        match change {
            CreateEnum(e) => (
                "enum".to_string(),
                format!("Create new enum type '{}'", e.name),
                vec![e.name.clone()],
            ),
            AddEnumValue { enum_name, value } => (
                "enum".to_string(),
                format!("Add value '{}' to enum '{}'", value, enum_name),
                vec![enum_name.clone()],
            ),
            DropEnum { name } => (
                "enum".to_string(),
                format!("Drop enum type '{}' – irreversible data loss", name),
                vec![name.clone()],
            ),
            CreateTable(t) => (
                "schema".to_string(),
                format!("Create new table '{}'", t.name),
                vec![t.name.clone()],
            ),
            DropTable { name } => (
                "schema".to_string(),
                format!("Drop table '{}' – all data lost", name),
                vec![name.clone()],
            ),
            RenameTable { from, to } => (
                "schema".to_string(),
                format!("Rename table '{}' to '{}'", from, to),
                vec![from.clone(), to.clone()],
            ),
            AddColumn { table, column } => (
                "schema".to_string(),
                format!("Add column '{}.{}'", table, column.name),
                vec![table.clone(), format!("{}.{}", table, column.name)],
            ),
            DropColumn { table, column } => (
                "schema".to_string(),
                format!("Drop column '{}.{}' – data lost", table, column),
                vec![table.clone(), format!("{}.{}", table, column)],
            ),
            RenameColumn { table, from, to } => (
                "schema".to_string(),
                format!("Rename column '{}.{}' to '{}'", table, from, to),
                vec![
                    table.clone(),
                    format!("{}.{}", table, from),
                    format!("{}.{}", table, to),
                ],
            ),
            AlterColumnType {
                table,
                column,
                from,
                to,
            } => (
                "schema".to_string(),
                format!(
                    "Change type of '{}.{}' from {} to {} – possible data loss",
                    table, column, from, to
                ),
                vec![table.clone(), format!("{}.{}", table, column)],
            ),
            SetNotNull { table, column, .. } => (
                "constraint".to_string(),
                format!("Make '{}.{}' required (NOT NULL)", table, column),
                vec![table.clone(), format!("{}.{}", table, column)],
            ),
            DropNotNull { table, column } => (
                "constraint".to_string(),
                format!("Allow NULL in '{}.{}'", table, column),
                vec![table.clone(), format!("{}.{}", table, column)],
            ),
            SetDefault {
                table,
                column,
                default,
            } => (
                "constraint".to_string(),
                format!(
                    "Set default value for '{}.{}' to {}",
                    table,
                    column,
                    default.to_sql()
                ),
                vec![table.clone(), format!("{}.{}", table, column)],
            ),
            DropDefault { table, column } => (
                "constraint".to_string(),
                format!("Remove default value from '{}.{}'", table, column),
                vec![table.clone(), format!("{}.{}", table, column)],
            ),
            AddPrimaryKey { table, columns, .. } => (
                "constraint".to_string(),
                format!("Add primary key on {}({})", table, columns.join(", ")),
                vec![table.clone()],
            ),
            DropPrimaryKey { table, .. } => (
                "constraint".to_string(),
                format!("Drop primary key on {}", table),
                vec![table.clone()],
            ),
            AddUnique { table, columns, .. } => (
                "constraint".to_string(),
                format!("Add unique constraint on {}({})", table, columns.join(", ")),
                vec![table.clone()],
            ),
            DropUnique { table, name } => (
                "constraint".to_string(),
                format!("Drop unique constraint {} on {}", name, table),
                vec![table.clone()],
            ),
            AddCheck {
                table, expression, ..
            } => (
                "constraint".to_string(),
                format!("Add check constraint on {}: {}", table, expression),
                vec![table.clone()],
            ),
            DropCheck { table, name } => (
                "constraint".to_string(),
                format!("Drop check constraint {} on {}", name, table),
                vec![table.clone()],
            ),
            CreateIndex { table, index } => (
                "index".to_string(),
                format!("Create index on {}({})", table, index.columns.join(", ")),
                vec![table.clone()],
            ),
            DropIndex { name } => (
                "index".to_string(),
                format!("Drop index {}", name),
                vec![name.clone()],
            ),
            AddForeignKey { table, fk } => (
                "foreign_key".to_string(),
                format!(
                    "Add foreign key {} on {} referencing {}",
                    fk.name, table, fk.ref_table
                ),
                vec![table.clone(), fk.ref_table.clone()],
            ),
            DropForeignKey { table, name } => (
                "foreign_key".to_string(),
                format!("Drop foreign key {} on {}", name, table),
                vec![table.clone()],
            ),
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
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f").to_string();

    // Build planned changes with all metadata
    let mut planned: Vec<PlannedChange> = changes
        .into_iter()
        .enumerate()
        .map(|(idx, change)| PlannedChange::from_change(change, idx, &timestamp))
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
        | SchemaChange::DropForeignKey { .. }
        | SchemaChange::RenameTable { .. }
        | SchemaChange::RenameColumn { .. } => Risk::Risky,

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
