//! The board schema's tables (spec sections 4.1 and 5.4).
//!
//! Tables are rows of their own, not item content, so foreign keys are
//! checked across the whole board and a table can't be defined twice. Every
//! create, update, delete and undelete bumps the board's `schema_version`
//! in the same transaction, which is how Design knows to refetch.
//!
//! The rules live in [`crate::schema`]; this module stores the result.

use chrono::TimeDelta;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::SqliteConnection;

use board_model::TableDefinition;

use crate::content::CardType;
use crate::error::{AppError, AppResult};
use crate::schema::{self, BoardTable, Warning};
use crate::service::items::{self, Item, ItemPatch};
use crate::service::{boards, search};
use crate::types::Ctx;

const COALESCE_WINDOW: TimeDelta = TimeDelta::minutes(5);
const KEEP_REVISIONS: i64 = 20;

const COLS: &str = "id, board_id, name, definition, version, created_by, updated_by, \
                    content_updated_at, deleted_at, deleted_by, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
struct Row {
    id: String,
    board_id: String,
    name: String,
    definition: String,
    version: i64,
    created_by: String,
    updated_by: String,
    content_updated_at: String,
    deleted_at: Option<String>,
    deleted_by: Option<String>,
    created_at: String,
    updated_at: String,
}

impl Row {
    fn definition(&self) -> AppResult<TableDefinition> {
        serde_json::from_str(&self.definition).map_err(|e| AppError::Internal(e.into()))
    }

    fn board_table(&self) -> AppResult<BoardTable> {
        Ok(BoardTable {
            id: self.id.clone(),
            name: self.name.clone(),
            definition: self.definition()?,
            live: self.deleted_at.is_none(),
        })
    }
}

/// A table as the API returns it: the definition carries names next to ids.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Table {
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub definition: TableDefinition,
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

fn to_table(row: Row, board: &[BoardTable]) -> AppResult<Table> {
    let def = row.definition()?;
    Ok(Table {
        definition: schema::with_names(&row.id, &row.name, &def, board),
        id: row.id,
        board_id: row.board_id,
        name: row.name,
        version: row.version,
        created_by: row.created_by,
        updated_by: row.updated_by,
        content_updated_at: row.content_updated_at,
        deleted_at: row.deleted_at,
        deleted_by: row.deleted_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// The result of a schema write.
#[derive(Debug, Clone, Serialize)]
pub struct Written {
    pub table: Table,
    pub schema_version: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<Warning>,
    /// Other tables changed as a side effect (references removed by a forced delete).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub updated: Vec<Table>,
    /// The schema card a new table was added to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Item>,
}

// ---------- Reads ----------

async fn rows(conn: &mut SqliteConnection, board_id: &str) -> AppResult<Vec<Row>> {
    let sql = format!("SELECT {COLS} FROM schema_tables WHERE board_id = ? ORDER BY name, id");
    Ok(sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(sql))
        .bind(board_id)
        .fetch_all(conn)
        .await?)
}

async fn row(conn: &mut SqliteConnection, id: &str) -> AppResult<Row> {
    let sql = format!("SELECT {COLS} FROM schema_tables WHERE id = ?");
    sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or(AppError::NotFound("table"))
}

async fn live_row(conn: &mut SqliteConnection, id: &str) -> AppResult<Row> {
    let r = row(conn, id).await?;
    if r.deleted_at.is_some() {
        return Err(AppError::NotFound("table"));
    }
    Ok(r)
}

/// Every table on a board, live or trashed, as the schema rules need them.
pub async fn board_tables(
    conn: &mut SqliteConnection,
    board_id: &str,
) -> AppResult<Vec<BoardTable>> {
    rows(conn, board_id)
        .await?
        .iter()
        .map(Row::board_table)
        .collect()
}

/// (id, name, live) for every table on a board, for content references.
pub async fn names(
    conn: &mut SqliteConnection,
    board_id: &str,
) -> AppResult<Vec<(String, String, bool)>> {
    let rows: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT id, name, deleted_at IS NULL FROM schema_tables WHERE board_id = ? ORDER BY name",
    )
    .bind(board_id)
    .fetch_all(conn)
    .await?;
    Ok(rows)
}

/// Live tables on a board, by name.
pub async fn list_live(conn: &mut SqliteConnection, board_id: &str) -> AppResult<Vec<Table>> {
    list(conn, board_id, true).await
}

/// Trashed tables on a board, most recently deleted first.
pub async fn list_trash(conn: &mut SqliteConnection, board_id: &str) -> AppResult<Vec<Table>> {
    let mut tables = list(conn, board_id, false).await?;
    tables.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at));
    Ok(tables)
}

