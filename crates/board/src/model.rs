//! The board model (spec section 9.2): what Design, Claude in another
//! project, or a committed `board-model.json` read about a board's plan.
//!
//! [`board_model`] is pure: give it a board's live items, connectors and
//! tables and it returns the model. Everything in it is in a stable order,
//! so the same board always produces the same bytes (and the same ETag).

use std::collections::{HashMap, HashSet};

use board_model::{
    BoardModel, CONTRACT, CONTRACT_VERSION, Feature, Flow, FlowEdge, FlowNode, ModelBoard,
    ModelColumn, ModelTable, Note, SchemaColumn, SitemapNode, humanise,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::content::{FeatureContent, SitemapContent};
use crate::schema::{self, BoardTable};
use crate::service::boards::Board;
use crate::service::connectors::Connector;
use crate::service::items::Item;

/// Items whose tops are within this many units count as one row when reading.
pub const READING_BAND: f64 = 200.0;

/// Sort items into reading order: rows of 200-unit bands by `y`, then left
/// to right, then by id so ties are stable.
pub fn reading_order<'a>(items: impl IntoIterator<Item = &'a Item>) -> Vec<&'a Item> {
    let mut list: Vec<&Item> = items.into_iter().collect();
    list.sort_by(|a, b| {
        let band = |i: &Item| (i.y / READING_BAND).floor() as i64;
        band(a)
            .cmp(&band(b))
            .then(a.x.total_cmp(&b.x))
            .then(a.id.cmp(&b.id))
    });
    list
}

/// A short hash of a column's definition: everything except its id and the
/// names filled in on read. Stable across Board versions (SHA-256).
pub fn column_hash(c: &SchemaColumn) -> String {
    let mut bare = c.clone();
    bare.id = String::new();
    if let Some(r) = &mut bare.references {
        r.table = None;
        r.column = None;
    }
    let json = serde_json::to_string(&bare).unwrap_or_default();
    hex(&Sha256::digest(json.as_bytes())[..4])
}

