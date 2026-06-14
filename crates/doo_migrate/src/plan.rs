//! Migration Planner
//!
//! Takes raw schema changes from the diff engine and produces an ordered,
//! risk-assessed migration plan. Handles dependency ordering, rename detection,
//! and destructive change flagging.

use sha2::{Digest, Sha256};

use crate::diff::SchemaChange;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

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
    /// Number of connected components in the dependency graph.
    /// Each component is a batch of changes that must be approved/rejected together.
    pub component_count: u32,
    /// Batches (chains) of interdependent changes.
    /// Each batch groups all changes in a connected dependency component.
    /// Independent changes (no deps) form their own single-item batches.
    /// Chained changes (with deps) are grouped together — approving/rejecting
    /// one impacts all others in the chain.
    pub batches: Vec<ChangeBatch>,
}

/// A batch of interdependent changes that must be approved/rejected together.
#[derive(Debug, Clone, Serialize)]
pub struct ChangeBatch {
    /// Component ID — matches `component_id` on each PlannedChange.
    pub component_id: u32,
    /// Ordered list of change IDs in this batch.
    pub change_ids: Vec<String>,
    /// Whether this batch has dependencies (is a chain, not independent).
    /// True = changes are linked by deps → must approve/reject as one unit.
    /// False = changes are independent → can approve/reject individually.
    pub is_chained: bool,
    /// Human-readable description of what this batch does.
    pub description: String,
    /// Summary statistics for this batch.
    pub summary: BatchSummary,
}

/// Summary statistics for a batch of changes.
#[derive(Debug, Clone, Serialize)]
pub struct BatchSummary {
    pub total: usize,
    pub safe_count: usize,
    pub risky_count: usize,
    pub destructive_count: usize,
    pub affected_tables: Vec<String>,
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

    // list of change_id
    pub depends_on: Vec<String>,
    // strings this change depends on
    pub component_id: u32,
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
            depends_on: Vec::new(),
            component_id: 0,
        }
    }

    fn metadata(change: &SchemaChange) -> (String, String, Vec<String>) {
        use SchemaChange::*;
        let category = match change {
            CreateEnum(_) | AddEnumValue { .. } | DropEnum { .. } => "enum",
            CreateTable(_) | DropTable { .. } | RenameTable { .. } => "schema",
            AddColumn { .. } | DropColumn { .. } | RenameColumn { .. } | AlterColumnType { .. } => {
                "schema"
            }
            SetNotNull { .. }
            | DropNotNull { .. }
            | SetDefault { .. }
            | DropDefault { .. }
            | AddPrimaryKey { .. }
            | DropPrimaryKey { .. }
            | AddUnique { .. }
            | DropUnique { .. }
            | AddCheck { .. }
            | DropCheck { .. } => "constraint",
            CreateIndex { .. } | DropIndex { .. } => "index",
            AddForeignKey { .. } | DropForeignKey { .. } => "foreign_key",
        }
        .to_string();

        let reason = match change {
            CreateEnum(e) => format!("Create new enum type '{}'", e.name),
            AddEnumValue { enum_name, value } => {
                format!("Add value '{}' to enum '{}'", value, enum_name)
            }
            DropEnum { name } => format!("Drop enum type '{}' – irreversible data loss", name),
            CreateTable(t) => format!("Create new table '{}'", t.name),
            DropTable { name } => format!("Drop table '{}' – all data lost", name),
            RenameTable { from, to } => format!("Rename table '{}' to '{}'", from, to),
            AddColumn { table, column } => format!("Add column '{}.{}'", table, column.name),
            DropColumn { table, column } => {
                format!("Drop column '{}.{}' – data lost", table, column)
            }
            RenameColumn { table, from, to } => {
                format!("Rename column '{}.{}' to '{}'", table, from, to)
            }
            AlterColumnType {
                table,
                column,
                from,
                to,
                ..
            } => format!(
                "Change type of '{}.{}' from {} to {} – possible data loss",
                table, column, from, to
            ),
            SetNotNull { table, column, .. } => {
                format!("Make '{}.{}' required (NOT NULL)", table, column)
            }
            DropNotNull { table, column } => format!("Allow NULL in '{}.{}'", table, column),
            SetDefault {
                table,
                column,
                default,
            } => {
                format!(
                    "Set default value for '{}.{}' to {}",
                    table,
                    column,
                    default.to_sql()
                )
            }
            DropDefault { table, column } => {
                format!("Remove default value from '{}.{}'", table, column)
            }
            AddPrimaryKey { table, columns, .. } => {
                format!("Add primary key on {}({})", table, columns.join(", "))
            }
            DropPrimaryKey { table, .. } => format!("Drop primary key on {}", table),
            AddUnique { table, columns, .. } => {
                format!("Add unique constraint on {}({})", table, columns.join(", "))
            }
            DropUnique { table, name } => format!("Drop unique constraint {} on {}", name, table),
            AddCheck {
                table, expression, ..
            } => {
                format!("Add check constraint on {}: {}", table, expression)
            }
            DropCheck { table, name } => format!("Drop check constraint {} on {}", name, table),
            CreateIndex { table, index } => {
                format!("Create index on {}({})", table, index.columns.join(", "))
            }
            DropIndex { table, name } => format!("Drop index {} on {}", name, table),
            AddForeignKey { table, fk } => format!(
                "Add foreign key {} on {} referencing {}",
                fk.name, table, fk.ref_table
            ),
            DropForeignKey { table, name } => format!("Drop foreign key {} on {}", name, table),
        };

        // Use the SINGLE SOURCE OF TRUTH from diff.rs
        let affected_objects = crate::diff::affected_objects_for(change);

        (category, reason, affected_objects)
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

    let mut planned: Vec<PlannedChange> = changes
        .into_iter()
        .enumerate()
        .map(|(idx, change)| PlannedChange::from_change(change, idx, &timestamp))
        .collect();

    // Sort by dependency order
    sort_by_dependency(&mut planned);

    // Compute dependency graph and component IDs
    compute_dependencies_and_components(&mut planned);

    // Count unique components
    let component_count = planned.iter().map(|p| p.component_id).max().unwrap_or(0);

    // Build batches from components
    let batches = build_batches(&planned, component_count);

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
        component_count,
        batches,
    }
}

