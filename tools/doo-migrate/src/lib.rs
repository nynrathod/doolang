//! Doo Migration Engine
//!
//! Compiler-level schema diffing and safe DDL execution.
//! Extracts desired schema from `.doo` source files via the HIR pipeline,
//! compares against live PostgreSQL database state, and generates+executes
//! safe migrations with full transaction support.
//!
//! ## Architecture
//! ```text
//! .doo files → Lex → Parse → HIR → extract → DesiredSchema
//!                                                    ↓
//! PostgreSQL → introspect → CurrentSchema → diff → MigrationPlan
//!                                                    ↓
//!                                              sql → execute
//! ```

pub mod diff;
pub mod execute;
pub mod extract;
pub mod history;
pub mod introspect;
pub mod plan;
pub mod schema;
pub mod sql;

use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
struct MigrationReport {
    status: String,
    can_apply: bool,
    blocked_reason: Option<String>,
    execute_mode: String,
    summary: MigrationSummary,
    migration_plan: Option<plan::MigrationPlan>,
    history: Option<Vec<MigrationRecord>>,
}

#[derive(Serialize)]
struct MigrationSummary {
    total_changes: usize,
    safe_count: usize,
    risky_count: usize,
    destructive_count: usize,
    affected_tables: Vec<String>,
    affected_enums: Vec<String>,
}

#[derive(Serialize)]
struct MigrationRecord {
    id: String,
    applied_at: String,
    status: String,
    duration_ms: Option<i64>,
}

/// Options for the migrate command.
#[derive(Debug, Clone)]
pub struct MigrateOptions {
    /// Path to project directory or main.doo
    pub path: PathBuf,
    /// Show SQL without executing
    pub dry_run: bool,
    /// Show migration status/history
    pub status: bool,
    /// Rollback N migrations
    pub rollback: Option<u32>,
    /// Auto-approve destructive changes
    pub force: bool,
    /// Show diff without executing
    pub diff_only: bool,
    /// Json output
    pub json_output: bool,
    /// Database URL override (reads DATABASE_URL from .env otherwise)
    pub database_url: Option<String>,
}

impl Default for MigrateOptions {
    fn default() -> Self {
        Self {
            path: PathBuf::from("."),
            dry_run: false,
            status: false,
            rollback: None,
            force: false,
            diff_only: false,
            json_output: false,
            database_url: None,
        }
    }
}

const CHECK: &str = "✓ ";
const ERROR: &str = "✗ ";
const WARN: &str = "⚠ ";
const ARROW: &str = "→ ";

// ─── System / Internal Tables ───────────────────────────────────────────────
// These tables are managed by the Doo runtime (migration history, webhook logs,
// etc.) and MUST NEVER appear in user-facing schema diffs, change detection,
// or migration plans. They are completely invisible to the user.
//
// SINGLE SOURCE OF TRUTH: Add new system tables HERE. Both the introspector
// and the diff engine reference this list.

/// Tables managed internally by the Doo runtime — never part of user schema.
pub const SYSTEM_TABLES: &[&str] = &["doo_migrations", "webhook_deliveries"];

/// Returns `true` if the given table name (case-insensitive) is a system table.
pub fn is_system_table(name: &str) -> bool {
    let lower = name.to_lowercase();
    SYSTEM_TABLES.iter().any(|t| *t == lower.as_str())
}

/// Build a SQL `NOT IN (...)` clause for excluding system tables from queries.
/// Returns a comma-separated, single-quoted list suitable for `NOT IN (...)`.
pub fn system_tables_sql_exclusion() -> String {
    SYSTEM_TABLES
        .iter()
        .map(|t| format!("'{}'", t))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Run an async future to completion, safely handling the case where a Tokio
/// runtime already exists on the current thread (e.g., when `doo migrate` is
/// invoked as a subprocess from the DooCloud server).
///
/// Uses a persistent runtime (OnceLock) so that deadpool connections stay
/// alive across multiple `block_on_safe` calls within the same process.
/// If no runtime is active, creates a new multi-threaded runtime once and
/// reuses it. If already inside a runtime, uses `block_in_place` to avoid
/// the "Cannot start a runtime from within a runtime" panic.
fn block_on_safe<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    use std::sync::OnceLock;
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

    let rt = RT.get_or_init(|| {
        tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime for migration")
    });

    match tokio::runtime::Handle::try_current() {
        Ok(_) => {
            // Already inside a runtime (e.g., subprocess of DooCloud server).
            // block_in_place lets the current runtime drive this future
            // without panicking or blocking worker threads.
            tokio::task::block_in_place(|| rt.block_on(f))
        }
        Err(_) => rt.block_on(f),
    }
}

