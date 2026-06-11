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

pub mod schema;
pub mod extract;
pub mod introspect;
pub mod diff;
pub mod plan;
pub mod sql;
pub mod history;
pub mod execute;

use std::path::PathBuf;

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
            database_url: None,
        }
    }
}

const CHECK: &str = "✓ ";
const ERROR: &str = "✗ ";
const WARN: &str = "⚠ ";
const ARROW: &str = "→ ";

/// Main entry point for `doo migrate`.
///
/// Returns exit code (0 = success, 1 = error).
pub fn run_migrate(opts: MigrateOptions) -> Result<i32, String> {
    // Resolve database URL
    let db_url = resolve_database_url(&opts)?;

    // Phase 1: Extract desired schema from .doo sources
    eprintln!("{}Extracting schema from .doo sources...", ARROW);
    let desired = extract::extract_schema(&opts.path)
        .map_err(|e| format!("Schema extraction failed: {}", e))?;

    if desired.tables.is_empty() {
        eprintln!("{}No @table structs found in project", WARN);
        return Ok(0);
    }

    eprintln!(
        "  {}Found {} table(s), {} enum type(s)",
        CHECK,
        desired.tables.len(),
        desired.enums.len()
    );
    for t in &desired.tables {
        eprintln!("    {} ({} columns)", t.name, t.columns.len());
    }

    // Phase 2: Connect to database and introspect current schema
    eprintln!("{}Connecting to database...", ARROW);
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| format!("Failed to create async runtime: {}", e))?;

    let current = runtime.block_on(async {
        let mut client = introspect::connect(&db_url).await?;

        // Ensure migration history table exists
        history::ensure_history_table(&client).await?;

        // Handle status command
        if opts.status {
            history::print_status(&client).await?;
            return Ok::<_, String>(None);
        }

        // Handle rollback command
        if let Some(n) = opts.rollback {
            execute::rollback_migrations(&mut client, n).await?;
            return Ok(None);
        }

        // Introspect current database schema
        eprintln!("  {}Introspecting database schema...", CHECK);
        let schema = introspect::introspect_schema(&client).await?;
        eprintln!(
            "  {}Found {} existing table(s)",
            CHECK,
            schema.tables.len()
        );
        Ok(Some((client, schema)))
    })?;

    // Early return for status/rollback commands
    let (mut client, current) = match current {
        Some(pair) => pair,
        None => return Ok(0),
    };

    // Phase 3: Compute diff
    eprintln!("{}Computing schema diff...", ARROW);
    let changes = diff::compute_diff(&current, &desired);

    if changes.is_empty() {
        eprintln!("  {}Schema is up to date — no changes needed", CHECK);
        return Ok(0);
    }

    // Phase 4: Plan migration
    let migration_plan = plan::build_plan(changes);

    eprintln!(
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
        eprintln!("    {} {} {}", risk_tag, planned.description(), planned.up_sql_preview());
    }

    // Phase 5: Generate SQL
    let up_sql = sql::generate_up_sql(&migration_plan);
    let down_sql = sql::generate_down_sql(&migration_plan);

    if opts.dry_run || opts.diff_only {
        eprintln!();
        eprintln!("=== Migration SQL (dry run) ===");
        eprintln!("{}", up_sql);
        if !down_sql.is_empty() {
            eprintln!();
            eprintln!("=== Rollback SQL ===");
            eprintln!("{}", down_sql);
        }
        return Ok(0);
    }

    // Phase 6: Check for destructive changes requiring approval
    let has_destructive = migration_plan
        .changes
        .iter()
        .any(|c| c.requires_approval);

    if has_destructive && !opts.force {
        eprintln!();
        eprintln!(
            "\x1b[1;31m{}Destructive changes detected!\x1b[0m",
            WARN
        );
        for planned in &migration_plan.changes {
            if planned.requires_approval {
                eprintln!(
                    "  \x1b[31m{}\x1b[0m {}",
                    ERROR,
                    planned.description()
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

    // Phase 7: Execute migration
    eprintln!("{}Executing migration...", ARROW);
    runtime.block_on(async {
        execute::execute_migration(&mut client, &migration_plan, &up_sql, &down_sql).await
    })?;

    eprintln!("  \x1b[1;32m{}Migration complete\x1b[0m", CHECK);
    Ok(0)
}

/// Resolve database URL from options, .env file, or environment variable.
fn resolve_database_url(opts: &MigrateOptions) -> Result<String, String> {
    // 1. CLI flag override
    if let Some(url) = &opts.database_url {
        return Ok(url.clone());
    }

    // 2. Environment variable
    if let Ok(url) = std::env::var(doo_core::constants::env_vars::DATABASE_URL) {
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
                        if key == doo_core::constants::env_vars::DATABASE_URL {
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
        doo_core::constants::env_vars::DATABASE_URL
    ))
}
