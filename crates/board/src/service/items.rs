//! Items: everything on the canvas (spec sections 4 and 5).
//!
//! Every function takes an open write transaction (`&mut SqliteConnection`
//! from [`crate::db::write_tx`]) so single requests and batches share one
//! code path. The caller commits.

use chrono::TimeDelta;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqliteConnection;

use crate::content::{self, CardType, Kind, PageRef, Refs};
use crate::error::{AppError, AppResult};
use crate::service::placement::{self, Rect};
use crate::service::{boards, frames, search, tables};
use crate::types::{Ctx, new_id};

/// Revisions by the same actor within this window are merged (spec 4.3).
const COALESCE_WINDOW: TimeDelta = TimeDelta::minutes(5);
/// Revisions kept per item.
const KEEP_REVISIONS: i64 = 20;
/// Coordinates beyond this are almost certainly a client bug.
const MAX_COORD: f64 = 10_000_000.0;
const MAX_SIZE: f64 = 100_000.0;
/// A sanity limit, not a design target (spec 5.5).
pub const MAX_ITEMS_PER_BOARD: i64 = 5_000;

/// Every column except the cached SVG, which only the exporter reads.
const COLS: &str = "id, board_id, kind, card_type, frame_id, title, content, style, x, y, w, h, z, \
                    locked, version, created_by, updated_by, content_updated_at, deleted_at, \
                    deleted_by, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
struct Row {
    id: String,
    board_id: String,
    kind: String,
    card_type: Option<String>,
    frame_id: Option<String>,
    title: Option<String>,
    content: String,
    style: Option<String>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    z: i64,
    locked: bool,
    version: i64,
    created_by: String,
    updated_by: String,
    content_updated_at: String,
    deleted_at: Option<String>,
    deleted_by: Option<String>,
    created_at: String,
    updated_at: String,
}