async fn list(conn: &mut SqliteConnection, board_id: &str, live: bool) -> AppResult<Vec<Table>> {
    let all = rows(conn, board_id).await?;
    let board: Vec<BoardTable> = all.iter().map(Row::board_table).collect::<AppResult<_>>()?;
    all.into_iter()
        .filter(|r| r.deleted_at.is_none() == live)
        .map(|r| to_table(r, &board))
        .collect()
}

pub async fn get(conn: &mut SqliteConnection, id: &str) -> AppResult<Table> {
    let r = live_row(conn, id).await?;
    let board = board_tables(conn, &r.board_id).await?;
    to_table(r, &board)
}

/// Like [`get`], but finds trashed tables too.
async fn get_any(conn: &mut SqliteConnection, id: &str) -> AppResult<Table> {
    let r = row(conn, id).await?;
    let board = board_tables(conn, &r.board_id).await?;
    to_table(r, &board)
}

pub async fn board_of(conn: &mut SqliteConnection, id: &str) -> AppResult<String> {
    Ok(row(conn, id).await?.board_id)
}

pub async fn schema_version(conn: &mut SqliteConnection, board_id: &str) -> AppResult<i64> {
    Ok(boards::get(conn, board_id).await?.schema_version)
}

/// Bump the board's schema version; returns the new one.
async fn bump(conn: &mut SqliteConnection, board_id: &str, ctx: Ctx) -> AppResult<i64> {
    let (v,): (i64,) = sqlx::query_as(
        "UPDATE boards SET schema_version = schema_version + 1, updated_at = ? WHERE id = ? RETURNING schema_version",
    )
    .bind(ctx.now.to_db())
    .bind(board_id)
    .fetch_one(conn)
    .await?;
    Ok(v)
}

// ---------- Create ----------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewTable {
    pub name: String,
    #[serde(default = "empty_definition")]
    pub definition: Value,
    /// A schema card to show the new table on.
    #[serde(default)]
    pub item_id: Option<String>,
}

fn empty_definition() -> Value {
    json!({})
}

pub async fn create(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    board_id: &str,
    new: NewTable,
) -> AppResult<Written> {
    boards::require_exists(conn, board_id).await?;
    let card = match &new.item_id {
        Some(item_id) => Some(schema_card(conn, board_id, item_id).await?),
        None => None,
    };
    let board = board_tables(conn, board_id).await?;
    let id = schema::table_id();
    let prepared = schema::prepare(&id, new.name.trim(), &new.definition, None, &board)?;
    let definition = to_json(&prepared.definition)?;
    let now = ctx.now.to_db();
    sqlx::query(
        "INSERT INTO schema_tables (id, board_id, name, definition, created_by, updated_by,
                                    content_updated_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(board_id)
    .bind(&prepared.name)
    .bind(&definition)
    .bind(ctx.actor.as_str())
    .bind(ctx.actor.as_str())
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;
    record_revision(conn, ctx, &id, &prepared.name, &definition, 1).await?;
    index(conn, &id, board_id, &prepared.name, &prepared.definition).await?;
    let schema_version = bump(conn, board_id, ctx).await?;

    let item = match card {
        Some(card) => Some(add_to_card(conn, ctx, card, &id).await?),
        None => None,
    };
    Ok(Written {
        table: get(conn, &id).await?,
        schema_version,
        warnings: prepared.warnings,
        updated: Vec::new(),
        item,
    })
}

/// A live schema card on `board_id`.
async fn schema_card(
    conn: &mut SqliteConnection,
    board_id: &str,
    item_id: &str,
) -> AppResult<Item> {
    let item = items::get(conn, item_id)
        .await
        .map_err(|_| AppError::invalid("item_id", "no item with this id"))?;
    if item.board_id != board_id || item.card_type.as_deref() != Some(CardType::Schema.as_str()) {
        return Err(AppError::invalid(
            "item_id",
            "not a schema card on this board",
        ));
    }
    Ok(item)
}

async fn add_to_card(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    card: Item,
    table_id: &str,
) -> AppResult<Item> {
    let mut ids: Vec<Value> = card
        .content
        .get("table_ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !ids.iter().any(|v| v.as_str() == Some(table_id)) {
        ids.push(json!(table_id));
    }
    items::update(
        conn,
        ctx,
        &card.id,
        ItemPatch {
            version: card.version,
            title: None,
            content: Some(json!({ "table_ids": ids })),
            style: None,
        },
    )
    .await
}

// ---------- Update ----------

/// A table update. Fields left out are unchanged.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TablePatch {
    pub version: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub definition: Option<Value>,
}

