//! Batches: several operations in one transaction (spec section 6, "Batch").
//!
//! A `create_item` or `create_connector` can carry a `ref`; later operations
//! can use that ref wherever they take an item or connector id. If any
//! operation fails, nothing is applied and the error says which operation it
//! was.
//!
//! Besides one result per operation, the response lists `affected` items:
//! items the operations changed as a side effect (a frame's members moving
//! with it, frame membership changing), in their final state. The UI applies
//! those so it never has to repeat the server's rules.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::SqliteConnection;

use crate::error::{AppError, AppResult};
use crate::events::Change;
use crate::service::connectors::{self, Anchor, ConnectorPatch, NewConnector};
use crate::service::items::{self, Geometry, Item, ItemPatch, NewItem};
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
        #[serde(default)]
        with_contents: bool,
    },
    UndeleteItem {
        item: String,
    },
    CreateConnector {
        #[serde(default, rename = "ref")]
        reference: Option<String>,
        from: String,
        to: String,
        #[serde(default)]
        from_anchor: Option<Anchor>,
        #[serde(default)]
        to_anchor: Option<Anchor>,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        style: Option<Value>,
    },
    UpdateConnector {
        connector: String,
        #[serde(default)]
        from: Option<String>,
        #[serde(default)]
        to: Option<String>,
        #[serde(default)]
        from_anchor: Option<Anchor>,
        #[serde(default)]
        to_anchor: Option<Anchor>,
        #[serde(default, deserialize_with = "present_string")]
        label: Option<Option<String>>,
        #[serde(default, deserialize_with = "present")]
        style: Option<Option<Value>>,
    },
    DeleteConnector {
        connector: String,
    },
    UndeleteConnector {
        connector: String,
    },
}

fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<Value>>, D::Error> {
    Option::<Value>::deserialize(d).map(Some)
}

fn present_string<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

#[derive(Debug, Serialize)]
pub struct BatchResult {
    /// One entry per operation: the item or connector afterwards, or what a
    /// delete removed.
    pub results: Vec<Value>,
    /// New ids, keyed by the `ref` given when creating.
    pub refs: HashMap<String, String>,
    /// Items changed as a side effect, in their final state.
    pub affected: Vec<Item>,
    /// What changed, for live updates. Not part of the response.
    #[serde(skip)]
    pub changes: Vec<Change>,
}

/// Refs created so far, and items changed as side effects.
struct State {
    refs: HashMap<String, String>,
    affected: BTreeSet<String>,
    changes: Vec<Change>,
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

    let mut state = State {
        refs: HashMap::new(),
        affected: BTreeSet::new(),
        changes: Vec::new(),
    };
    let mut results = Vec::with_capacity(batch.ops.len());
    for (index, op) in batch.ops.into_iter().enumerate() {
        let result = apply_one(conn, ctx, board_id, op, &mut state)
            .await
            .map_err(|source| AppError::InBatch {
                index,
                source: Box::new(source),
            })?;
        results.push(result);
    }

    // Final state of side-effected items that are still live.
    let mut affected = Vec::new();
    for id in &state.affected {
        if let Ok(item) = items::get(conn, id).await {
            state
                .changes
                .push(Change::item("item.geometry", item.clone()));
            affected.push(item);
        }
    }
    Ok(BatchResult {
        results,
        refs: state.refs,
        affected,
        changes: state.changes,
    })
}