/// An item as the API returns it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub id: String,
    pub board_id: String,
    pub kind: String,
    pub card_type: Option<String>,
    pub frame_id: Option<String>,
    pub title: Option<String>,
    pub content: Value,
    pub style: Option<Value>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub z: i64,
    pub locked: bool,
    pub version: i64,
    pub created_by: String,
    pub updated_by: String,
    pub content_updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Row {
    fn into_item(self) -> AppResult<Item> {
        let parse = |s: &str| -> AppResult<Value> {
            serde_json::from_str(s).map_err(|e| AppError::Internal(e.into()))
        };
        Ok(Item {
            content: parse(&self.content)?,
            style: self.style.as_deref().map(parse).transpose()?,
            id: self.id,
            board_id: self.board_id,
            kind: self.kind,
            card_type: self.card_type,
            frame_id: self.frame_id,
            title: self.title,
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
            z: self.z,
            locked: self.locked,
            version: self.version,
            created_by: self.created_by,
            updated_by: self.updated_by,
            content_updated_at: self.content_updated_at,
            deleted_at: self.deleted_at,
            deleted_by: self.deleted_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

fn to_value(item: &Item) -> Value {
    serde_json::to_value(item).unwrap_or(Value::Null)
}

// ---------- Reads ----------

async fn row(conn: &mut SqliteConnection, id: &str) -> AppResult<Option<Row>> {
    let sql = format!("SELECT {COLS} FROM items WHERE id = ?");
    Ok(sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(conn)
        .await?)
}

async fn live_row(conn: &mut SqliteConnection, id: &str) -> AppResult<Row> {
    match row(conn, id).await? {
        Some(r) if r.deleted_at.is_none() => Ok(r),
        _ => Err(AppError::NotFound("item")),
    }
}

/// A live item.
pub async fn get(conn: &mut SqliteConnection, id: &str) -> AppResult<Item> {
    live_row(conn, id).await?.into_item()
}

/// Live items on a board, bottom to top.
pub async fn list_live(conn: &mut SqliteConnection, board_id: &str) -> AppResult<Vec<Item>> {
    let sql = format!(
        "SELECT {COLS} FROM items WHERE board_id = ? AND deleted_at IS NULL ORDER BY z, id"
    );
    sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(sql))
        .bind(board_id)
        .fetch_all(conn)
        .await?
        .into_iter()
        .map(Row::into_item)
        .collect()
}

/// Deleted (trashed) items on a board, most recently deleted first.
pub async fn list_trash(conn: &mut SqliteConnection, board_id: &str) -> AppResult<Vec<Item>> {
    let sql = format!(
        "SELECT {COLS} FROM items WHERE board_id = ? AND deleted_at IS NOT NULL ORDER BY deleted_at DESC"
    );
    sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(sql))
        .bind(board_id)
        .fetch_all(conn)
        .await?
        .into_iter()
        .map(Row::into_item)
        .collect()
}

/// The board an item (live or trashed) belongs to.
pub async fn board_of(conn: &mut SqliteConnection, id: &str) -> AppResult<String> {
    row(conn, id)
        .await?
        .map(|r| r.board_id)
        .ok_or(AppError::NotFound("item"))
}

/// What an item's content may refer to, loaded only for the kinds that
/// refer to anything (schema, sitemap and feature cards, and shapes).
async fn refs(
    conn: &mut SqliteConnection,
    board_id: &str,
    kind: Kind,
    card_type: Option<CardType>,
    item_id: Option<&str>,
    current: Option<Value>,
) -> AppResult<Refs> {
    let mut refs = Refs {
        item_id: item_id.map(str::to_string),
        current,
        ..Refs::default()
    };
    if matches!(card_type, Some(CardType::Schema | CardType::Feature)) {
        refs.tables = tables::names(conn, board_id).await?;
    }
    if kind == Kind::Shape || matches!(card_type, Some(CardType::Sitemap | CardType::Feature)) {
        refs.pages = pages(conn, board_id).await?;
    }
    Ok(refs)
}

/// Every page on a board's live sitemap cards, in card order.
pub async fn pages(conn: &mut SqliteConnection, board_id: &str) -> AppResult<Vec<PageRef>> {
    let cards: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, content FROM items
         WHERE board_id = ? AND card_type = 'sitemap' AND deleted_at IS NULL ORDER BY y, x, id",
    )
    .bind(board_id)
    .fetch_all(conn)
    .await?;
    let mut pages = Vec::new();
    for (item_id, content) in cards {
        let Ok(c) = serde_json::from_str::<content::SitemapContent>(&content) else {
            continue;
        };
        for p in c.root.walk() {
            pages.push(PageRef {
                id: p.id.clone(),
                slug: p.slug.clone(),
                title: p.title.clone(),
                item_id: item_id.clone(),
            });
        }
    }
    Ok(pages)
}

// ---------- Create ----------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewItem {
    pub kind: String,
    #[serde(default)]
    pub card_type: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    pub content: Value,
    #[serde(default)]
    pub style: Option<Value>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub w: Option<f64>,
    #[serde(default)]
    pub h: Option<f64>,
    #[serde(default)]
    pub z: Option<i64>,
}

