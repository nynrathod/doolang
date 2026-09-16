//! Migration Executor
//!
//! Executes migration plans against the live database with safety layers:
//! - Advisory locks (prevent concurrent migrations)
//! - History recording (insert into `doo_migrations`)
//! - Rollback support
//!
//! ## Execution Strategy
//!
//! DDL statements are executed individually via `simple_query` (PostgreSQL's
//! simple query protocol) — the same protocol used by `psql`. Each statement
//! gets its own request-response cycle, ensuring reliable persistence.
//!
//! We intentionally avoid `batch_execute` (multi-statement simple query) and
//! `execute` (extended query protocol) because:
//! - `batch_execute` may not reliably persist DDL in all tokio-postgres versions
//! - `execute` uses Parse/Bind/Execute which restricts certain DDL (ALTER TYPE,
//!   DROP INDEX, ALTER COLUMN SET NOT NULL)

use deadpool_postgres::Client;

use crate::history;
use crate::plan::MigrationPlan;

// Hardcoded lock ID for doo_migrate (matching standard practices)
const ADVISORY_LOCK_ID: i64 = 4242424242;

/// Split a SQL string into individual statements on `;` boundaries,
/// respecting single-quoted string literals (so `;` inside `'...'` is preserved).
/// Each returned statement is trimmed and guaranteed to end with `;`.
fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut chars = sql.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\'' {
            // Enter single-quoted string — consume until closing unescaped quote
            current.push(ch);
            while let Some(qch) = chars.next() {
                current.push(qch);
                if qch == '\'' {
                    // Check for escaped quote ''
                    if chars.peek() == Some(&'\'') {
                        current.push(chars.next().unwrap());
                    } else {
                        break; // End of quoted string
                    }
                }
            }
        } else if ch == ';' {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                statements.push(if trimmed.ends_with(';') {
                    trimmed
                } else {
                    format!("{};", trimmed)
                });
            }
            current = String::new();
        } else {
            current.push(ch);
        }
    }

    // Flush final trailing fragment (statements without trailing ;)
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        statements.push(if trimmed.ends_with(';') {
            trimmed
        } else {
            format!("{};", trimmed)
        });
    }

    statements
}

/// Execute a batch of SQL statements.
///
/// Each DDL statement is wrapped in its own explicit `BEGIN`/`COMMIT` block.
/// This guarantees persistence regardless of client-side autocommit state —
/// PostgreSQL always commits data that reaches a successful `COMMIT`.
/// This is the most portable and reliable DDL execution strategy, matching
/// what battle-tested migration tools (Alembic, Flyway, etc.) do internally.
async fn execute_sql_batch(client: &Client, sql: &str) -> Result<(), String> {
    let statements = split_sql_statements(sql);
    if statements.is_empty() {
        return Ok(());
    }

    for stmt in &statements {
        // Wrap each DDL statement in its own transaction to guarantee
        // persistence.  This is the same pattern used by Alembic
        // (SQLAlchemy) and Flyway — explicit COMMIT removes any
        // ambiguity about autocommit state.
        //
        // We wrap per-statement rather than the whole batch so that
        // one failing statement does not roll back the others.
        let wrapped = format!("BEGIN;\n{}\nCOMMIT;", stmt);
        client
            .simple_query(&wrapped)
            .await
            .map_err(|e| format!("SQL error: {}\n  Statement: {}", e, stmt))?;
    }

    Ok(())
}

/// Execute a migration plan safely.
pub async fn execute_migration(
    client: &mut Client,
    plan: &MigrationPlan,
    up_sql: &str,
    down_sql: &str,
) -> Result<(), String> {
    // 1. Acquire advisory lock
    acquire_lock(client).await?;

    let result = execute_with_lock(client, plan, up_sql, down_sql).await;

    // 6. Release advisory lock (in finally block equivalent)
    let _ = release_lock(client).await;

    result
}

async fn execute_with_lock(
    client: &mut Client,
    plan: &MigrationPlan,
    up_sql: &str,
    down_sql: &str,
) -> Result<(), String> {
    // 2. Check if already applied (idempotency)
    let applied = client
        .query_opt(
            "SELECT status FROM doo_migrations WHERE id = $1",
            &[&plan.id],
        )
        .await
        .map_err(|e| format!("Failed to check migration status: {}", e))?;

    if let Some(row) = applied {
        let status: String = row.get(0);
        if status == "applied" {
            return Ok(()); // Already done
        }
        // If 'failed' or 'rolled_back', we can retry
    }

    let start_time = std::time::Instant::now();

    // Execute each DDL statement individually via simple_query.
    // This ensures each statement is fully processed and committed by
    // PostgreSQL before moving to the next — the same approach psql uses.
    let trimmed_sql = up_sql.trim();
    if let Err(e) = execute_sql_batch(client, trimmed_sql).await {
        eprintln!("  {}", e);
        let _ = history::record_failure(client, &plan.id, &plan.checksum, up_sql).await;
        return Err(e);
    }

    let duration = start_time.elapsed().as_millis() as i64;

    // Record success
    history::record_success(client, &plan.id, &plan.checksum, duration, up_sql, down_sql).await?;

    Ok(())
}

/// Rollback the last N applied migrations.
pub async fn rollback_migrations(client: &mut Client, count: u32) -> Result<(), String> {
    acquire_lock(client).await?;
    let res = rollback_with_lock(client, count).await;
    let _ = release_lock(client).await;
    res
}

async fn rollback_with_lock(client: &mut Client, count: u32) -> Result<(), String> {
    // Fetch last N applied migrations
    let rows = client
        .query(
            "SELECT id, down_sql FROM doo_migrations WHERE status = 'applied' ORDER BY id DESC LIMIT $1",
            &[&(count as i64)],
        )
        .await
        .map_err(|e| format!("Failed to fetch migrations to rollback: {}", e))?;

    if rows.is_empty() {
        eprintln!("No applied migrations found to rollback.");
        return Ok(());
    }

    for row in rows {
        let id: String = row.get(0);
        let down_sql: Option<String> = row.get(1);

        eprintln!("Rolling back {}...", id);

        if let Some(sql) = down_sql {
            if sql.is_empty() {
                eprintln!(
                    "  Warning: No rollback SQL for {}. Marking as rolled_back without execution.",
                    id
                );
            } else {
                // Execute each rollback statement individually via simple_query
                // (same reliable protocol used by forward migration and psql).
                if let Err(e) = execute_sql_batch(client, &sql).await {
                    let err_msg = format!("Rollback of {} failed: {}", id, e);
                    eprintln!("  {}", err_msg);
                    return Err(err_msg);
                }
            }
        } else {
            eprintln!(
                "  Error: Migration {} contains irreversible changes and cannot be rolled back.",
                id
            );
            return Err("Irreversible migration encountered".to_string());
        }

        // Update status
        client
            .execute(
                "UPDATE doo_migrations SET status = 'rolled_back' WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| format!("Failed to update migration status: {}", e))?;

        eprintln!("  Rolled back successfully.");
    }

    Ok(())
}

async fn acquire_lock(client: &Client) -> Result<(), String> {
    client
        .execute("SELECT pg_advisory_lock($1)", &[&ADVISORY_LOCK_ID])
        .await
        .map_err(|e| format!("Failed to acquire advisory lock: {}", e))?;
    Ok(())
}

async fn release_lock(client: &Client) -> Result<(), String> {
    client
        .execute("SELECT pg_advisory_unlock($1)", &[&ADVISORY_LOCK_ID])
        .await
        .map_err(|e| format!("Failed to release advisory lock: {}", e))?;
    Ok(())
}