/// The ETag for a model: a hash of its JSON.
pub fn etag(model: &BoardModel) -> String {
    let json = serde_json::to_string(model).unwrap_or_default();
    format!("\"{}\"", hex(&Sha256::digest(json.as_bytes())[..8]))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Build the model. `items` and `connectors` are the board's live ones;
/// `tables` every table on the board (trashed ones are left out here).
pub fn board_model(
    board: &Board,
    items: &[Item],
    connectors: &[Connector],
    tables: &[BoardTable],
    table_versions: &HashMap<String, i64>,
) -> BoardModel {
    let mut warnings = Vec::new();

    // Tables, by name, with labels filled in and a hash per column.
    let mut live: Vec<&BoardTable> = tables.iter().filter(|t| t.live).collect();
    live.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    let live_ids: HashSet<&str> = live.iter().map(|t| t.id.as_str()).collect();
    let model_tables = live
        .iter()
        .map(|t| {
            let def = schema::with_names(&t.id, &t.name, &t.definition, tables);
            ModelTable {
                id: t.id.clone(),
                name: t.name.clone(),
                comment: def.comment.clone(),
                version: table_versions.get(&t.id).copied().unwrap_or(1),
                columns: def
                    .columns
                    .iter()
                    .map(|c| {
                        let hash = column_hash(c);
                        let mut column = c.clone();
                        column.label.get_or_insert_with(|| humanise(&c.name));
                        ModelColumn { column, hash }
                    })
                    .collect(),
                indexes: def.indexes,
            }
        })
        .collect();

    let ordered = reading_order(items);
    let cards = |kind: &str| -> Vec<&Item> {
        ordered
            .iter()
            .copied()
            .filter(|i| i.card_type.as_deref() == Some(kind))
            .collect()
    };

    // Pages: one root per sitemap card.
    let sitemaps = cards("sitemap");
    if sitemaps.len() > 1 {
        warnings.push(format!(
            "This board has {} sitemap cards; their pages are listed one after another, in reading order.",
            sitemaps.len()
        ));
    }
    let pages: Vec<SitemapNode> = sitemaps
        .iter()
        .filter_map(|i| serde_json::from_value::<SitemapContent>(i.content.clone()).ok())
        .map(|c| c.root)
        .collect();
    let page_ids: HashSet<String> = pages
        .iter()
        .flat_map(|p| p.walk().into_iter().map(|n| n.id.clone()))
        .collect();

    let features = cards("feature")
        .iter()
        .filter_map(|i| {
            let f: FeatureContent = serde_json::from_value(i.content.clone()).ok()?;
            Some(Feature {
                id: i.id.clone(),
                title: i.title.clone().unwrap_or_default(),
                status: f.status,
                summary: f.summary,
                stories: f.stories,
                acceptance: f
                    .acceptance
                    .into_iter()
                    .map(|a| board_model::AcceptanceItem {
                        text: a.text,
                        done: a.done,
                    })
                    .collect(),
                table_ids: f
                    .table_ids
                    .into_iter()
                    .filter(|id| live_ids.contains(id.as_str()))
                    .collect(),
                page_ids: f
                    .page_ids
                    .into_iter()
                    .filter(|id| page_ids.contains(id))
                    .collect(),
            })
        })
        .collect();

    let notes = cards("note")
        .iter()
        .map(|i| Note {
            id: i.id.clone(),
            title: i.title.clone().unwrap_or_default(),
            markdown: text(&i.content, "markdown").unwrap_or_default(),
        })
        .collect();

    // Flows: frames with at least one connector between their members.
    let flows = ordered
        .iter()
        .filter(|i| i.kind == "frame")
        .filter_map(|frame| {
            let members: Vec<&Item> = ordered
                .iter()
                .copied()
                .filter(|i| {
                    i.frame_id.as_deref() == Some(frame.id.as_str())
                        && (i.kind == "shape" || i.kind == "card")
                })
                .collect();
            let ids: HashSet<&str> = members.iter().map(|i| i.id.as_str()).collect();
            let mut edges: Vec<(&Connector, FlowEdge)> = connectors
                .iter()
                .filter(|c| ids.contains(c.from_item.as_str()) && ids.contains(c.to_item.as_str()))
                .map(|c| {
                    (
                        c,
                        FlowEdge {
                            from: c.from_item.clone(),
                            to: c.to_item.clone(),
                            label: c.label.clone().filter(|l| !l.is_empty()),
                        },
                    )
                })
                .collect();
            if edges.is_empty() {
                return None;
            }
            edges.sort_by(|a, b| {
                a.0.created_at
                    .cmp(&b.0.created_at)
                    .then(a.0.id.cmp(&b.0.id))
            });
            let nodes = members
                .iter()
                .map(|i| {
                    let shape = i.kind == "shape";
                    FlowNode {
                        id: i.id.clone(),
                        kind: i.kind.clone(),
                        shape: shape.then(|| text(&i.content, "shape")).flatten(),
                        text: shape.then(|| text(&i.content, "text")).flatten(),
                        card_type: i.card_type.clone(),
                        title: i.title.clone(),
                        page_id: text(&i.content, "page_id").filter(|p| page_ids.contains(p)),
                    }
                })
                .collect();
            Some(Flow {
                id: frame.id.clone(),
                title: frame.title.clone().unwrap_or_default(),
                nodes,
                edges: edges.into_iter().map(|(_, e)| e).collect(),
            })
        })
        .collect();

    BoardModel {
        contract: CONTRACT.to_string(),
        contract_version: CONTRACT_VERSION,
        board: ModelBoard {
            id: board.id.clone(),
            title: board.title.clone(),
            description: board.description.clone(),
            stack: board.stack.clone(),
            client: board.client.clone(),
        },
        schema_version: board.schema_version,
        tables: model_tables,
        pages,
        features,
        flows,
        notes,
        warnings,
    }
}

/// Load a board and build its model.
pub async fn load(
    conn: &mut sqlx::SqliteConnection,
    board_id: &str,
) -> crate::error::AppResult<BoardModel> {
    use crate::service::{boards, connectors, items, tables};
    let board = boards::get(conn, board_id).await?;
    let items = items::list_live(conn, board_id).await?;
    let connectors = connectors::list_live(conn, board_id).await?;
    let all = tables::board_tables(conn, board_id).await?;
    let versions: Vec<(String, i64)> =
        sqlx::query_as("SELECT id, version FROM schema_tables WHERE board_id = ?")
            .bind(board_id)
            .fetch_all(conn)
            .await?;
    Ok(board_model(
        &board,
        &items,
        &connectors,
        &all,
        &versions.into_iter().collect(),
    ))
}

#[cfg(test)]
mod tests;