/// Create an item. Returns it, plus the ids of other items whose frame
/// membership changed as a result (when the new item is a frame).
pub async fn create(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    board_id: &str,
    new: NewItem,
) -> AppResult<(Item, Vec<String>)> {
    boards::require_exists(conn, board_id).await?;
    require_room(conn, board_id, 1).await?;

    let kind = Kind::parse(&new.kind)?;
    let card_type = new.card_type.as_deref().map(CardType::parse).transpose()?;
    let refs = refs(conn, board_id, kind, card_type, None, None).await?;
    let valid = content::validate(
        kind,
        card_type,
        new.title.as_deref(),
        &new.content,
        new.style.as_ref(),
        &refs,
    )?;

    let (default_w, default_h) = valid.default_size;
    let w = new.w.unwrap_or(default_w);
    let h = new.h.unwrap_or(default_h);
    check_size("w", w)?;
    check_size("h", h)?;

    let (x, y) = match (new.x, new.y) {
        (Some(x), Some(y)) => (x, y),
        (None, None) => {
            let existing = rects(conn, board_id).await?;
            placement::place(&existing, w, h)
        }
        _ => {
            return Err(AppError::invalid(
                if new.x.is_none() { "x" } else { "y" },
                "give both x and y, or neither to auto-place",
            ));
        }
    };
    check_coord("x", x)?;
    check_coord("y", y)?;

    let z = match new.z {
        Some(z) => z,
        None => top_z(conn, board_id).await? + 1,
    };

    let id = new_id();
    let now = ctx.now.to_db();
    sqlx::query(
        "INSERT INTO items (id, board_id, kind, card_type, title, content, style, x, y, w, h, z,
                            created_by, updated_by, content_updated_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(board_id)
    .bind(kind.as_str())
    .bind(valid.card_type.map(CardType::as_str))
    .bind(&valid.title)
    .bind(&valid.content)
    .bind(&valid.style)
    .bind(x)
    .bind(y)
    .bind(w)
    .bind(h)
    .bind(z)
    .bind(ctx.actor.as_str())
    .bind(ctx.actor.as_str())
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;

    record_revision(
        conn,
        ctx,
        &id,
        valid.title.as_deref(),
        &valid.content,
        valid.style.as_deref(),
        1,
    )
    .await?;
    search::index_item(
        conn,
        &id,
        board_id,
        valid.title.as_deref(),
        &valid.search_body,
    )
    .await?;
    let affected = if kind == Kind::Frame {
        frames::recompute_board(conn, board_id).await?
    } else {
        frames::recompute_item(conn, &id).await?;
        Vec::new()
    };
    boards::touch(conn, board_id, ctx).await?;
    Ok((get(conn, &id).await?, affected))
}

/// Refuse to grow a board past [`MAX_ITEMS_PER_BOARD`].
pub async fn require_room(
    conn: &mut SqliteConnection,
    board_id: &str,
    adding: i64,
) -> AppResult<()> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM items WHERE board_id = ? AND deleted_at IS NULL")
            .bind(board_id)
            .fetch_one(conn)
            .await?;
    if count + adding > MAX_ITEMS_PER_BOARD {
        return Err(AppError::State {
            code: "board_full",
            message: format!("a board holds at most {MAX_ITEMS_PER_BOARD} items"),
        });
    }
    Ok(())
}

async fn rects(conn: &mut SqliteConnection, board_id: &str) -> AppResult<Vec<Rect>> {
    let rows: Vec<(f64, f64, f64, f64)> =
        sqlx::query_as("SELECT x, y, w, h FROM items WHERE board_id = ? AND deleted_at IS NULL")
            .bind(board_id)
            .fetch_all(conn)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(x, y, w, h)| Rect { x, y, w, h })
        .collect())
}

async fn top_z(conn: &mut SqliteConnection, board_id: &str) -> AppResult<i64> {
    let (z,): (Option<i64>,) =
        sqlx::query_as("SELECT max(z) FROM items WHERE board_id = ? AND deleted_at IS NULL")
            .bind(board_id)
            .fetch_one(conn)
            .await?;
    Ok(z.unwrap_or(0))
}

fn check_coord(field: &str, v: f64) -> AppResult<()> {
    if !v.is_finite() || v.abs() > MAX_COORD {
        return Err(AppError::invalid(field, "coordinate is out of range"));
    }
    Ok(())
}

fn check_size(field: &str, v: f64) -> AppResult<()> {
    if !v.is_finite() || v <= 0.0 || v > MAX_SIZE {
        return Err(AppError::invalid(
            field,
            format!("size must be between 0 and {MAX_SIZE}"),
        ));
    }
    Ok(())
}

// ---------- Update content ----------

/// A content update. Fields left out are unchanged; `style: null` clears the style.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemPatch {
    pub version: i64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    pub style: Option<Option<Value>>,
}

/// Distinguish "field missing" (`None`) from "field is null" (`Some(None)`).
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<Value>>, D::Error> {
    Option::<Value>::deserialize(d).map(Some)
}