pub async fn update(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    id: &str,
    patch: TablePatch,
) -> AppResult<Written> {
    let current = live_row(conn, id).await?;
    if patch.version != current.version {
        return Err(AppError::Conflict {
            current: json!(get(conn, id).await?),
        });
    }
    let board = board_tables(conn, &current.board_id).await?;
    let existing = current.definition()?;
    let name = patch
        .name
        .as_deref()
        .map(str::trim)
        .unwrap_or(&current.name)
        .to_string();
    let input = match patch.definition {
        Some(d) => d,
        None => serde_json::to_value(&existing).map_err(|e| AppError::Internal(e.into()))?,
    };
    let prepared = schema::prepare(id, &name, &input, Some(&existing), &board)?;
    let schema_version = write(conn, ctx, &current, &prepared.name, &prepared.definition).await?;
    Ok(Written {
        table: get(conn, id).await?,
        schema_version: match schema_version {
            Some(v) => v,
            None => self::schema_version(conn, &current.board_id).await?,
        },
        warnings: prepared.warnings,
        updated: Vec::new(),
        item: None,
    })
}

/// Store a new name and definition with a version bump, a revision and a
/// schema version bump. Returns the new schema version, or `None` when
/// nothing changed.
async fn write(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    current: &Row,
    name: &str,
    definition: &TableDefinition,
) -> AppResult<Option<i64>> {
    let json = to_json(definition)?;
    if name == current.name && json == current.definition {
        return Ok(None);
    }
    let version = current.version + 1;
    let now = ctx.now.to_db();
    sqlx::query(
        "UPDATE schema_tables SET name = ?, definition = ?, version = ?, updated_by = ?,
                                  content_updated_at = ?, updated_at = ?
         WHERE id = ? AND version = ?",
    )
    .bind(name)
    .bind(&json)
    .bind(version)
    .bind(ctx.actor.as_str())
    .bind(&now)
    .bind(&now)
    .bind(&current.id)
    .bind(current.version)
    .execute(&mut *conn)
    .await?;
    record_revision(conn, ctx, &current.id, name, &json, version).await?;
    index(conn, &current.id, &current.board_id, name, definition).await?;
    Ok(Some(bump(conn, &current.board_id, ctx).await?))
}

// ---------- Delete and undelete ----------

