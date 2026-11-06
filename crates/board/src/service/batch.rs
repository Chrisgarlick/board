//! Batches: several operations in one transaction (spec section 6, "Batch").
//!
//! A `create_item` can carry a `ref`; later operations can use that ref
//! wherever they take an item id. If any operation fails, nothing is applied
//! and the error says which operation it was.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::SqliteConnection;

use crate::error::{AppError, AppResult};
use crate::service::items::{self, Geometry, ItemPatch, NewItem};
use crate::types::Ctx;

pub const MAX_OPS: usize = 500;
const MAX_REF_LEN: usize = 64;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub ops: Vec<Op>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    CreateItem {
        #[serde(default, rename = "ref")]
        reference: Option<String>,
        kind: String,
        #[serde(default)]
        card_type: Option<String>,
        #[serde(default)]
        title: Option<String>,
        content: Value,
        #[serde(default)]
        style: Option<Value>,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
        #[serde(default)]
        w: Option<f64>,
        #[serde(default)]
        h: Option<f64>,
        #[serde(default)]
        z: Option<i64>,
    },
    UpdateItem {
        item: String,
        version: i64,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        content: Option<Value>,
        #[serde(default, deserialize_with = "present")]
        style: Option<Option<Value>>,
    },
    SetGeometry {
        item: String,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
        #[serde(default)]
        w: Option<f64>,
        #[serde(default)]
        h: Option<f64>,
        #[serde(default)]
        z: Option<i64>,
        #[serde(default)]
        locked: Option<bool>,
    },
    DeleteItem {
        item: String,
    },
    UndeleteItem {
        item: String,
    },
}

fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<Value>>, D::Error> {
    Option::<Value>::deserialize(d).map(Some)
}

#[derive(Debug, Serialize)]
pub struct BatchResult {
    /// One entry per operation: the item afterwards, or `{ "deleted": id }`.
    pub results: Vec<Value>,
    /// New item ids, keyed by the `ref` given in `create_item`.
    pub refs: HashMap<String, String>,
}

pub async fn apply(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    board_id: &str,
    batch: Batch,
) -> AppResult<BatchResult> {
    if batch.ops.len() > MAX_OPS {
        return Err(AppError::invalid(
            "ops",
            format!("at most {MAX_OPS} operations per batch"),
        ));
    }
    crate::service::boards::require_exists(conn, board_id).await?;

    let mut refs: HashMap<String, String> = HashMap::new();
    let mut results = Vec::with_capacity(batch.ops.len());

    for (index, op) in batch.ops.into_iter().enumerate() {
        let result = apply_one(conn, ctx, board_id, op, &mut refs)
            .await
            .map_err(|source| AppError::InBatch {
                index,
                source: Box::new(source),
            })?;
        results.push(result);
    }
    Ok(BatchResult { results, refs })
}

async fn apply_one(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    board_id: &str,
    op: Op,
    refs: &mut HashMap<String, String>,
) -> AppResult<Value> {
    match op {
        Op::CreateItem {
            reference,
            kind,
            card_type,
            title,
            content,
            style,
            x,
            y,
            w,
            h,
            z,
        } => {
            if let Some(r) = &reference {
                check_ref(r, refs)?;
            }
            let new = NewItem {
                kind,
                card_type,
                title,
                content,
                style,
                x,
                y,
                w,
                h,
                z,
            };
            let created = items::create(conn, ctx, board_id, new).await?;
            if let Some(r) = reference {
                refs.insert(r, created.id.clone());
            }
            Ok(json!(created))
        }
        Op::UpdateItem {
            item,
            version,
            title,
            content,
            style,
        } => {
            let id = resolve(conn, board_id, &item, refs).await?;
            let updated = items::update(
                conn,
                ctx,
                &id,
                ItemPatch {
                    version,
                    title,
                    content,
                    style,
                },
            )
            .await?;
            Ok(json!(updated))
        }
        Op::SetGeometry {
            item,
            x,
            y,
            w,
            h,
            z,
            locked,
        } => {
            let id = resolve(conn, board_id, &item, refs).await?;
            let moved = items::set_geometry(
                conn,
                ctx,
                &id,
                Geometry {
                    x,
                    y,
                    w,
                    h,
                    z,
                    locked,
                },
            )
            .await?;
            Ok(json!(moved))
        }
        Op::DeleteItem { item } => {
            let id = resolve(conn, board_id, &item, refs).await?;
            items::delete(conn, ctx, &id).await?;
            Ok(json!({ "deleted": id }))
        }
        Op::UndeleteItem { item } => {
            let id = resolve(conn, board_id, &item, refs).await?;
            Ok(json!(items::undelete(conn, ctx, &id).await?))
        }
    }
}

fn check_ref(r: &str, refs: &HashMap<String, String>) -> AppResult<()> {
    let valid = !r.is_empty()
        && r.len() <= MAX_REF_LEN
        && r.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid {
        return Err(AppError::invalid(
            "ref",
            format!("refs are 1–{MAX_REF_LEN} letters, digits, `_` or `-`"),
        ));
    }
    if refs.contains_key(r) {
        return Err(AppError::invalid("ref", format!("ref `{r}` is used twice")));
    }
    Ok(())
}

/// A ref from earlier in this batch, or an item id. Either way the item must
/// be on this batch's board.
async fn resolve(
    conn: &mut SqliteConnection,
    board_id: &str,
    item: &str,
    refs: &HashMap<String, String>,
) -> AppResult<String> {
    let id = refs.get(item).cloned().unwrap_or_else(|| item.to_string());
    if items::board_of(conn, &id).await? != board_id {
        return Err(AppError::NotFound("item"));
    }
    Ok(id)
}
