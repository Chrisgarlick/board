//! Board-wide settings (the `settings` table).
//!
//! - `review_mode`: on by default. The browser is then a review layer: a
//!   cookie session may look, export and restore, but not edit. Claude (MCP)
//!   and API tokens are unaffected. Only a signed-in browser can change it.

use sqlx::SqliteExecutor;

use crate::error::AppResult;

const REVIEW_MODE: &str = "review_mode";

/// Whether review mode is on. A missing row counts as on.
pub async fn review_mode<'e>(db: impl SqliteExecutor<'e>) -> AppResult<bool> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(REVIEW_MODE)
        .fetch_optional(db)
        .await?;
    Ok(row.is_none_or(|(value,)| value != "false"))
}

pub async fn set_review_mode<'e>(db: impl SqliteExecutor<'e>, on: bool) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(REVIEW_MODE)
    .bind(if on { "true" } else { "false" })
    .execute(db)
    .await?;
    Ok(())
}