async fn apply_one(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    board_id: &str,
    op: Op,
    state: &mut State,
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
                check_ref(r, &state.refs)?;
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
            let (created, affected) = items::create(conn, ctx, board_id, new).await?;
            if let Some(r) = reference {
                state.refs.insert(r, created.id.clone());
            }
            state.affected.extend(affected);
            state
                .changes
                .push(Change::item("item.created", created.clone()));
            Ok(json!(created))
        }
        Op::UpdateItem {
            item,
            version,
            title,
            content,
            style,
        } => {
            let id = resolve_item(conn, board_id, &item, &state.refs).await?;
            let patch = ItemPatch {
                version,
                title,
                content,
                style,
            };
            let updated = items::update(conn, ctx, &id, patch).await?;
            state
                .changes
                .push(Change::item("item.updated", updated.clone()));
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
            let id = resolve_item(conn, board_id, &item, &state.refs).await?;
            let g = Geometry {
                x,
                y,
                w,
                h,
                z,
                locked,
            };
            let (moved, affected) = items::set_geometry(conn, ctx, &id, g).await?;
            state.affected.extend(affected);
            state
                .changes
                .push(Change::item("item.geometry", moved.clone()));
            Ok(json!(moved))
        }
        Op::DeleteItem {
            item,
            with_contents,
        } => {
            let id = resolve_item(conn, board_id, &item, &state.refs).await?;
            let deleted = items::delete(conn, ctx, &id, with_contents).await?;
            state.affected.extend(deleted.affected.iter().cloned());
            for gone in &deleted.deleted {
                state
                    .changes
                    .push(Change::removed("item.deleted", gone.clone()));
            }
            Ok(json!(deleted))
        }
        Op::UndeleteItem { item } => {
            let id = resolve_item(conn, board_id, &item, &state.refs).await?;
            let (restored, affected) = items::undelete(conn, ctx, &id).await?;
            state.affected.extend(affected);
            state
                .changes
                .push(Change::item("item.undeleted", restored.clone()));
            Ok(json!(restored))
        }
        Op::CreateConnector {
            reference,
            from,
            to,
            from_anchor,
            to_anchor,
            label,
            style,
        } => {
            if let Some(r) = &reference {
                check_ref(r, &state.refs)?;
            }
            let from = resolve_item(conn, board_id, &from, &state.refs)
                .await
                .map_err(|_| AppError::invalid("from", "no such item on this board"))?;
            let to = resolve_item(conn, board_id, &to, &state.refs)
                .await
                .map_err(|_| AppError::invalid("to", "no such item on this board"))?;
            let new = NewConnector {
                from,
                to,
                from_anchor,
                to_anchor,
                label,
                style,
            };
            let created = connectors::create(conn, ctx, board_id, new).await?;
            if let Some(r) = reference {
                state.refs.insert(r, created.id.clone());
            }
            state
                .changes
                .push(Change::connector("connector.created", created.clone()));
            Ok(json!(created))
        }
        Op::UpdateConnector {
            connector,
            from,
            to,
            from_anchor,
            to_anchor,
            label,
            style,
        } => {
            let id = resolve_connector(conn, board_id, &connector, &state.refs).await?;
            let from = match from {
                Some(f) => Some(resolve_item(conn, board_id, &f, &state.refs).await?),
                None => None,
            };
            let to = match to {
                Some(t) => Some(resolve_item(conn, board_id, &t, &state.refs).await?),
                None => None,
            };
            let patch = ConnectorPatch {
                from,
                to,
                from_anchor,
                to_anchor,
                label,
                style,
            };
            let updated = connectors::update(conn, ctx, &id, patch).await?;
            state
                .changes
                .push(Change::connector("connector.updated", updated.clone()));
            Ok(json!(updated))
        }
        Op::DeleteConnector { connector } => {
            let id = resolve_connector(conn, board_id, &connector, &state.refs).await?;
            connectors::delete(conn, ctx, &id).await?;
            state
                .changes
                .push(Change::removed("connector.deleted", id.clone()));
            Ok(json!({ "deleted_connector": id }))
        }
        Op::UndeleteConnector { connector } => {
            let id = resolve_connector(conn, board_id, &connector, &state.refs).await?;
            let restored = connectors::undelete(conn, ctx, &id).await?;
            state
                .changes
                .push(Change::connector("connector.created", restored.clone()));
            Ok(json!(restored))
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
async fn resolve_item(
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

async fn resolve_connector(
    conn: &mut SqliteConnection,
    board_id: &str,
    connector: &str,
    refs: &HashMap<String, String>,
) -> AppResult<String> {
    let id = refs
        .get(connector)
        .cloned()
        .unwrap_or_else(|| connector.to_string());
    if connectors::board_of(conn, &id).await? != board_id {
        return Err(AppError::NotFound("connector"));
    }
    Ok(id)
}
