//! Migration History
//!
//! Manages the `doo_migrations` table that tracks applied migrations.

use deadpool_postgres::Client;

/// Ensure the migration history table exists.
pub async fn ensure_history_table(client: &Client) -> Result<(), String> {
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS doo_migrations (
                id TEXT PRIMARY KEY,
                checksum TEXT NOT NULL,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                duration_ms BIGINT,
                status TEXT NOT NULL DEFAULT 'applied',
                up_sql TEXT,
                down_sql TEXT
            )",
            &[],
        )
        .await
        .map_err(|e| format!("Failed to create doo_migrations table: {}", e))?;

    Ok(())
}

/// Print migration status history.
pub async fn print_status(client: &Client) -> Result<(), String> {
    let rows = client
        .query(
            "SELECT id, applied_at, status, duration_ms
             FROM doo_migrations
             ORDER BY id DESC
             LIMIT 50",
            &[],
        )
        .await
        .map_err(|e| format!("Failed to query migration history: {}", e))?;

    if rows.is_empty() {
        eprintln!("No migrations have been applied.");
        return Ok(());
    }

    eprintln!("Migration History (latest first):");
    eprintln!(
        "{:<20} | {:<25} | {:<10} | {:<10}",
        "ID", "Applied At", "Status", "Duration"
    );
    eprintln!("{:-<20}-+-{:-<25}-+-{:-<10}-+-{:-<10}", "", "", "", "");

    for row in rows {
        let id: String = row.get(0);
        let applied_at: chrono::DateTime<chrono::Utc> = row.get(1);
        let status: String = row.get(2);
        let duration_ms: Option<i64> = row.get(3);

        let duration_str = match duration_ms {
            Some(ms) => format!("{}ms", ms),
            None => "-".to_string(),
        };

        let status_colored = match status.as_str() {
            "applied" => format!("\x1b[32m{}\x1b[0m", status),
            "failed" => format!("\x1b[31m{}\x1b[0m", status),
            "rolled_back" => format!("\x1b[33m{}\x1b[0m", status),
            _ => status,
        };

        eprintln!(
            "{:<20} | {:<25} | {:<19} | {:<10}",
            id,
            applied_at.format("%Y-%m-%d %H:%M:%S UTC"),
            status_colored,
            duration_str
        );
    }

    Ok(())
}

/// Mark a migration as failed.
pub async fn record_failure(
    client: &Client,
    id: &str,
    checksum: &str,
    up_sql: &str,
) -> Result<(), String> {
    client
        .execute(
            "INSERT INTO doo_migrations (id, checksum, status, up_sql)
             VALUES ($1, $2, 'failed', $3)
             ON CONFLICT (id) DO UPDATE SET status = 'failed'",
            &[&id, &checksum, &up_sql],
        )
        .await
        .map_err(|e| format!("Failed to record migration failure: {}", e))?;

    Ok(())
}

/// Record successful migration.
pub async fn record_success(
    client: &Client,
    id: &str,
    checksum: &str,
    duration_ms: i64,
    up_sql: &str,
    down_sql: &str,
) -> Result<(), String> {
    client
        .execute(
            "INSERT INTO doo_migrations (id, checksum, duration_ms, status, up_sql, down_sql)
             VALUES ($1, $2, $3, 'applied', $4, $5)
             ON CONFLICT (id) DO UPDATE SET
                 status = 'applied',
                 duration_ms = $3",
            &[&id, &checksum, &duration_ms, &up_sql, &down_sql],
        )
        .await
        .map_err(|e| format!("Failed to record migration success: {}", e))?;

    Ok(())
}