/// Move a table to the trash. Refused while other tables reference it,
/// unless `force`, which removes those references in the same transaction.
pub async fn delete(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    id: &str,
    force: bool,
) -> AppResult<Written> {
    let current = live_row(conn, id).await?;
    let board = board_tables(conn, &current.board_id).await?;
    let referrers = schema::referencing(id, &board);
    if !referrers.is_empty() && !force {
        let from: Vec<&str> = referrers.iter().map(|r| r.from.as_str()).collect();
        return Err(AppError::State {
            code: "table_referenced",
            message: format!(
                "`{}` is referenced by {}; remove those references first, or delete with force to remove them too",
                current.name,
                from.join(", ")
            ),
        });
    }

    let mut updated_ids: Vec<String> = referrers.iter().map(|r| r.table_id.clone()).collect();
    updated_ids.sort();
    updated_ids.dedup();
    for other in &updated_ids {
        let row = live_row(conn, other).await?;
        let mut def = row.definition()?;
        schema::drop_references_to(id, &mut def);
        write(conn, ctx, &row, &row.name.clone(), &def).await?;
    }

    let now = ctx.now.to_db();
    sqlx::query(
        "UPDATE schema_tables SET deleted_at = ?, deleted_by = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(ctx.actor.as_str())
    .bind(&now)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    search::unindex(conn, "table", id).await?;
    let schema_version = bump(conn, &current.board_id, ctx).await?;

    let mut updated = Vec::new();
    for other in &updated_ids {
        updated.push(get(conn, other).await?);
    }
    Ok(Written {
        table: get_any(conn, id).await?,
        schema_version,
        warnings: Vec::new(),
        updated,
        item: None,
    })
}

pub async fn undelete(conn: &mut SqliteConnection, ctx: Ctx, id: &str) -> AppResult<Written> {
    let current = row(conn, id).await?;
    if current.deleted_at.is_none() {
        return Err(AppError::State {
            code: "not_deleted",
            message: "this table isn't in the trash".into(),
        });
    }
    let board = board_tables(conn, &current.board_id).await?;
    if let Some(clash) = board
        .iter()
        .find(|t| t.live && t.name.eq_ignore_ascii_case(&current.name))
    {
        return Err(AppError::State {
            code: "name_taken",
            message: format!(
                "there's a table called `{}` on the board now; rename it before restoring this one",
                clash.name
            ),
        });
    }
    let def = current.definition()?;
    let missing = schema::missing_targets(id, &def, &board);
    if !missing.is_empty() {
        return Err(AppError::State {
            code: "references_missing",
            message: format!(
                "`{}` references {}, which {} in the trash; restore {} first",
                current.name,
                missing.join(", "),
                if missing.len() == 1 { "is" } else { "are" },
                if missing.len() == 1 { "it" } else { "them" }
            ),
        });
    }
    sqlx::query(
        "UPDATE schema_tables SET deleted_at = NULL, deleted_by = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(ctx.now.to_db())
    .bind(id)
    .execute(&mut *conn)
    .await?;
    index(conn, id, &current.board_id, &current.name, &def).await?;
    let schema_version = bump(conn, &current.board_id, ctx).await?;
    Ok(Written {
        table: get(conn, id).await?,
        schema_version,
        warnings: Vec::new(),
        updated: Vec::new(),
        item: None,
    })
}

// ---------- Revisions ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Revision {
    pub id: i64,
    pub version: i64,
    pub name: String,
    #[sqlx(json)]
    pub definition: Value,
    pub changed_by: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Same coalescing as items (spec 4.3): one revision per actor per 5 minutes.
async fn record_revision(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    table_id: &str,
    name: &str,
    definition: &str,
    version: i64,
) -> AppResult<()> {
    let now = ctx.now.to_db();
    let window_start = ctx.now.minus(COALESCE_WINDOW).to_db();
    let newest: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT id, changed_by, updated_at FROM table_revisions WHERE table_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(table_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((rev_id, changed_by, updated_at)) = newest
        && changed_by == ctx.actor.as_str()
        && updated_at > window_start
    {
        sqlx::query(
            "UPDATE table_revisions SET name = ?, definition = ?, version = ?, updated_at = ? WHERE id = ?",
        )
        .bind(name)
        .bind(definition)
        .bind(version)
        .bind(&now)
        .bind(rev_id)
        .execute(&mut *conn)
        .await?;
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO table_revisions (table_id, name, definition, version, changed_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(table_id)
    .bind(name)
    .bind(definition)
    .bind(version)
    .bind(ctx.actor.as_str())
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM table_revisions WHERE table_id = ?1 AND id NOT IN
           (SELECT id FROM table_revisions WHERE table_id = ?1 ORDER BY id DESC LIMIT ?2)",
    )
    .bind(table_id)
    .bind(KEEP_REVISIONS)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn revisions(conn: &mut SqliteConnection, table_id: &str) -> AppResult<Vec<Revision>> {
    row(conn, table_id).await?;
    Ok(sqlx::query_as::<_, Revision>(
        "SELECT id, version, name, definition, changed_by, created_at, updated_at
         FROM table_revisions WHERE table_id = ? ORDER BY id DESC",
    )
    .bind(table_id)
    .fetch_all(conn)
    .await?)
}

/// Restore a revision: an ordinary versioned update, checked against the
/// schema as it is now.
pub async fn restore(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    table_id: &str,
    revision_id: i64,
    version: i64,
) -> AppResult<Written> {
    let rev: Option<(String, String)> = sqlx::query_as(
        "SELECT name, definition FROM table_revisions WHERE id = ? AND table_id = ?",
    )
    .bind(revision_id)
    .bind(table_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (name, definition) = rev.ok_or(AppError::NotFound("revision"))?;
    let definition: Value =
        serde_json::from_str(&definition).map_err(|e| AppError::Internal(e.into()))?;
    update(
        conn,
        ctx,
        table_id,
        TablePatch {
            version,
            name: Some(name),
            definition: Some(definition),
        },
    )
    .await
}

// ---------- Helpers ----------

async fn index(
    conn: &mut SqliteConnection,
    id: &str,
    board_id: &str,
    name: &str,
    def: &TableDefinition,
) -> AppResult<()> {
    search::index(
        conn,
        "table",
        id,
        board_id,
        Some(name),
        &schema::search_body(def),
    )
    .await
}

fn to_json<T: Serialize>(v: &T) -> AppResult<String> {
    serde_json::to_string(v).map_err(|e| AppError::Internal(e.into()))
}