/// Build batch groupings from the planned changes after dependency computation.
///
/// Each connected component becomes a batch. If all changes in a batch have
/// no `depends_on`, the batch is marked `is_chained: false` (independent).
/// Otherwise, the batch is a dependency chain.
fn build_batches(planned: &[PlannedChange], component_count: u32) -> Vec<ChangeBatch> {
    let mut batches: Vec<ChangeBatch> = Vec::new();

    for comp_id in 1..=component_count {
        let members: Vec<&PlannedChange> = planned
            .iter()
            .filter(|p| p.component_id == comp_id)
            .collect();

        if members.is_empty() {
            continue;
        }

        let change_ids: Vec<String> = members.iter().map(|m| m.change_id.clone()).collect();

        // A batch is "chained" if ANY member has dependencies on another
        // member within the same batch OR any member has depends_on set.
        let has_any_deps = members.iter().any(|m| !m.depends_on.is_empty());

        // Check if deps are internal to this batch (depend on another member)
        let has_internal_deps = members.iter().any(|m| {
            m.depends_on
                .iter()
                .any(|dep_id| change_ids.contains(dep_id))
        });

        let is_chained = has_internal_deps || (members.len() > 1 && has_any_deps);

        // Build summary
        let safe_count = members.iter().filter(|m| m.risk == Risk::Safe).count();
        let risky_count = members.iter().filter(|m| m.risk == Risk::Risky).count();
        let destructive_count = members
            .iter()
            .filter(|m| m.risk == Risk::Destructive)
            .count();

        // Collect unique affected tables
        let mut tables: Vec<String> = members
            .iter()
            .flat_map(|m| &m.affected_objects)
            .filter(|obj| !obj.contains('.') && !obj.contains("::"))
            .map(|s| s.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        tables.sort();

        // Build description from the primary table/enum in this batch
        let description = if tables.is_empty() {
            "Schema changes".to_string()
        } else if tables.len() == 1 {
            format!("Changes to {}", tables[0])
        } else if tables.len() <= 3 {
            format!("Changes to {}", tables.join(", "))
        } else {
            format!("Changes to {} and {} other(s)", tables[0], tables.len() - 1)
        };

        batches.push(ChangeBatch {
            component_id: comp_id,
            change_ids,
            is_chained,
            description,
            summary: BatchSummary {
                total: members.len(),
                safe_count,
                risky_count,
                destructive_count,
                affected_tables: tables,
            },
        });
    }

    batches
}

// ============================================================================
// Dependency Graph Computation
// ============================================================================

/// Computes `depends_on` and `component_id` for each PlannedChange.
///
/// ## Dependency Rules
///
/// A change B **depends on** change A (B → A) when A creates or establishes an
/// object that B modifies or references:
///
/// | If change B...                          | It depends on the change that...    |
/// |-----------------------------------------|-------------------------------------|
/// | Adds/modifies/drops a column            | Created the table                   |
/// | Adds/drops a constraint (PK, UQ, FK)    | Created the table                   |
/// | Creates/drops an index                  | Created the table                   |
/// | Adds an enum value                      | Created the enum type               |
/// | Alters a column type to an enum         | Created the enum type               |
/// | Renames a table                         | Created the table (if just created) |
/// | Drops a table                           | Created any FK referencing it (must drop FK first) |
///
/// ## Component Batching
///
/// Changes are grouped into **connected components** (undirected graph) for
/// batch approval. Two changes are in the same component if they share a
/// dependency (direct or transitive) OR if they are linked via a foreign key
/// relationship. This ensures that interdependent schema changes are always
/// approved/rejected as a unit by the frontend.
///
/// ## Structural Dependencies vs Component Grouping
///
/// - **depends_on** (directed): Execution ordering — what must exist before
///   this change can run.
/// - **component_id** (undirected): Approval batching — what changes must be
///   approved or rejected together.
///
/// The directed graph captures structural ordering needs. The undirected graph
/// additionally connects FK-paired changes so that frontend users see coherent
/// "chains" of interrelated changes rather than isolated items.
fn compute_dependencies_and_components(planned: &mut Vec<PlannedChange>) {
    if planned.is_empty() {
        return;
    }

    // ── Phase 1: Collect all "creator" indices ─────────────────────────
    // A "creator" is a change that establishes an object that other changes
    // may reference. Multiple changes can reference the same object key.
    let mut creator_map: HashMap<String, Vec<usize>> = HashMap::new();

    // Track FK relationship edges for component grouping:
    // Each entry: (local_table, ref_table, fk_change_index)
    let mut fk_edges: Vec<(String, String, usize)> = Vec::new();

    for (idx, ch) in planned.iter().enumerate() {
        match &ch.change {
            SchemaChange::CreateTable(t) => {
                creator_map.entry(t.name.clone()).or_default().push(idx);
            }
            SchemaChange::CreateEnum(e) => {
                creator_map.entry(e.name.clone()).or_default().push(idx);
            }
            SchemaChange::RenameTable { to, .. } => {
                // Rename establishes the *new* name as a valid table reference
                creator_map.entry(to.clone()).or_default().push(idx);
            }
            SchemaChange::AddForeignKey { table, fk } => {
                // Record FK relationship for component grouping —
                // changes to either `table` or `ref_table` affect this FK
                fk_edges.push((table.clone(), fk.ref_table.clone(), idx));
            }
            _ => {}
        }
    }

    // ── Phase 2: Build directed dependency edges ───────────────────────
    // Edges go from dependant → dependency (dependant needs dependency first)
    let mut depend_edges: Vec<Vec<usize>> = vec![Vec::new(); planned.len()];

    for (idx, ch) in planned.iter().enumerate() {
        // Use affected_objects (single source of truth from diff.rs) to find
        // what objects this change references and look up their creators.
        for obj in &ch.affected_objects {
            // Determine the key to look up in creator_map:
            // - "table.column" → look up "table" (the table must exist)
            // - "table.fk.name" → look up "table" (the FK's table)
            // - "table" → look up "table"
            // - "enum_name" → look up "enum_name"
            let lookup_key = if let Some(dot_pos) = obj.find('.') {
                let prefix = &obj[..dot_pos];
                // If prefix is a table name, use it
                // (we don't create entries for "table.column" keys, only bare names)
                prefix.to_string()
            } else {
                obj.clone()
            };

            if let Some(creator_indices) = creator_map.get(&lookup_key) {
                for &creator_idx in creator_indices {
                    if creator_idx != idx {
                        if !depend_edges[idx].contains(&creator_idx) {
                            depend_edges[idx].push(creator_idx);
                        }
                    }
                }
            }
        }
    }

    // ── Phase 3: Transitive closure for depends_on ─────────────────────
    // BFS from each node following depend_edges (which go dependant → dependency)
    let mut transitive_deps: Vec<Vec<usize>> = vec![Vec::new(); planned.len()];
    for i in 0..planned.len() {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        // Start BFS from `i`'s direct dependencies
        for &dep in &depend_edges[i] {
            if !visited.contains(&dep) {
                visited.insert(dep);
                queue.push_back(dep);
            }
        }
        while let Some(cur) = queue.pop_front() {
            transitive_deps[i].push(cur);
            for &next_dep in &depend_edges[cur] {
                if !visited.contains(&next_dep) {
                    visited.insert(next_dep);
                    queue.push_back(next_dep);
                }
            }
        }
    }

    // Store depends_on as change_id strings
    for i in 0..planned.len() {
        planned[i].depends_on = transitive_deps[i]
            .iter()
            .map(|&dep_idx| planned[dep_idx].change_id.clone())
            .collect::<Vec<_>>();
    }

    // ── Phase 4: Build undirected graph for connected components ───────
    let mut undirected_adj: Vec<Vec<usize>> = vec![Vec::new(); planned.len()];

    // 4a: Add structural dependency edges (undirected)
    for i in 0..planned.len() {
        for &j in &transitive_deps[i] {
            if !undirected_adj[i].contains(&j) {
                undirected_adj[i].push(j);
            }
            if !undirected_adj[j].contains(&i) {
                undirected_adj[j].push(i);
            }
        }
    }

    // 4b: Add FK-based component edges
    // If change A adds a FK from X → Y, and change B modifies either X or Y,
    // then A and B are in the same component (approving/rejecting one impacts
    // the other).
    for (table, ref_table, fk_idx) in &fk_edges {
        for (other_idx, _other_ch) in planned.iter().enumerate() {
            if other_idx == *fk_idx {
                continue;
            }
            // Check if the other change's affected_objects include either
            // the FK's local table or its referenced table
            for obj in &planned[other_idx].affected_objects {
                let obj_key = if let Some(dot_pos) = obj.find('.') {
                    &obj[..dot_pos]
                } else {
                    obj.as_str()
                };
                if obj_key == table.as_str() || obj_key == ref_table.as_str() {
                    if !undirected_adj[*fk_idx].contains(&other_idx) {
                        undirected_adj[*fk_idx].push(other_idx);
                    }
                    if !undirected_adj[other_idx].contains(fk_idx) {
                        undirected_adj[other_idx].push(*fk_idx);
                    }
                    break;
                }
            }
        }
    }

    // 4c: Add cross-type edges for enum ↔ table connections
    // If a change affects a column with an enum type, and another change
    // affects that enum type, they should be in the same component.
    // We detect this by checking if any change's affected_objects mention
    // a table name that is also an enum name (or vice versa).
    let table_names: HashSet<String> = planned
        .iter()
        .filter_map(|ch| match &ch.change {
            SchemaChange::CreateTable(t) => Some(t.name.clone()),
            SchemaChange::RenameTable { to, .. } => Some(to.clone()),
            _ => None,
        })
        .collect();
    let enum_names: HashSet<String> = planned
        .iter()
        .filter_map(|ch| match &ch.change {
            SchemaChange::CreateEnum(e) => Some(e.name.clone()),
            _ => None,
        })
        .collect();

    for i in 0..planned.len() {
        for obj in &planned[i].affected_objects {
            let obj_key = if let Some(dot_pos) = obj.find('.') {
                &obj[..dot_pos]
            } else {
                obj.as_str()
            };
            // If a change references an object that looks like a table name
            // but also matches an enum name (or vice versa), connect them
            if table_names.contains(obj_key) && enum_names.contains(obj_key) {
                // Find changes that reference this as an enum
                for (j, other_ch) in planned.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    for other_obj in &other_ch.affected_objects {
                        let other_key = if let Some(dot_pos) = other_obj.find('.') {
                            &other_obj[..dot_pos]
                        } else {
                            other_obj.as_str()
                        };
                        if other_key == obj_key {
                            if !undirected_adj[i].contains(&j) {
                                undirected_adj[i].push(j);
                            }
                            if !undirected_adj[j].contains(&i) {
                                undirected_adj[j].push(i);
                            }
                        }
                    }
                }
            }
        }
    }

    // 4d: Connect changes that share the same affected table (same-table grouping).
    // If two changes both affect "users" (even if neither depends on the other),
    // they should be in the same component for batch approval.
    // Build a reverse index: table_name → list of change indices
    let mut table_to_changes: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, ch) in planned.iter().enumerate() {
        for obj in &ch.affected_objects {
            let table_key = if let Some(dot_pos) = obj.find('.') {
                let prefix = &obj[..dot_pos];
                // Skip "fk." pseudo-prefix
                if prefix.ends_with(".fk") || obj.starts_with("fk.") {
                    continue;
                }
                prefix.to_string()
            } else {
                obj.clone()
            };
            // Only use actual table names (skip enum names, index names, etc.)
            // A table name appears as a bare string in affected_objects
            // and also as a prefix in "table.column" entries
            let entry = table_to_changes.entry(table_key).or_default();
            if !entry.contains(&idx) {
                entry.push(idx);
            }
        }
    }
    // Connect all changes that share a table
    for indices in table_to_changes.values() {
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let a = indices[i];
                let b = indices[j];
                if !undirected_adj[a].contains(&b) {
                    undirected_adj[a].push(b);
                }
                if !undirected_adj[b].contains(&a) {
                    undirected_adj[b].push(a);
                }
            }
        }
    }

    // ── Phase 5: Find connected components via DFS ─────────────────────
    let mut component_id = vec![0u32; planned.len()];
    let mut comp_counter = 0u32;
    let mut visited = vec![false; planned.len()];

    for start in 0..planned.len() {
        if visited[start] {
            continue;
        }
        comp_counter += 1;
        let mut stack = vec![start];
        visited[start] = true;
        while let Some(node) = stack.pop() {
            component_id[node] = comp_counter;
            for &neighbor in &undirected_adj[node] {
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    stack.push(neighbor);
                }
            }
        }
    }

    // Assign component_id to each change
    for i in 0..planned.len() {
        planned[i].component_id = component_id[i];
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
