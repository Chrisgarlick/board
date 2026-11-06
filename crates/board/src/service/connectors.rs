//! Connectors: arrows between any two items on a board (spec sections 4.1 and 5.2).
//!
//! A connector is visible while it isn't deleted and both of its items are
//! live. Deleting an item hides its connectors without touching them, so
//! restoring the item brings them back.

use chrono::TimeDelta;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqliteConnection;

use crate::content::Color;
use crate::error::{AppError, AppResult};
use crate::service::boards;
use crate::types::{Ctx, new_id};

const MAX_LABEL_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    Auto,
    Top,
    Right,
    Bottom,
    Left,
}

impl Anchor {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Top => "top",
            Self::Right => "right",
            Self::Bottom => "bottom",
            Self::Left => "left",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Line {
    Solid,
    Dashed,
    Dotted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Head {
    None,
    Arrow,
    OpenArrow,
    Dot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Routing {
    Straight,
    Elbow,
    Curved,
}

/// Connector style. Every field is optional; the defaults are a solid grey
/// elbow line with an arrow at the end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConnectorStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Line>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Head>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Head>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<Routing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
}

#[derive(Debug, Clone, Serialize, PartialEq, sqlx::FromRow)]
pub struct Connector {
    pub id: String,
    pub board_id: String,
    pub from_item: String,
    pub to_item: String,
    pub from_anchor: String,
    pub to_anchor: String,
    pub label: Option<String>,
    #[sqlx(json(nullable))]
    pub style: Option<Value>,
    pub created_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

const COLS: &str = "c.id, c.board_id, c.from_item, c.to_item, c.from_anchor, c.to_anchor, c.label, \
                    c.style, c.created_by, c.deleted_at, c.deleted_by, c.created_at, c.updated_at";

async fn row(conn: &mut SqliteConnection, id: &str) -> AppResult<Option<Connector>> {
    let sql = format!("SELECT {COLS} FROM connectors c WHERE c.id = ?");
    Ok(sqlx::query_as::<_, Connector>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(conn)
        .await?)
}

/// A visible connector: not deleted, and both ends live.
pub async fn get(conn: &mut SqliteConnection, id: &str) -> AppResult<Connector> {
    let c = row(conn, id)
        .await?
        .ok_or(AppError::NotFound("connector"))?;
    if c.deleted_at.is_some() || !ends_live(conn, &c.from_item, &c.to_item).await? {
        return Err(AppError::NotFound("connector"));
    }
    Ok(c)
}

/// The board a connector (visible or not) belongs to.
pub async fn board_of(conn: &mut SqliteConnection, id: &str) -> AppResult<String> {
    row(conn, id)
        .await?
        .map(|c| c.board_id)
        .ok_or(AppError::NotFound("connector"))
}

/// Visible connectors on a board.
pub async fn list_live(conn: &mut SqliteConnection, board_id: &str) -> AppResult<Vec<Connector>> {
    let sql = format!(
        "SELECT {COLS} FROM connectors c
         JOIN items f ON f.id = c.from_item AND f.deleted_at IS NULL
         JOIN items t ON t.id = c.to_item AND t.deleted_at IS NULL
         WHERE c.board_id = ? AND c.deleted_at IS NULL
         ORDER BY c.id"
    );
    Ok(sqlx::query_as::<_, Connector>(sqlx::AssertSqlSafe(sql))
        .bind(board_id)
        .fetch_all(conn)
        .await?)
}

/// Connectors deleted on their own (not just hidden by a deleted item).
pub async fn list_trash(conn: &mut SqliteConnection, board_id: &str) -> AppResult<Vec<Connector>> {
    let sql = format!(
        "SELECT {COLS} FROM connectors c WHERE c.board_id = ? AND c.deleted_at IS NOT NULL ORDER BY c.deleted_at DESC"
    );
    Ok(sqlx::query_as::<_, Connector>(sqlx::AssertSqlSafe(sql))
        .bind(board_id)
        .fetch_all(conn)
        .await?)
}

async fn ends_live(conn: &mut SqliteConnection, from: &str, to: &str) -> AppResult<bool> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM items WHERE id IN (?, ?) AND deleted_at IS NULL")
            .bind(from)
            .bind(to)
            .fetch_one(conn)
            .await?;
    Ok(n == 2)
}

/// Check that `item` is a live item on `board_id`.
async fn require_end(
    conn: &mut SqliteConnection,
    board_id: &str,
    field: &str,
    item: &str,
) -> AppResult<()> {
    let found: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM items WHERE id = ? AND board_id = ? AND deleted_at IS NULL")
            .bind(item)
            .bind(board_id)
            .fetch_optional(conn)
            .await?;
    if found.is_none() {
        return Err(AppError::invalid(
            field,
            "connectors join two live items on the same board",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewConnector {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub from_anchor: Option<Anchor>,
    #[serde(default)]
    pub to_anchor: Option<Anchor>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub style: Option<Value>,
}

pub async fn create(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    board_id: &str,
    new: NewConnector,
) -> AppResult<Connector> {
    boards::require_exists(conn, board_id).await?;
    if new.from == new.to {
        return Err(AppError::invalid(
            "to",
            "a connector can't join an item to itself",
        ));
    }
    require_end(conn, board_id, "from", &new.from).await?;
    require_end(conn, board_id, "to", &new.to).await?;
    let label = check_label(new.label)?;
    let style = check_style(new.style.as_ref())?;

    let id = new_id();
    let now = ctx.now.to_db();
    sqlx::query(
        "INSERT INTO connectors (id, board_id, from_item, to_item, from_anchor, to_anchor, label, style,
                                 created_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(board_id)
    .bind(&new.from)
    .bind(&new.to)
    .bind(new.from_anchor.unwrap_or(Anchor::Auto).as_str())
    .bind(new.to_anchor.unwrap_or(Anchor::Auto).as_str())
    .bind(label)
    .bind(style)
    .bind(ctx.actor.as_str())
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;
    boards::touch(conn, board_id, ctx).await?;
    get(conn, &id).await
}

/// A connector update. Missing fields are unchanged; `null` clears label or
/// style. Last write wins (spec section 6).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorPatch {
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub from_anchor: Option<Anchor>,
    #[serde(default)]
    pub to_anchor: Option<Anchor>,
    #[serde(default, deserialize_with = "present_string")]
    pub label: Option<Option<String>>,
    #[serde(default, deserialize_with = "present_value")]
    pub style: Option<Option<Value>>,
}

fn present_string<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

fn present_value<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<Value>>, D::Error> {
    Option::<Value>::deserialize(d).map(Some)
}

pub async fn update(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    id: &str,
    patch: ConnectorPatch,
) -> AppResult<Connector> {
    let current = get(conn, id).await?;
    let from = patch.from.unwrap_or(current.from_item.clone());
    let to = patch.to.unwrap_or(current.to_item.clone());
    if from == to {
        return Err(AppError::invalid(
            "to",
            "a connector can't join an item to itself",
        ));
    }
    if from != current.from_item {
        require_end(conn, &current.board_id, "from", &from).await?;
    }
    if to != current.to_item {
        require_end(conn, &current.board_id, "to", &to).await?;
    }
    let from_anchor = patch
        .from_anchor
        .map(|a| a.as_str().to_string())
        .unwrap_or(current.from_anchor);
    let to_anchor = patch
        .to_anchor
        .map(|a| a.as_str().to_string())
        .unwrap_or(current.to_anchor);
    let label = match patch.label {
        Some(l) => check_label(l)?,
        None => current.label,
    };
    let style = match patch.style {
        Some(s) => check_style(s.as_ref())?,
        None => current.style.map(|v| v.to_string()),
    };

    sqlx::query(
        "UPDATE connectors SET from_item = ?, to_item = ?, from_anchor = ?, to_anchor = ?, label = ?,
                               style = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(from)
    .bind(to)
    .bind(from_anchor)
    .bind(to_anchor)
    .bind(label)
    .bind(style)
    .bind(ctx.now.to_db())
    .bind(id)
    .execute(&mut *conn)
    .await?;
    boards::touch(conn, &current.board_id, ctx).await?;
    get(conn, id).await
}

pub async fn delete(conn: &mut SqliteConnection, ctx: Ctx, id: &str) -> AppResult<()> {
    let current = get(conn, id).await?;
    let now = ctx.now.to_db();
    sqlx::query(
        "UPDATE connectors SET deleted_at = ?, deleted_by = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(ctx.actor.as_str())
    .bind(&now)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    boards::touch(conn, &current.board_id, ctx).await?;
    Ok(())
}

pub async fn undelete(conn: &mut SqliteConnection, ctx: Ctx, id: &str) -> AppResult<Connector> {
    let current = row(conn, id)
        .await?
        .ok_or(AppError::NotFound("connector"))?;
    if current.deleted_at.is_none() {
        return Err(AppError::State {
            code: "not_deleted",
            message: "this connector isn't in the trash".into(),
        });
    }
    let same_board: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM items WHERE id IN (?, ?) AND board_id = ? AND deleted_at IS NULL",
    )
    .bind(&current.from_item)
    .bind(&current.to_item)
    .bind(&current.board_id)
    .fetch_one(&mut *conn)
    .await?;
    if same_board.0 != 2 {
        return Err(AppError::State {
            code: "ends_missing",
            message: "restore both of the items this connector joins first".into(),
        });
    }
    sqlx::query(
        "UPDATE connectors SET deleted_at = NULL, deleted_by = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(ctx.now.to_db())
    .bind(id)
    .execute(&mut *conn)
    .await?;
    boards::touch(conn, &current.board_id, ctx).await?;
    get(conn, id).await
}

/// Hard-delete connectors that have been in the trash longer than `days`.
pub async fn purge_trash(conn: &mut SqliteConnection, ctx: Ctx, days: i64) -> AppResult<u64> {
    let cutoff = ctx.now.minus(TimeDelta::days(days)).to_db();
    let result =
        sqlx::query("DELETE FROM connectors WHERE deleted_at IS NOT NULL AND deleted_at < ?")
            .bind(cutoff)
            .execute(conn)
            .await?;
    Ok(result.rows_affected())
}

fn check_label(label: Option<String>) -> AppResult<Option<String>> {
    let label = label.filter(|l| !l.trim().is_empty());
    if let Some(l) = &label
        && l.chars().count() > MAX_LABEL_CHARS
    {
        return Err(AppError::invalid(
            "label",
            format!("labels are at most {MAX_LABEL_CHARS} characters"),
        ));
    }
    Ok(label)
}

fn check_style(style: Option<&Value>) -> AppResult<Option<String>> {
    let Some(value) = style.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let style: ConnectorStyle = serde_path_to_error::deserialize(value).map_err(|err| {
        let inner = err.path().to_string();
        let path = if inner == "." {
            "style".to_string()
        } else {
            format!("style.{inner}")
        };
        AppError::invalid(path, err.into_inner().to_string())
    })?;
    if style == ConnectorStyle::default() {
        return Ok(None);
    }
    Ok(Some(
        serde_json::to_string(&style).map_err(|e| AppError::Internal(e.into()))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn style_is_validated_and_canonical() {
        assert_eq!(check_style(Some(&json!({}))).unwrap(), None);
        assert_eq!(
            check_style(Some(&json!({ "end": "open_arrow", "line": "dashed" })))
                .unwrap()
                .as_deref(),
            Some(r#"{"line":"dashed","end":"open_arrow"}"#)
        );
        let err = check_style(Some(&json!({ "routing": "zigzag" }))).unwrap_err();
        assert!(matches!(err, AppError::Invalid { ref path, .. } if path == "style.routing"));
    }

    #[test]
    fn blank_labels_are_no_label() {
        assert_eq!(check_label(Some("  ".into())).unwrap(), None);
        assert!(check_label(Some("x".repeat(201))).is_err());
    }
}
