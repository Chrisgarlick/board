//! Full-text search over items and schema tables.
//!
//! `search_fts` holds one row per live item or table. The service layer keeps it in
//! step: index on create, update and undelete; remove on delete.

use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};

use crate::error::AppResult;

pub async fn index_item(
    conn: &mut SqliteConnection,
    item_id: &str,
    board_id: &str,
    title: Option<&str>,
    body: &str,
) -> AppResult<()> {
    index(conn, "item", item_id, board_id, title, body).await
}

pub async fn unindex_item(conn: &mut SqliteConnection, item_id: &str) -> AppResult<()> {
    unindex(conn, "item", item_id).await
}

/// (Re)index one entity: `item` or `table`.
pub async fn index(
    conn: &mut SqliteConnection,
    entity_kind: &str,
    id: &str,
    board_id: &str,
    title: Option<&str>,
    body: &str,
) -> AppResult<()> {
    unindex(conn, entity_kind, id).await?;
    sqlx::query(
        "INSERT INTO search_fts (entity_kind, entity_id, board_id, title, body) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(entity_kind)
    .bind(id)
    .bind(board_id)
    .bind(title.unwrap_or(""))
    .bind(body)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn unindex(conn: &mut SqliteConnection, entity_kind: &str, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM search_fts WHERE entity_kind = ? AND entity_id = ?")
        .bind(entity_kind)
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn unindex_board(conn: &mut SqliteConnection, board_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM search_fts WHERE board_id = ?")
        .bind(board_id)
        .execute(conn)
        .await?;
    Ok(())
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Hit {
    pub entity_kind: String,
    pub entity_id: String,
    pub board_id: String,
    pub board_title: String,
    pub title: String,
    /// A short excerpt of the matching text, plain (no markup).
    pub snippet: String,
}

pub async fn search(pool: &SqlitePool, q: &str, board_id: Option<&str>) -> AppResult<Vec<Hit>> {
    let Some(expr) = match_expression(q) else {
        return Ok(Vec::new());
    };
    let hits = sqlx::query_as::<_, Hit>(
        "SELECT f.entity_kind, f.entity_id, f.board_id, b.title AS board_title, f.title,
                snippet(search_fts, 4, '', '', '…', 12) AS snippet
         FROM search_fts f
         JOIN boards b ON b.id = f.board_id
         WHERE search_fts MATCH ?1 AND (?2 IS NULL OR f.board_id = ?2)
         ORDER BY rank
         LIMIT 50",
    )
    .bind(expr)
    .bind(board_id)
    .fetch_all(pool)
    .await?;
    Ok(hits)
}

/// Turn what someone typed into a safe FTS5 query: every word must match,
/// and the last word matches as a prefix (so results appear while typing).
/// Each word is quoted, so FTS5 operators and punctuation are taken literally.
fn match_expression(q: &str) -> Option<String> {
    let words: Vec<String> = q
        .split_whitespace()
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect();
    let (last, rest) = words.split_last()?;
    let mut expr = rest.join(" ");
    if !expr.is_empty() {
        expr.push(' ');
    }
    expr.push_str(last);
    expr.push('*');
    Some(expr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_quoted_prefix_queries() {
        assert_eq!(match_expression("   "), None);
        assert_eq!(match_expression("check"), Some("\"check\"*".into()));
        assert_eq!(
            match_expression("user OR \"x"),
            Some("\"user\" \"OR\" \"\"\"x\"*".into())
        );
    }
}
