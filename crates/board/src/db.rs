//! SQLite pool setup and the write-transaction helper (spec section 4.2).

use std::path::Path;
use std::str::FromStr as _;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::{Sqlite, Transaction};

pub type Tx = Transaction<'static, Sqlite>;

/// Open (creating if needed) the database at `path` and run migrations.
pub async fn connect(path: &Path) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str("sqlite://")
        .expect("static sqlite url")
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // NORMAL is safe with WAL: a power cut can lose the last transactions
        // but never corrupts the database.
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .with_context(|| format!("opening database {}", path.display()))?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running database migrations")?;

    Ok(pool)
}

/// Start a write transaction.
///
/// `BEGIN IMMEDIATE` takes the write lock up front. A plain (deferred)
/// transaction that reads and then writes can fail straight away with
/// `SQLITE_BUSY` when another writer got in first, and `busy_timeout` doesn't
/// help in that case. Every write in Board goes through here.
pub async fn write_tx(pool: &SqlitePool) -> sqlx::Result<Tx> {
    pool.begin_with("BEGIN IMMEDIATE").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn concurrent_writers_never_hit_busy() {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("t.db")).await.unwrap();
        sqlx::query("CREATE TABLE counter (n INTEGER NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO counter (n) VALUES (0)")
            .execute(&pool)
            .await
            .unwrap();

        // Each task reads, pauses while holding the transaction, then writes:
        // the shape that fails with deferred transactions.
        let mut tasks = Vec::new();
        for _ in 0..20 {
            let pool = pool.clone();
            tasks.push(tokio::spawn(async move {
                let mut tx = write_tx(&pool).await?;
                let (n,): (i64,) = sqlx::query_as("SELECT n FROM counter")
                    .fetch_one(&mut *tx)
                    .await?;
                tokio::time::sleep(Duration::from_millis(2)).await;
                sqlx::query("UPDATE counter SET n = ?")
                    .bind(n + 1)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }

        let (n,): (i64,) = sqlx::query_as("SELECT n FROM counter")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 20, "every increment applied, none lost");
    }

    #[tokio::test]
    async fn fts5_is_available() {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("t.db")).await.unwrap();
        sqlx::query("INSERT INTO search_fts (entity_kind, entity_id, board_id, title, body) VALUES ('item', 'a', 'b', 'Checkout', 'basket payment')")
            .execute(&pool)
            .await
            .unwrap();
        let (count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM search_fts WHERE search_fts MATCH 'payment'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
    }
}