/// Main entry point for `doo migrate`.
///
/// Returns exit code (0 = success, 1 = error).
pub fn run_migrate(opts: MigrateOptions) -> Result<i32, String> {
    // Resolve database URL
    let db_url = resolve_database_url(&opts)?;

    // Helper to conditionally print human output
    macro_rules! human_println {
        ($($arg:tt)*) => {
            if !opts.json_output {
                eprintln!($($arg)*);
            }
        };
    }

    // Phase 1: Extract desired schema
    human_println!("{}Extracting schema from .doo sources...", ARROW);
    let desired = extract::extract_schema(&opts.path)
        .map_err(|e| format!("Schema extraction failed: {}", e))?;

    if desired.tables.is_empty() && !opts.status && opts.rollback.is_none() {
        human_println!("{}No tables found in project", WARN);
        if opts.json_output {
            let summary = MigrationSummary {
                total_changes: 0,
                safe_count: 0,
                risky_count: 0,
                destructive_count: 0,
                affected_tables: vec![],
                affected_enums: vec![],
            };
            let report = MigrationReport {
                status: "no_tables".to_string(),
                can_apply: true,
                blocked_reason: None,
                execute_mode: "plan".to_string(),
                summary,
                migration_plan: None,
                history: None,
            };
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        }
        return Ok(0);
    }

    human_println!(
        "  {}Found {} table(s), {} enum type(s)",
        CHECK,
        desired.tables.len(),
        desired.enums.len()
    );
    for t in &desired.tables {
        human_println!("    {} ({} columns)", t.name, t.columns.len());
    }

    // Phase 2: Connect and introspect (async runtime)
    human_println!("{}Connecting to database...", ARROW);

    let result = block_on_safe(async {
        let mut client = introspect::connect(&db_url).await?;
        history::ensure_history_table(&client).await?;

        // Handle status command
        if opts.status {
            let history = fetch_history_json(&client).await?;
            if opts.json_output {
                let summary = MigrationSummary {
                    total_changes: 0,
                    safe_count: 0,
                    risky_count: 0,
                    destructive_count: 0,
                    affected_tables: vec![],
                    affected_enums: vec![],
                };
                let report = MigrationReport {
                    status: "history".to_string(),
                    can_apply: true,
                    blocked_reason: None,
                    execute_mode: "status".to_string(),
                    summary,
                    migration_plan: None,
                    history: Some(history),
                };
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
                return Ok::<_, String>(None);
            } else {
                history::print_status(&client).await?;
                return Ok(None);
            }
        }

        // Handle rollback
        if let Some(n) = opts.rollback {
            execute::rollback_migrations(&mut client, n).await?;
            if opts.json_output {
                let summary = MigrationSummary {
                    total_changes: 0,
                    safe_count: 0,
                    risky_count: 0,
                    destructive_count: 0,
                    affected_tables: vec![],
                    affected_enums: vec![],
                };
                let report = MigrationReport {
                    status: "rolled_back".to_string(),
                    can_apply: true,
                    blocked_reason: None,
                    execute_mode: "rollback".to_string(),
                    summary,
                    migration_plan: None,
                    history: None,
                };
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
            }
            return Ok(None);
        }

        // Introspect current schema
        human_println!("  {}Introspecting database schema...", CHECK);
        let current = introspect::introspect_schema(&client).await?;
        human_println!(
            "  {}Found {} existing table(s)",
            CHECK,
            current.tables.len()
        );

        Ok(Some((client, current)))
    })?;

    let (mut client, current) = match result {
        Some(pair) => pair,
        None => return Ok(0),
    };

    // Phase 3: Compute diff
    human_println!("{}Computing schema diff...", ARROW);
    let changes = diff::compute_diff(&current, &desired);

    if changes.is_empty() {
        human_println!("  {}Schema is up to date — no changes needed", CHECK);
        if opts.json_output {
            let summary = MigrationSummary {
                total_changes: 0,
                safe_count: 0,
                risky_count: 0,
                destructive_count: 0,
                affected_tables: vec![],
                affected_enums: vec![],
            };
            let report = MigrationReport {
                status: "up_to_date".to_string(),
                can_apply: true,
                blocked_reason: None,
                execute_mode: "plan".to_string(),
                summary,
                migration_plan: None,
                history: None,
            };
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        }
        return Ok(0);
    }

    // Phase 4: Plan migration
    let mut migration_plan = plan::build_plan(changes);

    // === Compute summary for JSON output ===
    let summary = {
        let safe = migration_plan
            .changes
            .iter()
            .filter(|c| c.risk == plan::Risk::Safe)
            .count();
        let risky = migration_plan
            .changes
            .iter()
            .filter(|c| c.risk == plan::Risk::Risky)
            .count();
        let destructive = migration_plan
            .changes
            .iter()
            .filter(|c| c.risk == plan::Risk::Destructive)
            .count();

        let mut tables = std::collections::HashSet::new();
        let mut enums = std::collections::HashSet::new();
        for ch in &migration_plan.changes {
            if ch.category == "enum" {
                for obj in &ch.affected_objects {
                    enums.insert(obj.clone());
                }
            }
            for obj in &ch.affected_objects {
                if !obj.contains('.') && ch.category != "enum" {
                    tables.insert(obj.clone());
                }
            }
        }
        MigrationSummary {
            total_changes: migration_plan.changes.len(),
            safe_count: safe,
            risky_count: risky,
            destructive_count: destructive,
            affected_tables: tables.into_iter().collect(),
            affected_enums: enums.into_iter().collect(),
        }
    };

    let has_destructive = migration_plan.changes.iter().any(|c| c.requires_approval);
    let can_apply = !has_destructive || opts.force;
    let blocked_reason = if has_destructive && !opts.force {
        Some("Destructive changes require --force flag".to_string())
    } else {
        None
    };
    let execute_mode = if opts.dry_run || opts.diff_only {
        "dry_run"
    } else if opts.rollback.is_some() {
        "rollback"
    } else if opts.status {
        "status"
    } else {
        "apply"
    }
    .to_string();

    // ── Count affected rows for ALL changes (not just destructive) ────────
    // This runs here so both --json and --diff output include row counts,
    // not just the interactive approval prompt.
    let row_counts = block_on_safe(async { count_affected_rows(&client, &migration_plan).await });
    for planned in &mut migration_plan.changes {
        if let Some(&count) = row_counts.get(&planned.change_id) {
            if count >= 0 {
                planned.affected_rows = Some(count);
            }
        }
    }

    // Phase 5: Generate SQL (already inside plan)
    let up_sql = sql::generate_up_sql(&migration_plan);
    let down_sql = sql::generate_down_sql(&migration_plan);

    human_println!(
        "  {}Planned {} change(s):",
        CHECK,
        migration_plan.changes.len()
    );
    for planned in &migration_plan.changes {
        let risk_tag = match planned.risk {
            plan::Risk::Safe => "\x1b[32m[safe]\x1b[0m",
            plan::Risk::Risky => "\x1b[33m[risky]\x1b[0m",
            plan::Risk::Destructive => "\x1b[31m[destructive]\x1b[0m",
        };
        let rows = planned
            .affected_rows
            .map(|n| format!("  ({} row(s) affected)", n))
            .unwrap_or_default();
        human_println!(
            "    {} {} {}{}",
            risk_tag,
            planned.description(),
            planned.up_sql_preview(),
            rows,
        );
    }

    if opts.dry_run || opts.diff_only {
        if opts.json_output {
            let report = MigrationReport {
                status: "dry_run".to_string(),
                can_apply,
                blocked_reason: blocked_reason.clone(),
                execute_mode: execute_mode.clone(),
                summary,
                migration_plan: Some(migration_plan),
                history: None,
            };
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else {
            eprintln!();
            eprintln!("=== Migration SQL (dry run) ===");
            eprintln!("{}", up_sql);
            if !down_sql.is_empty() {
                eprintln!();
                eprintln!("=== Rollback SQL ===");
                eprintln!("{}", down_sql);
            }
        }
        return Ok(0);
    }

    // Phase 6: Destructive changes approval
    if has_destructive && !opts.force {
        if opts.json_output {
            let report = MigrationReport {
                status: "destructive_changes_require_force".to_string(),
                can_apply,
                blocked_reason: blocked_reason.clone(),
                execute_mode: execute_mode.clone(),
                summary,
                migration_plan: Some(migration_plan),
                history: None,
            };
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
            return Ok(1);
        } else {
            eprintln!();
            eprintln!("\x1b[1;31m{}Destructive changes detected!\x1b[0m", WARN);

            // affected_rows already populated above — use it directly
            for planned in &migration_plan.changes {
                if planned.requires_approval {
                    let count_str = planned
                        .affected_rows
                        .map(|n| format!("  ({} row(s) affected)", n))
                        .unwrap_or_default();
                    eprintln!(
                        "  \x1b[31m{}\x1b[0m {}{}",
                        ERROR,
                        planned.description(),
                        count_str
                    );
                }
            }
            let confirm = dialoguer::Confirm::new()
                .with_prompt("Apply these destructive changes?")
                .default(false)
                .interact()
                .unwrap_or(false);
            if !confirm {
                eprintln!("{}Migration cancelled", ERROR);
                return Ok(1);
            }
        }
    }

    // Phase 7: Execute migration
    human_println!("{}Executing migration...", ARROW);
    block_on_safe(async {
        execute::execute_migration(&mut client, &migration_plan, &up_sql, &down_sql).await
    })?;

    human_println!("  \x1b[1;32m{}Migration complete\x1b[0m", CHECK);

    if opts.json_output {
        let report = MigrationReport {
            status: "applied".to_string(),
            can_apply: true,
            blocked_reason: None,
            execute_mode: "apply".to_string(),
            summary,
            migration_plan: Some(migration_plan),
            history: None,
        };
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    }

    Ok(0)
}

/// Count affected rows for every change in the plan so the user can see
/// how much data will be touched before confirming.
///
/// Uses pg_class reltuples estimate for full-table operations,
/// and COUNT(*) queries for column-level operations.
///
/// Single source of truth — called once, populates PlannedChange.affected_rows.
async fn count_affected_rows(
    client: &deadpool_postgres::Client,
    plan: &plan::MigrationPlan,
) -> std::collections::HashMap<String, i64> {
    use crate::diff::SchemaChange;
    let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();

    for planned in &plan.changes {
        let count_result = match &planned.change {
            SchemaChange::DropTable { name, .. } => {
                // Fast estimate from pg_class — avoids full table scan
                let query = format!(
                    "SELECT COALESCE(reltuples::bigint, 0) FROM pg_class WHERE relname = '{}'",
                    name.replace('\'', "''")
                );
                client
                    .query_opt(&query, &[])
                    .await
                    .map(|row| row.map(|r| r.get::<_, i64>(0)).unwrap_or(0))
            }
            SchemaChange::DropColumn { table, column, .. }
            | SchemaChange::AlterColumnType { table, column, .. } => {
                let query = format!(
                    "SELECT COUNT(*) FROM \"{}\" WHERE \"{}\" IS NOT NULL",
                    table.replace('\'', "''"),
                    column.replace('\'', "''")
                );
                client
                    .query_one(&query, &[])
                    .await
                    .map(|row| row.get::<_, i64>(0))
            }
            SchemaChange::RenameTable { from, .. } => {
                // Rename touches every row — use source table count
                let query = format!(
                    "SELECT COALESCE(reltuples::bigint, 0) FROM pg_class WHERE relname = '{}'",
                    from.replace('\'', "''")
                );
                client
                    .query_opt(&query, &[])
                    .await
                    .map(|row| row.map(|r| r.get::<_, i64>(0)).unwrap_or(0))
            }
            SchemaChange::RenameColumn { table, .. } => {
                // Rename touches every row — total count
                let query = format!(
                    "SELECT COALESCE(reltuples::bigint, 0) FROM pg_class WHERE relname = '{}'",
                    table.replace('\'', "''")
                );
                client
                    .query_opt(&query, &[])
                    .await
                    .map(|row| row.map(|r| r.get::<_, i64>(0)).unwrap_or(0))
            }
            SchemaChange::SetNotNull { table, column, .. } => {
                // Rows with NULL in this column — these would cause the ALTER to fail
                let query = format!(
                    "SELECT COUNT(*) FROM \"{}\" WHERE \"{}\" IS NULL",
                    table.replace('\'', "''"),
                    column.replace('\'', "''")
                );
                client
                    .query_one(&query, &[])
                    .await
                    .map(|row| row.get::<_, i64>(0))
            }
            SchemaChange::DropNotNull { table, column, .. } => {
                // Total rows in table — all will now allow nulls
                let query = format!(
                    "SELECT COALESCE(reltuples::bigint, 0) FROM pg_class WHERE relname = '{}'",
                    table.replace('\'', "''")
                );
                client
                    .query_opt(&query, &[])
                    .await
                    .map(|row| row.map(|r| r.get::<_, i64>(0)).unwrap_or(0))
            }
            SchemaChange::AddColumn { table, column } if !column.nullable => {
                // Adding a NOT NULL column without a default on an existing table
                // requires a full-table scan to backfill. Show total rows.
                let query = format!(
                    "SELECT COALESCE(reltuples::bigint, 0) FROM pg_class WHERE relname = '{}'",
                    table.replace('\'', "''")
                );
                client
                    .query_opt(&query, &[])
                    .await
                    .map(|row| row.map(|r| r.get::<_, i64>(0)).unwrap_or(0))
            }
            SchemaChange::DropEnum { .. } => {
                // Enum drops are tracked implicitly — can't easily count
                // which tables use the enum without an expensive scan.
                Ok(-1)
            }
            _ => Ok(0),
        };

        match count_result {
            Ok(count) => {
                counts.insert(planned.change_id.clone(), count);
            }
            Err(e) => {
                eprintln!(
                    "  {}Failed to count rows for {}: {}",
                    ERROR, planned.change_id, e
                );
            }
        }
    }

    counts
}

/// Resolve database URL from options, .env file, or environment variable.
fn resolve_database_url(opts: &MigrateOptions) -> Result<String, String> {
    // 1. CLI flag override
    if let Some(url) = &opts.database_url {
        return Ok(url.clone());
    }

    // 2. Environment variable
    if let Ok(url) = std::env::var("DATABASE_URL") {
        return Ok(url);
    }

    // 3. Walk up from project path to find .env file
    let start = if opts.path.is_file() {
        opts.path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf()
    } else {
        opts.path.clone()
    };

    let mut current = start;
    loop {
        let env_file = current.join(".env");
        if env_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&env_file) {
                for line in content.lines() {
                    let line = line.trim();
                    if line.starts_with('#') || line.is_empty() {
                        continue;
                    }
                    if let Some((key, value)) = line.split_once('=') {
                        let key = key.trim().trim_start_matches("export ").trim();
                        if key == "DATABASE_URL" {
                            let value = value.trim();
                            // Strip surrounding quotes
                            let value = if value.len() >= 2 {
                                let bytes = value.as_bytes();
                                if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
                                    || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
                                {
                                    &value[1..value.len() - 1]
                                } else {
                                    value
                                }
                            } else {
                                value
                            };
                            return Ok(value.to_string());
                        }
                    }
                }
            }
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => break,
        }
    }

    Err(format!(
        "{} not found. Set it in .env, environment, or use --database-url",
        "DATABASE_URL"
    ))
}

async fn fetch_history_json(
    client: &deadpool_postgres::Client,
) -> Result<Vec<MigrationRecord>, String> {
    let rows = client
        .query(
            "SELECT id, applied_at, status, duration_ms FROM doo_migrations ORDER BY id DESC",
            &[],
        )
        .await
        .map_err(|e| format!("Failed to query migration history: {}", e))?;

    let mut records = Vec::new();
    for row in rows {
        records.push(MigrationRecord {
            id: row.get(0),
            applied_at: row.get::<_, chrono::DateTime<chrono::Utc>>(1).to_rfc3339(),
            status: row.get(2),
            duration_ms: row.get(3),
        });
    }
    Ok(records)
}
