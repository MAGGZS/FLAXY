use std::path::Path;

use sha2::{Digest, Sha384};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::Result;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Abre o SQLite em modo WAL e aplica as migrações embutidas no binário.
pub async fn connect(path: &Path) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new().max_connections(8).connect_with(opts).await?;
    heal_line_endings(&pool, &MIGRATOR).await?;
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}

/// The sqlx checksum is a SHA-384 of the raw SQL bytes, so the same migration
/// checked out with CRLF instead of LF (git `core.autocrlf`) looks "modified"
/// and the app refuses to open the database. When a stored checksum matches
/// the embedded migration with the other line ending, the SQL that ran was
/// identical: rewrite the stored checksum to the embedded one.
async fn heal_line_endings(pool: &SqlitePool, migrator: &Migrator) -> Result<()> {
    let table: Option<(String,)> = sqlx::query_as("SELECT name FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'")
        .fetch_optional(pool)
        .await?;
    if table.is_none() {
        return Ok(());
    }
    let applied: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations").fetch_all(pool).await?;
    for (version, stored) in applied {
        let Some(m) = migrator.iter().find(|m| m.version == version) else { continue };
        if stored.as_slice() == m.checksum.as_ref() {
            continue;
        }
        let lf = m.sql.replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");
        if [lf, crlf].iter().any(|s| Sha384::digest(s.as_bytes()).as_slice() == stored.as_slice()) {
            tracing::info!("migration {version}: checksum differs only by line endings, updating");
            sqlx::query("UPDATE _sqlx_migrations SET checksum=? WHERE version=?").bind(m.checksum.as_ref()).bind(version).execute(pool).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Database migrated from LF files opens with the same files in CRLF.
    #[tokio::test]
    async fn heals_line_ending_checksums() {
        let dir = std::env::temp_dir().join(format!("flaxy-db-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let pool = connect(&path).await.unwrap();
        // Simulate checksums recorded from the other line ending.
        for m in MIGRATOR.iter() {
            let other = if m.sql.contains("\r\n") { m.sql.replace("\r\n", "\n") } else { m.sql.replace('\n', "\r\n") };
            sqlx::query("UPDATE _sqlx_migrations SET checksum=? WHERE version=?")
                .bind(Sha384::digest(other.as_bytes()).to_vec())
                .bind(m.version)
                .execute(&pool)
                .await
                .unwrap();
        }
        assert!(MIGRATOR.run(&pool).await.is_err());
        pool.close().await;
        let pool = connect(&path).await.unwrap();
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