pub async fn update(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    id: &str,
    patch: ItemPatch,
) -> AppResult<Item> {
    let current = live_row(conn, id).await?;
    if patch.version != current.version {
        return Err(AppError::Conflict {
            current: to_value(&current.into_item()?),
        });
    }

    let kind = Kind::parse(&current.kind)?;
    let card_type = current
        .card_type
        .as_deref()
        .map(CardType::parse)
        .transpose()?;
    let current_content: Value =
        serde_json::from_str(&current.content).map_err(|e| AppError::Internal(e.into()))?;
    let current_style: Option<Value> = current
        .style
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| AppError::Internal(e.into()))?;

    let title = patch.title.or(current.title.clone());
    let refs = refs(
        conn,
        &current.board_id,
        kind,
        card_type,
        Some(id),
        Some(current_content.clone()),
    )
    .await?;
    let content = patch.content.unwrap_or(current_content);
    let style = match patch.style {
        Some(style) => style,
        None => current_style,
    };
    let valid = content::validate(
        kind,
        card_type,
        title.as_deref(),
        &content,
        style.as_ref(),
        &refs,
    )?;

    // Nothing changed: don't bump the version or write a revision.
    if valid.title == current.title
        && valid.content == current.content
        && valid.style == current.style
    {
        return current.into_item();
    }

    let new_version = current.version + 1;
    let now = ctx.now.to_db();
    let updated = sqlx::query(
        "UPDATE items SET title = ?, content = ?, style = ?, version = ?, updated_by = ?,
                          content_updated_at = ?, updated_at = ?
         WHERE id = ? AND version = ? AND deleted_at IS NULL",
    )
    .bind(&valid.title)
    .bind(&valid.content)
    .bind(&valid.style)
    .bind(new_version)
    .bind(ctx.actor.as_str())
    .bind(&now)
    .bind(&now)
    .bind(id)
    .bind(current.version)
    .execute(&mut *conn)
    .await?;
    if updated.rows_affected() == 0 {
        // Can't happen inside BEGIN IMMEDIATE, but never write blind.
        return Err(AppError::Conflict {
            current: to_value(&get(conn, id).await?),
        });
    }

    record_revision(
        conn,
        ctx,
        id,
        valid.title.as_deref(),
        &valid.content,
        valid.style.as_deref(),
        new_version,
    )
    .await?;
    search::index_item(
        conn,
        id,
        &current.board_id,
        valid.title.as_deref(),
        &valid.search_body,
    )
    .await?;
    boards::touch(conn, &current.board_id, ctx).await?;
    get(conn, id).await
}

// ---------- Geometry ----------

/// A geometry update. No version check: last write wins (spec section 6).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub w: Option<f64>,
    #[serde(default)]
    pub h: Option<f64>,
    #[serde(default)]
    pub z: Option<i64>,
    #[serde(default)]
    pub locked: Option<bool>,
}

/// Move, resize, restack or (un)lock an item. Returns it, plus the ids of
/// other items that changed as a result: a frame's members move with it, and
/// any move can change frame membership (spec 4.5).
pub async fn set_geometry(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    id: &str,
    g: Geometry,
) -> AppResult<(Item, Vec<String>)> {
    let current = live_row(conn, id).await?;

    let x = g.x.unwrap_or(current.x);
    let y = g.y.unwrap_or(current.y);
    let w = g.w.unwrap_or(current.w);
    let h = g.h.unwrap_or(current.h);
    let z = g.z.unwrap_or(current.z);
    let locked = g.locked.unwrap_or(current.locked);
    check_coord("x", x)?;
    check_coord("y", y)?;
    check_size("w", w)?;
    check_size("h", h)?;

    let moves = x != current.x || y != current.y || w != current.w || h != current.h;
    // A locked item can't move until it's unlocked; unlocking and moving in
    // one request is fine.
    if moves && locked {
        return Err(AppError::State {
            code: "locked",
            message: "this item is locked; unlock it to move or resize it".into(),
        });
    }

    sqlx::query(
        "UPDATE items SET x = ?, y = ?, w = ?, h = ?, z = ?, locked = ?, updated_at = ? WHERE id = ?",
    )
    .bind(x)
    .bind(y)
    .bind(w)
    .bind(h)
    .bind(z)
    .bind(locked)
    .bind(ctx.now.to_db())
    .bind(id)
    .execute(&mut *conn)
    .await?;

    let mut affected = Vec::new();
    if current.kind == "frame" {
        // Members move with their frame, locked or not.
        let (dx, dy) = (x - current.x, y - current.y);
        if dx != 0.0 || dy != 0.0 {
            let members = frames::members(conn, id).await?;
            sqlx::query(
                "UPDATE items SET x = x + ?, y = y + ?, updated_at = ? WHERE frame_id = ? AND deleted_at IS NULL",
            )
            .bind(dx)
            .bind(dy)
            .bind(ctx.now.to_db())
            .bind(id)
            .execute(&mut *conn)
            .await?;
            affected.extend(members);
        }
        if moves || z != current.z {
            affected.extend(frames::recompute_board(conn, &current.board_id).await?);
        }
    } else if moves {
        frames::recompute_item(conn, id).await?;
    }
    affected.sort();
    affected.dedup();

    boards::touch(conn, &current.board_id, ctx).await?;
    Ok((get(conn, id).await?, affected))
}

