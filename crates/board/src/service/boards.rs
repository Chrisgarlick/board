//! Boards: a flat list for v1 (spec section 15, question 3).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqliteConnection;

use crate::content::MAX_TITLE_CHARS;
use crate::error::{AppError, AppResult};
use crate::service::items::{self, Item};
use crate::service::search;
use crate::types::{Ctx, new_id};

const MAX_DESCRIPTION_CHARS: usize = 2_000;
const MAX_STACK_CHARS: usize = 200;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Board {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub stack: Option<String>,
    pub schema_version: i64,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct BoardSummary {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub board: Board,
    pub item_count: i64,
}

/// A board with everything on it, for the canvas.
#[derive(Debug, Clone, Serialize)]
pub struct FullBoard {
    pub board: Board,
    pub items: Vec<Item>,
    /// Arrive in phase 2.
    pub connectors: Vec<Value>,
    /// Arrive in phase 5.
    pub tables: Vec<Value>,
}

const COLS: &str = "b.id, b.title, b.description, b.stack, b.schema_version, b.archived, b.created_at, b.updated_at";

pub async fn list(
    conn: &mut SqliteConnection,
    q: Option<&str>,
    archived: bool,
) -> AppResult<Vec<BoardSummary>> {
    let pattern = q
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(|q| format!("%{}%", escape_like(q)));
    let sql = format!(
        "SELECT {COLS},
                (SELECT count(*) FROM items i WHERE i.board_id = b.id AND i.deleted_at IS NULL) AS item_count
         FROM boards b
         WHERE b.archived = ?1 AND (?2 IS NULL OR b.title LIKE ?2 ESCAPE '\\')
         ORDER BY b.updated_at DESC"
    );
    Ok(sqlx::query_as::<_, BoardSummary>(sqlx::AssertSqlSafe(sql))
        .bind(archived)
        .bind(pattern)
        .fetch_all(conn)
        .await?)
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub async fn get(conn: &mut SqliteConnection, id: &str) -> AppResult<Board> {
    let sql = format!("SELECT {COLS} FROM boards b WHERE b.id = ?");
    sqlx::query_as::<_, Board>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or(AppError::NotFound("board"))
}

pub async fn get_full(conn: &mut SqliteConnection, id: &str) -> AppResult<FullBoard> {
    let board = get(conn, id).await?;
    let items = items::list_live(conn, id).await?;
    Ok(FullBoard {
        board,
        items,
        connectors: Vec::new(),
        tables: Vec::new(),
    })
}

pub async fn require_exists(conn: &mut SqliteConnection, id: &str) -> AppResult<()> {
    let found: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM boards WHERE id = ?")
        .bind(id)
        .fetch_optional(conn)
        .await?;
    found.map(|_| ()).ok_or(AppError::NotFound("board"))
}

/// Mark a board as changed, so the list shows recently used boards first.
pub async fn touch(conn: &mut SqliteConnection, id: &str, ctx: Ctx) -> AppResult<()> {
    sqlx::query("UPDATE boards SET updated_at = ? WHERE id = ?")
        .bind(ctx.now.to_db())
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewBoard {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub stack: Option<String>,
}

pub async fn create(conn: &mut SqliteConnection, ctx: Ctx, new: NewBoard) -> AppResult<Board> {
    let title = check_title(&new.title)?;
    let description = check_optional("description", new.description, MAX_DESCRIPTION_CHARS)?;
    let stack = check_optional("stack", new.stack, MAX_STACK_CHARS)?;
    let id = new_id();
    let now = ctx.now.to_db();
    sqlx::query(
        "INSERT INTO boards (id, title, description, stack, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(title)
    .bind(description)
    .bind(stack)
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;
    get(conn, &id).await
}

/// A board update. Missing fields are unchanged; `null` clears description or stack.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardPatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub stack: Option<Option<String>>,
    #[serde(default)]
    pub archived: Option<bool>,
}

fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

pub async fn update(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    id: &str,
    patch: BoardPatch,
) -> AppResult<Board> {
    let current = get(conn, id).await?;
    let title = match patch.title {
        Some(t) => check_title(&t)?,
        None => current.title,
    };
    let description = match patch.description {
        Some(d) => check_optional("description", d, MAX_DESCRIPTION_CHARS)?,
        None => current.description,
    };
    let stack = match patch.stack {
        Some(s) => check_optional("stack", s, MAX_STACK_CHARS)?,
        None => current.stack,
    };
    let archived = patch.archived.unwrap_or(current.archived);
    sqlx::query(
        "UPDATE boards SET title = ?, description = ?, stack = ?, archived = ?, updated_at = ? WHERE id = ?",
    )
    .bind(title)
    .bind(description)
    .bind(stack)
    .bind(archived)
    .bind(ctx.now.to_db())
    .bind(id)
    .execute(&mut *conn)
    .await?;
    get(conn, id).await
}

/// Delete a board and everything on it. This is a hard delete (spec 4.1).
pub async fn delete(conn: &mut SqliteConnection, id: &str) -> AppResult<()> {
    require_exists(conn, id).await?;
    search::unindex_board(conn, id).await?;
    sqlx::query("DELETE FROM boards WHERE id = ?")
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}

fn check_title(title: &str) -> AppResult<String> {
    let title = title.trim();
    if title.is_empty() {
        return Err(AppError::invalid("title", "a board needs a title"));
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(AppError::invalid(
            "title",
            format!("titles are at most {MAX_TITLE_CHARS} characters"),
        ));
    }
    Ok(title.to_string())
}

fn check_optional(field: &str, value: Option<String>, max: usize) -> AppResult<Option<String>> {
    let value = value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if let Some(v) = &value
        && v.chars().count() > max
    {
        return Err(AppError::invalid(
            field,
            format!("at most {max} characters"),
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_patterns_are_escaped() {
        assert_eq!(escape_like("50%_off\\"), "50\\%\\_off\\\\");
    }
}