// ---------- Delete and undelete ----------

/// What a delete did.
#[derive(Debug, Default, Serialize)]
pub struct Deleted {
    /// Items moved to the trash: the item, plus a frame's members when
    /// deleting with contents.
    pub deleted: Vec<String>,
    /// Live items whose frame membership changed (a frame's released members).
    pub affected: Vec<String>,
}

/// Move an item to the trash. Its connectors are hidden until it's restored.
/// Deleting a frame releases its members unless `with_contents` is set, in
/// which case they go to the trash too (spec 4.5).
pub async fn delete(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    id: &str,
    with_contents: bool,
) -> AppResult<Deleted> {
    let current = live_row(conn, id).await?;
    let mut result = Deleted {
        deleted: vec![id.to_string()],
        affected: Vec::new(),
    };
    if current.kind == "frame" && with_contents {
        result.deleted.extend(frames::members(conn, id).await?);
    }

    let now = ctx.now.to_db();
    for item in &result.deleted {
        sqlx::query("UPDATE items SET deleted_at = ?, deleted_by = ?, updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(ctx.actor.as_str())
            .bind(&now)
            .bind(item)
            .execute(&mut *conn)
            .await?;
        search::unindex_item(conn, item).await?;
    }
    if current.kind == "frame" {
        result.affected = frames::recompute_board(conn, &current.board_id).await?;
    }
    boards::touch(conn, &current.board_id, ctx).await?;
    Ok(result)
}

/// Bring an item back from the trash. Returns it, plus the ids of items whose
/// frame membership changed (restoring a frame re-captures what's inside it).
pub async fn undelete(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    id: &str,
) -> AppResult<(Item, Vec<String>)> {
    let current = row(conn, id).await?.ok_or(AppError::NotFound("item"))?;
    if current.deleted_at.is_none() {
        return Err(AppError::State {
            code: "not_deleted",
            message: "this item isn't in the trash".into(),
        });
    }
    sqlx::query(
        "UPDATE items SET deleted_at = NULL, deleted_by = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(ctx.now.to_db())
    .bind(id)
    .execute(&mut *conn)
    .await?;

    let kind = Kind::parse(&current.kind)?;
    let card_type = current
        .card_type
        .as_deref()
        .map(CardType::parse)
        .transpose()?;
    let content: Value =
        serde_json::from_str(&current.content).map_err(|e| AppError::Internal(e.into()))?;
    let mut refs = refs(conn, &current.board_id, kind, card_type, Some(id), None).await?;
    refs.trusted = true;
    let valid = content::validate(
        kind,
        card_type,
        current.title.as_deref(),
        &content,
        None,
        &refs,
    )?;
    search::index_item(
        conn,
        id,
        &current.board_id,
        current.title.as_deref(),
        &valid.search_body,
    )
    .await?;
    require_room(conn, &current.board_id, 0).await?;
    let affected = if current.kind == "frame" {
        frames::recompute_board(conn, &current.board_id).await?
    } else {
        frames::recompute_item(conn, id).await?;
        Vec::new()
    };
    boards::touch(conn, &current.board_id, ctx).await?;
    Ok((get(conn, id).await?, affected))
}

/// Hard-delete items that have been in the trash longer than `days`.
/// Their revisions go with them (ON DELETE CASCADE).
pub async fn purge_trash(conn: &mut SqliteConnection, ctx: Ctx, days: i64) -> AppResult<u64> {
    let cutoff = ctx.now.minus(TimeDelta::days(days)).to_db();
    let result = sqlx::query("DELETE FROM items WHERE deleted_at IS NOT NULL AND deleted_at < ?")
        .bind(cutoff)
        .execute(conn)
        .await?;
    Ok(result.rows_affected())
}

// ---------- Revisions ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Revision {
    pub id: i64,
    pub version: i64,
    pub title: Option<String>,
    #[sqlx(json)]
    pub content: Value,
    #[sqlx(json(nullable))]
    pub style: Option<Value>,
    pub changed_by: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Record the state after a change, merging into the newest revision when
/// the same actor changed the item within the last 5 minutes (spec 4.3).
async fn record_revision(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    item_id: &str,
    title: Option<&str>,
    content: &str,
    style: Option<&str>,
    version: i64,
) -> AppResult<()> {
    let now = ctx.now.to_db();
    let window_start = ctx.now.minus(COALESCE_WINDOW).to_db();

    let newest: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT id, changed_by, updated_at FROM item_revisions WHERE item_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(item_id)
    .fetch_optional(&mut *conn)
    .await?;

    if let Some((rev_id, changed_by, updated_at)) = newest
        && changed_by == ctx.actor.as_str()
        && updated_at > window_start
    {
        sqlx::query(
            "UPDATE item_revisions SET title = ?, content = ?, style = ?, version = ?, updated_at = ? WHERE id = ?",
        )
        .bind(title)
        .bind(content)
        .bind(style)
        .bind(version)
        .bind(&now)
        .bind(rev_id)
        .execute(&mut *conn)
        .await?;
        return Ok(());
    }

    sqlx::query(
        "INSERT INTO item_revisions (item_id, title, content, style, version, changed_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(item_id)
    .bind(title)
    .bind(content)
    .bind(style)
    .bind(version)
    .bind(ctx.actor.as_str())
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;

    sqlx::query(
        "DELETE FROM item_revisions WHERE item_id = ?1 AND id NOT IN
           (SELECT id FROM item_revisions WHERE item_id = ?1 ORDER BY id DESC LIMIT ?2)",
    )
    .bind(item_id)
    .bind(KEEP_REVISIONS)
    .execute(conn)
    .await?;
    Ok(())
}

/// Revisions of an item, newest first.
pub async fn revisions(conn: &mut SqliteConnection, item_id: &str) -> AppResult<Vec<Revision>> {
    // Trashed items keep their history, so only require that the item exists.
    board_of(conn, item_id).await?;
    Ok(sqlx::query_as::<_, Revision>(
        "SELECT id, version, title, content, style, changed_by, created_at, updated_at
         FROM item_revisions WHERE item_id = ? ORDER BY id DESC",
    )
    .bind(item_id)
    .fetch_all(conn)
    .await?)
}

/// Restore a revision: an ordinary versioned update with the revision's
/// title, content and style, so the restore can itself be undone.
pub async fn restore(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    item_id: &str,
    revision_id: i64,
    version: i64,
) -> AppResult<Item> {
    let rev: Option<(Option<String>, String, Option<String>)> = sqlx::query_as(
        "SELECT title, content, style FROM item_revisions WHERE id = ? AND item_id = ?",
    )
    .bind(revision_id)
    .bind(item_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (title, content, style) = rev.ok_or(AppError::NotFound("revision"))?;
    let parse = |s: &str| -> AppResult<Value> {
        serde_json::from_str(s).map_err(|e| AppError::Internal(e.into()))
    };
    update(
        conn,
        ctx,
        item_id,
        ItemPatch {
            version,
            title,
            content: Some(parse(&content)?),
            style: Some(style.as_deref().map(parse).transpose()?),
        },
    )
    .await
}
