//! The MCP server at `/mcp` (spec section 7): thin wrappers over the service
//! layer, so Claude reads and draws on boards with the same rules, validation
//! and live updates as everything else.
//!
//! Auth is the usual bearer token, and it must be an `admin` token; writes
//! are recorded as the token's actor (`claude` for Claude's token). Errors
//! come back as tool errors carrying the same JSON as the REST API (`error`,
//! `message`, `path`), so Claude can correct itself.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::Request;
use axum::http::request::Parts;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::{Extension as AxumExtension, Router};
use base64::Engine as _;
use chrono::TimeDelta;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::routes::{ctx, run_batch};
use crate::service::batch::Op;
use crate::service::connectors::{self, Anchor};
use crate::service::flows::{self, FlowEdge, FlowNode, FlowRequest};
use crate::service::items::{self, Item};
use crate::service::moves::{self, MoveRequest};
use crate::service::{boards, search, tables};
use crate::types::{Caller, Scope};

/// Content in `get_board` is cut to about this many bytes per item.
const TRUNCATE_BYTES: usize = 2_000;
/// Claude can't delete what the user edited this recently without `force`.
const USER_EDIT_GUARD: TimeDelta = TimeDelta::minutes(5);

const INSTRUCTIONS: &str = "Board is a planning canvas. A board holds items (cards, shapes, \
sticky notes, text and frames) joined by connectors, plus one database schema (its tables). \
Use get_board to see a board; draw flows with draw_flow (Mermaid) or add_items with layout \
\"auto\"; put drawings meant for a planning document inside a frame. Design data with \
add_table and update_table (references by table and column name are fine), and show tables \
on a schema card. Pages go in a sitemap card; link a journey step to its page with the \
shape's \"page\" (slug). Plan features with feature cards. get_board_model returns \
the whole plan in the shape Design reads; export_pdf makes the client-ready planning PDF. Every change you make is recorded as yours and can be undone by \
the user, so act directly rather than asking for permission for ordinary edits.";

type ToolResult = Result<String, String>;

fn ok(value: Value) -> ToolResult {
    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
}

fn fail(err: AppError) -> String {
    let err = match err {
        AppError::InBatch { source, index } => {
            let mut body = source.body();
            body["op_index"] = json!(index);
            return body.to_string();
        }
        other => other,
    };
    err.body().to_string()
}

/// The authenticated caller, attached by the auth middleware.
fn caller(parts: &Parts) -> Result<Caller, String> {
    parts
        .extensions
        .get::<Caller>()
        .copied()
        .ok_or_else(|| fail(AppError::Unauthorized))
}

// ---------- Parameters ----------

#[derive(Deserialize, JsonSchema)]
pub struct BoardId {
    pub board_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct CreateBoard {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Free text, e.g. "Laravel + MySQL": the stack this job uses.
    #[serde(default)]
    pub stack: Option<String>,
    /// The client's slug in Clients (their brand styles the PDF).
    #[serde(default)]
    pub client: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateBoard {
    pub board_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub stack: Option<String>,
    /// The client's slug in Clients.
    #[serde(default)]
    pub client: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GetBoard {
    pub board_id: String,
    /// Only items in this frame (and the connectors between them).
    #[serde(default)]
    pub frame_id: Option<String>,
    /// Only these kinds: card, shape, sticky, text, frame.
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ItemId {
    pub item_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct Search {
    pub query: String,
    #[serde(default)]
    pub board_id: Option<String>,
}

/// An item to create. See the add_items description for content shapes.
#[derive(Deserialize, JsonSchema)]
pub struct NewItemArg {
    /// A local name for this item, so connectors in the same call can refer to it.
    #[serde(default, rename = "ref")]
    pub reference: Option<String>,
    /// card, shape, sticky, text or frame.
    pub kind: String,
    /// For cards: note, checklist, feature, schema or sitemap.
    #[serde(default)]
    pub card_type: Option<String>,
    /// Cards and frames only.
    #[serde(default)]
    pub title: Option<String>,
    pub content: Value,
    /// { "color": grey|blue|green|yellow|orange|red|purple|pink, "align": left|center|right (shapes and text) }
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
}

#[derive(Deserialize, JsonSchema)]
pub struct NewConnectorArg {
    /// An item id, or a ref from this call.
    pub from: String,
    /// An item id, or a ref from this call.
    pub to: String,
    #[serde(default)]
    pub label: Option<String>,
    /// auto (default), top, right, bottom or left.
    #[serde(default)]
    pub from_anchor: Option<Anchor>,
    #[serde(default)]
    pub to_anchor: Option<Anchor>,
    /// { "line": solid|dashed|dotted, "start"/"end": none|arrow|open_arrow|dot, "routing": straight|elbow|curved, "color": … }
    #[serde(default)]
    pub style: Option<Value>,
}

#[derive(Deserialize, JsonSchema)]
pub struct AddItems {
    pub board_id: String,
    pub items: Vec<NewItemArg>,
    #[serde(default)]
    pub connectors: Vec<NewConnectorArg>,
    /// "auto" lays shapes out as a flow (positions are ignored); "none" (default) uses the given positions, auto-placing items without one.
    #[serde(default)]
    pub layout: Option<String>,
    /// For layout "auto": TB (default), LR, BT or RL.
    #[serde(default)]
    pub direction: Option<String>,
    /// For layout "auto": wrap the flow in a frame with this title.
    #[serde(default)]
    pub frame_title: Option<String>,
    /// For layout "auto": place the flow beside this item.
    #[serde(default)]
    pub near: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DrawFlow {
    pub board_id: String,
    /// A Mermaid flowchart in the supported subset (see the tool description).
    pub mermaid: String,
    #[serde(default)]
    pub frame_title: Option<String>,
    #[serde(default)]
    pub near: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct FrameId {
    pub frame_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateItem {
    pub item_id: String,
    /// The item's current version (from get_board or get_item).
    pub version: i64,
    #[serde(default)]
    pub title: Option<String>,
    /// The whole new content for the item's kind.
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default)]
    pub style: Option<Value>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GeometryArg {
    pub item_id: String,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub w: Option<f64>,
    #[serde(default)]
    pub h: Option<f64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetGeometry {
    pub updates: Vec<GeometryArg>,
}

#[derive(Deserialize, JsonSchema)]
pub struct MoveToBoard {
    pub item_ids: Vec<String>,
    pub board_id: String,
    #[serde(default)]
    pub dx: Option<f64>,
    #[serde(default)]
    pub dy: Option<f64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DeleteItems {
    pub item_ids: Vec<String>,
    /// Delete even if the user edited the item in the last 5 minutes.
    #[serde(default)]
    pub force: bool,
    /// For frames: also delete what's inside.
    #[serde(default)]
    pub with_contents: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateConnector {
    pub connector_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub from_anchor: Option<Anchor>,
    #[serde(default)]
    pub to_anchor: Option<Anchor>,
    #[serde(default)]
    pub style: Option<Value>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ConnectorId {
    pub connector_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ItemImage {
    /// A frame (drawn with everything in it), a schema card (its ER diagram) or any single item.
    pub item_id: String,
    /// Pixels per canvas unit, 1–4 (default 1, which is plenty to look at).
    #[serde(default)]
    pub scale: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct Revisions {
    /// An item, or
    #[serde(default)]
    pub item_id: Option<String>,
    /// a table.
    #[serde(default)]
    pub table_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RestoreRevision {
    #[serde(default)]
    pub item_id: Option<String>,
    #[serde(default)]
    pub table_id: Option<String>,
    pub revision_id: i64,
    /// The item's (or table's) current version.
    pub version: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct PdfArgs {
    pub board_id: String,
    /// Frames and cards to include, in this order. Leave out for every frame and loose card, in reading order.
    #[serde(default)]
    pub item_ids: Vec<String>,
    /// Cover title (default: the board's title).
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    /// Who it's for, on the cover.
    #[serde(default)]
    pub recipient: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    /// A client's slug in Clients, for their brand (default: the board's client).
    #[serde(default)]
    pub client: Option<String>,
    /// For a document you'll join with others: start with a part divider of this title.
    #[serde(default)]
    pub part: Option<String>,
}

impl PdfArgs {
    fn options(self) -> (String, crate::typeset::Options) {
        (
            self.board_id,
            crate::typeset::Options {
                item_ids: self.item_ids,
                title: self.title,
                subtitle: self.subtitle,
                recipient: self.recipient,
                author: self.author,
                date: self.date,
                eyebrow: None,
                client: self.client,
                part: self.part,
            },
        )
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct AddTable {
    pub board_id: String,
    /// The table's database name: letters, digits and underscores.
    pub name: String,
    /// { "comment"?, "columns": [...], "indexes"?: [...] }; see the tool description.
    #[serde(default)]
    pub definition: Option<Value>,
    /// A schema card to show the new table on.
    #[serde(default)]
    pub item_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateTable {
    pub table_id: String,
    /// The table's current version (from get_schema).
    pub version: i64,
    #[serde(default)]
    pub name: Option<String>,
    /// The whole new definition. Keep each existing column's "id" (or its name) so the id survives.
    #[serde(default)]
    pub definition: Option<Value>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DeleteTable {
    pub table_id: String,
    /// Delete even if other tables reference it (those references are removed), or if the user edited it in the last 5 minutes.
    #[serde(default)]
    pub force: bool,
}

// ---------- Server ----------

#[derive(Clone)]
pub struct BoardMcp {
    state: AppState,
    tool_router: ToolRouter<Self>,
}

const ADD_ITEMS_DESCRIPTION: &str = r#"Create items (and connectors between them or existing items) in one step.

Item kinds and their content:
- shape: { "shape": "rect"|"rounded"|"diamond"|"ellipse"|"parallelogram"|"cylinder"|"document", "text": "…" }. rect = process step, rounded = start/end, diamond = decision, ellipse = event, parallelogram = input/output, cylinder = data store.
- sticky: { "text": "…" } (yellow unless style.color is set)
- text: { "text": "…", "size": "s"|"m"|"l"|"xl" }
- frame: needs "title"; content {} or { "description": "…" }. Items whose centre is inside a frame belong to it.
- card with card_type "note": needs "title"; content { "markdown": "…" }
- card with card_type "checklist": needs "title"; content { "items": [{ "text": "…", "done": false, "group": "optional" }] }
- card with card_type "schema": needs "title"; content { "tables": ["users", "orders"] } (names; stored as table_ids). Shows those tables of the board schema as an ER diagram. Create tables with add_table first.
- card with card_type "feature": needs "title"; content { "status": "idea"|"planned"|"in_progress"|"done", "summary": "markdown", "stories": ["As a … I want … so that …"], "acceptance": [{ "text": "…", "done": false }], "tables": ["orders"], "pages": ["/checkout"] } (tables by name and pages by slug; stored as table_ids and page_ids). One card per feature.
- card with card_type "sitemap": needs "title"; content { "root": { "title": "Home", "slug": "/", "purpose": "…", "children": [ { "title": "Shop", "slug": "/shop", "children": [] } ] } }. Pages get stable ids; when updating, keep each page's "id" (or its slug) so the id survives. One sitemap per board is normal.
- shapes can link to a sitemap page: add "page": "/login" (its slug) to the shape's content (stored as page_id).

Shapes, stickies and text have no title: their words go in content.text.
With layout "auto", every item must be a shape; they're laid out as a flow (like draw_flow) and connectors become its edges. Otherwise items go where you put them (x, y) or in the next free space."#;

const DRAW_FLOW_DESCRIPTION: &str = r#"Draw a flowchart from Mermaid, laid out automatically, optionally in a titled frame. The result is ordinary shapes and connectors the user can rearrange.

Supported subset:
- Header: flowchart TD|TB|LR|RL|BT (or graph …)
- Nodes: A[process] A(start/end) A{decision} A((event)) A[/input-output/] A[(data store)]; quote text with punctuation: A["Pay (card)"]
- Edges: --> (arrow), --- (no arrow), -.-> (dashed), ==> ; labels: A -->|yes| B or A -- yes --> B; chains A --> B --> C
- subgraph Title … end (one level) becomes a frame
- %% comments; ; separates statements
Anything else (classDef, style, &, click…) is an error naming the line."#;

const TABLE_DESCRIPTION: &str = r#"Add a table to the board's schema (one schema per board). References by name are fine.

definition: { "comment": "…", "columns": [column, …], "indexes": [{ "columns": ["a", "b"], "unique": false }] }
column: { "name": "user_id", "type": "bigint", … } with optional
  "nullable": bool (default false), "primary": bool, "unique": bool, "default": "literal",
  "generated": "increment" | "uuid" | "now" | "now_on_update",
  "references": { "table": "users", "column": "id" } (column defaults to the table's primary key),
  "label": "Human label", "comment": "…", "native_type": "varchar(191)" (stack-specific, recorded only)
Logical types (use these, not database types): string (with "length"), text, integer, bigint,
decimal (with "precision", "scale"), boolean, date, time, datetime, uuid, json, enum (with "values": [...]), binary.
Names: letters, digits and underscores, not starting with a digit; unique on the board (tables) or in the table (columns).
Returns the table with ids for every column, the new schema_version, and any warnings (they don't block the write)."#;

#[tool_router]
impl BoardMcp {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    async fn conn(&self) -> Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, String> {
        self.state.pool.acquire().await.map_err(|e| fail(e.into()))
    }

    #[tool(
        description = "List boards: id, title, stack, number of items and tables. Archived boards are left out."
    )]
    async fn list_boards(&self) -> ToolResult {
        let mut conn = self.conn().await?;
        let list = boards::list(&mut conn, None, false).await.map_err(fail)?;
        ok(json!(list
            .iter()
            .map(|b| json!({ "id": b.board.id, "title": b.board.title, "stack": b.board.stack, "items": b.item_count, "tables": b.table_count }))
            .collect::<Vec<_>>()))
    }

    #[tool(description = "Create a board (one per client job).")]
    async fn create_board(
        &self,
        Parameters(p): Parameters<CreateBoard>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let mut tx = db::write_tx(&self.state.pool)
            .await
            .map_err(|e| fail(e.into()))?;
        let board = boards::create(
            &mut tx,
            ctx(&self.state, who),
            boards::NewBoard {
                title: p.title,
                description: p.description,
                stack: p.stack,
                client: p.client,
            },
        )
        .await
        .map_err(fail)?;
        tx.commit().await.map_err(|e| fail(e.into()))?;
        ok(json!(board))
    }

    #[tool(
        description = "Rename a board, or change its description, stack or client (a slug in Clients, whose brand styles the PDF)."
    )]
    async fn update_board(
        &self,
        Parameters(p): Parameters<UpdateBoard>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let mut tx = db::write_tx(&self.state.pool)
            .await
            .map_err(|e| fail(e.into()))?;
        let patch = boards::BoardPatch {
            title: p.title,
            description: p.description.map(Some),
            stack: p.stack.map(Some),
            client: p.client.map(Some),
            archived: None,
        };
        let board = boards::update(&mut tx, ctx(&self.state, who), &p.board_id, patch)
            .await
            .map_err(fail)?;
        tx.commit().await.map_err(|e| fail(e.into()))?;
        self.state.hub.publish(
            &p.board_id,
            None,
            vec![crate::events::Change::board("board.updated", json!(board))],
        );
        ok(json!(board))
    }

    #[tool(
        description = "A board's items and connectors, and its tables by name (get_schema has their columns). Item content is cut to about 2 KB (marked \"truncated\": true; use get_item for all of it). Each item has a version, needed to update it. Filter with frame_id or kinds on big boards."
    )]
    async fn get_board(&self, Parameters(p): Parameters<GetBoard>) -> ToolResult {
        let mut conn = self.conn().await?;
        let full = boards::get_full(&mut conn, &p.board_id)
            .await
            .map_err(fail)?;
        let keep = |i: &Item| {
            p.frame_id
                .as_deref()
                .is_none_or(|f| i.frame_id.as_deref() == Some(f) || i.id == f)
                && p.kinds.as_ref().is_none_or(|k| k.contains(&i.kind))
        };
        let items: Vec<&Item> = full.items.iter().filter(|i| keep(i)).collect();
        let ids: std::collections::HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
        let connectors: Vec<Value> = full
            .connectors
            .iter()
            .filter(|c| ids.contains(c.from_item.as_str()) || ids.contains(c.to_item.as_str()))
            .map(|c| {
                json!({ "id": c.id, "from": c.from_item, "to": c.to_item, "label": c.label,
                        "from_anchor": c.from_anchor, "to_anchor": c.to_anchor, "style": c.style })
            })
            .collect();
        let tables: Vec<Value> = full
            .tables
            .iter()
            .map(|t| json!({ "id": t.id, "name": t.name, "columns": t.definition.columns.len(), "version": t.version }))
            .collect();
        ok(json!({
            "board": full.board,
            "items": items.iter().map(|i| summary(i)).collect::<Vec<_>>(),
            "connectors": connectors,
            "tables": tables,
        }))
    }

    #[tool(description = "One item in full, with its version.")]
    async fn get_item(&self, Parameters(p): Parameters<ItemId>) -> ToolResult {
        let mut conn = self.conn().await?;
        ok(json!(
            items::get(&mut conn, &p.item_id).await.map_err(fail)?
        ))
    }

    #[tool(
        description = "Search item titles and text across boards (or one board). Matches word prefixes."
    )]
    async fn search(&self, Parameters(p): Parameters<Search>) -> ToolResult {
        let hits = search::search(&self.state.pool, &p.query, p.board_id.as_deref())
            .await
            .map_err(fail)?;
        ok(json!(hits))
    }

    #[tool(description = ADD_ITEMS_DESCRIPTION)]
    async fn add_items(
        &self,
        Parameters(p): Parameters<AddItems>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        if p.layout.as_deref() == Some("auto") {
            return self.add_flow_from_items(who, p).await;
        }
        let mut ops = Vec::new();
        for (i, item) in p.items.into_iter().enumerate() {
            ops.push(Op::CreateItem {
                reference: Some(item.reference.unwrap_or_else(|| format!("item{i}"))),
                kind: item.kind,
                card_type: item.card_type,
                title: item.title,
                content: item.content,
                style: item.style,
                x: item.x,
                y: item.y,
                w: item.w,
                h: item.h,
                z: None,
            });
        }
        let item_count = ops.len();
        for (i, c) in p.connectors.into_iter().enumerate() {
            ops.push(Op::CreateConnector {
                reference: Some(format!("connector{i}")),
                from: c.from,
                to: c.to,
                from_anchor: c.from_anchor,
                to_anchor: c.to_anchor,
                label: c.label,
                style: c.style,
            });
        }
        let result = run_batch(&self.state, who, None, &p.board_id, ops)
            .await
            .map_err(fail)?;
        let (made, links) = result.results.split_at(item_count);
        ok(json!({
            "refs": result.refs,
            "items": made.iter().map(|v| json!({ "id": v["id"], "kind": v["kind"], "x": v["x"], "y": v["y"], "w": v["w"], "h": v["h"], "frame_id": v["frame_id"], "version": v["version"] })).collect::<Vec<_>>(),
            "connectors": links.iter().map(|v| v["id"].clone()).collect::<Vec<_>>(),
        }))
    }

    #[tool(description = DRAW_FLOW_DESCRIPTION)]
    async fn draw_flow(
        &self,
        Parameters(p): Parameters<DrawFlow>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let req = FlowRequest {
            mermaid: Some(p.mermaid),
            direction: None,
            nodes: Vec::new(),
            edges: Vec::new(),
            frame_title: p.frame_title,
            near: p.near,
        };
        self.create_flow(who, &p.board_id, req).await
    }

    #[tool(
        description = "A frame's drawing as Mermaid: the compact way to read a flow someone drew by hand. Shapes keep their shape; other items appear as boxes."
    )]
    async fn get_flow(&self, Parameters(p): Parameters<FrameId>) -> ToolResult {
        let mut conn = self.conn().await?;
        let frame = items::get(&mut conn, &p.frame_id).await.map_err(fail)?;
        let mermaid = flows::frame_to_mermaid(&mut conn, &frame.board_id, &p.frame_id)
            .await
            .map_err(fail)?;
        Ok(mermaid)
    }

    #[tool(
        description = "Change an item's title, content or style. Needs the item's current version; if someone changed it since, you get a conflict with the current item to merge from. Content replaces the whole content (same shapes as add_items); style null clears it."
    )]
    async fn update_item(
        &self,
        Parameters(p): Parameters<UpdateItem>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let board = self.board_of_item(&p.item_id).await?;
        let op = Op::UpdateItem {
            item: p.item_id,
            version: p.version,
            title: p.title,
            content: p.content,
            style: p.style.map(|s| if s.is_null() { None } else { Some(s) }),
        };
        let mut result = run_batch(&self.state, who, None, &board, vec![op])
            .await
            .map_err(fail)?;
        ok(result.results.pop().unwrap_or(Value::Null))
    }

    #[tool(
        description = "Move or resize items on their board. Moving a frame moves what's inside it."
    )]
    async fn set_geometry(
        &self,
        Parameters(p): Parameters<SetGeometry>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let Some(first) = p.updates.first() else {
            return Err(fail(AppError::invalid("updates", "nothing to change")));
        };
        let board = self.board_of_item(&first.item_id).await?;
        let ops = p
            .updates
            .into_iter()
            .map(|g| Op::SetGeometry {
                item: g.item_id,
                x: g.x,
                y: g.y,
                w: g.w,
                h: g.h,
                z: None,
                locked: None,
            })
            .collect();
        let result = run_batch(&self.state, who, None, &board, ops)
            .await
            .map_err(fail)?;
        ok(json!({ "moved": result.results.len(), "also_moved": result.affected.len() }))
    }

    #[tool(
        description = "Move items to another board. Frames take their contents. Connectors between moved items come along; connectors to items left behind are deleted."
    )]
    async fn move_items_to_board(
        &self,
        Parameters(p): Parameters<MoveToBoard>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let mut tx = db::write_tx(&self.state.pool)
            .await
            .map_err(|e| fail(e.into()))?;
        let moved = moves::move_items(
            &mut tx,
            ctx(&self.state, who),
            MoveRequest {
                item_ids: p.item_ids,
                board_id: p.board_id,
                dx: p.dx.unwrap_or(0.0),
                dy: p.dy.unwrap_or(0.0),
            },
        )
        .await
        .map_err(fail)?;
        tx.commit().await.map_err(|e| fail(e.into()))?;
        // Both boards' open tabs reload rather than patching a cross-board move.
        let resync = |b: &str| {
            self.state.hub.publish(
                b,
                None,
                vec![crate::events::Change::removed("resync", String::new())],
            )
        };
        resync(&moved.from_board);
        resync(&moved.to_board);
        ok(
            json!({ "moved": moved.items.len(), "connectors_moved": moved.connectors.len(), "connectors_removed": moved.removed_connectors.len() }),
        )
    }

    #[tool(
        description = "Move items to the trash (the user can restore them). Refuses items the user edited in the last 5 minutes unless force is true. For frames, with_contents also deletes what's inside."
    )]
    async fn delete_items(
        &self,
        Parameters(p): Parameters<DeleteItems>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let Some(first) = p.item_ids.first() else {
            return Err(fail(AppError::invalid("item_ids", "nothing to delete")));
        };
        let board = self.board_of_item(first).await?;
        if !p.force {
            let cutoff = self.state.clock.now().minus(USER_EDIT_GUARD).to_db();
            let mut conn = self.conn().await?;
            let mut recent = Vec::new();
            for id in &p.item_ids {
                let i = items::get(&mut conn, id).await.map_err(fail)?;
                if i.updated_by == "user" && i.content_updated_at > cutoff {
                    recent.push(i.title.clone().unwrap_or_else(|| id.clone()));
                }
            }
            if !recent.is_empty() {
                return Err(fail(AppError::State {
                    code: "recently_edited",
                    message: format!(
                        "the user edited {} in the last 5 minutes; check with them, or pass force: true",
                        recent.join(", ")
                    ),
                }));
            }
        }
        let ops = p
            .item_ids
            .into_iter()
            .map(|id| Op::DeleteItem {
                item: id,
                with_contents: p.with_contents,
            })
            .collect();
        let result = run_batch(&self.state, who, None, &board, ops)
            .await
            .map_err(fail)?;
        ok(json!({ "deleted": result.results }))
    }

    #[tool(description = "Join two items on the same board with a connector.")]
    async fn connect(
        &self,
        Parameters(p): Parameters<NewConnectorArg>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let board = self.board_of_item(&p.from).await?;
        let op = Op::CreateConnector {
            reference: None,
            from: p.from,
            to: p.to,
            from_anchor: p.from_anchor,
            to_anchor: p.to_anchor,
            label: p.label,
            style: p.style,
        };
        let mut result = run_batch(&self.state, who, None, &board, vec![op])
            .await
            .map_err(fail)?;
        ok(result.results.pop().unwrap_or(Value::Null))
    }

    #[tool(description = "Change a connector's label, anchors or style.")]
    async fn update_connector(
        &self,
        Parameters(p): Parameters<UpdateConnector>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let board = self.board_of_connector(&p.connector_id).await?;
        let op = Op::UpdateConnector {
            connector: p.connector_id,
            from: None,
            to: None,
            from_anchor: p.from_anchor,
            to_anchor: p.to_anchor,
            label: p.label.map(Some),
            style: p.style.map(|s| if s.is_null() { None } else { Some(s) }),
        };
        let mut result = run_batch(&self.state, who, None, &board, vec![op])
            .await
            .map_err(fail)?;
        ok(result.results.pop().unwrap_or(Value::Null))
    }

    #[tool(description = "Delete a connector (it goes to the trash).")]
    async fn delete_connector(
        &self,
        Parameters(p): Parameters<ConnectorId>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let board = self.board_of_connector(&p.connector_id).await?;
        let op = Op::DeleteConnector {
            connector: p.connector_id,
        };
        run_batch(&self.state, who, None, &board, vec![op])
            .await
            .map_err(fail)?;
        ok(json!({ "deleted": true }))
    }

    #[tool(
        description = "See a frame, a schema card (as its ER diagram) or any item as an image: returns the PNG, plus a signed link (valid 7 days) you can share. Use it to check a flow or schema you made looks right."
    )]
    async fn get_item_image(
        &self,
        Parameters(p): Parameters<ItemImage>,
    ) -> Result<CallToolResult, String> {
        let mut conn = self.conn().await?;
        let scene = crate::render::scene(&mut conn, &p.item_id)
            .await
            .map_err(fail)?;
        drop(conn);
        let scale = p.scale.unwrap_or(1.0);
        if !(1.0..=4.0).contains(&scale) {
            return Err(fail(AppError::invalid("scale", "between 1 and 4")));
        }
        let req = crate::render::Request {
            scale,
            ..Default::default()
        };
        let cache = self.state.renders.clone();
        let png = tokio::task::spawn_blocking(move || crate::render::draw(&cache, &scene, req))
            .await
            .map_err(|e| e.to_string())?
            .map_err(fail)?;
        let link = crate::routes::images::image_link(
            &self.state,
            &p.item_id,
            &crate::routes::images::LinkBody {
                format: "png".into(),
                scale: Some(2.0),
                theme: None,
                title: None,
                days: None,
            },
        )
        .map_err(fail)?;
        Ok(CallToolResult::success(vec![
            ContentBlock::image(
                base64::engine::general_purpose::STANDARD.encode(png.as_ref()),
                "image/png",
            ),
            ContentBlock::text(format!(
                "Shareable link (expires {}): {}",
                link["expires_at"].as_str().unwrap_or(""),
                link["url"].as_str().unwrap_or("")
            )),
        ]))
    }

    #[tool(
        description = "An item's (or table's) saved versions, newest first, with who made each. Give item_id or table_id."
    )]
    async fn list_revisions(&self, Parameters(p): Parameters<Revisions>) -> ToolResult {
        let mut conn = self.conn().await?;
        match (p.item_id, p.table_id) {
            (Some(item), None) => ok(json!(
                items::revisions(&mut conn, &item).await.map_err(fail)?
            )),
            (None, Some(table)) => ok(json!(
                tables::revisions(&mut conn, &table).await.map_err(fail)?
            )),
            _ => Err(fail(AppError::invalid(
                "item_id",
                "give item_id or table_id",
            ))),
        }
    }

    #[tool(
        description = "Make the board's planning PDF through Typeset, in the board's client's brand: a cover, contents, then each frame (its drawing, then the cards in it) and each loose card (features, schemas with their ER diagram and column tables, sitemaps, notes, checklists), in reading order. Checks it renders and returns a signed link to the PDF (valid 7 days, renders the board as it is when opened) and the page count. Only frames and cards go in: put drawings in a frame."
    )]
    async fn export_pdf(&self, Parameters(p): Parameters<PdfArgs>) -> ToolResult {
        let (board_id, opts) = p.options();
        let proof = crate::routes::pdf::proof(&self.state, &board_id, opts.clone())
            .await
            .map_err(fail)?;
        let link =
            crate::routes::pdf::pdf_link(&self.state, &board_id, &opts, None).map_err(fail)?;
        ok(json!({
            "url": link["url"],
            "expires_at": link["expires_at"],
            "pages": proof["pages"],
            "warnings": proof["warnings"],
        }))
    }

    #[tool(
        description = "The board as a Typeset document (JSON, schema 2) with its drawings as signed image links, for joining with other documents (e.g. Design's screens, which are `page` blocks) before rendering one combined PDF with Typeset. Pass `part` to start it with a part divider. To just make the board's PDF, use export_pdf."
    )]
    async fn get_typeset_document(&self, Parameters(p): Parameters<PdfArgs>) -> ToolResult {
        let (board_id, opts) = p.options();
        let doc = crate::routes::pdf::build(
            &self.state,
            &board_id,
            opts,
            crate::routes::pdf::Images::Links,
        )
        .await
        .map_err(fail)?;
        ok(json!({ "document": doc.json, "warnings": doc.warnings }))
    }

    #[tool(
        description = "The board model: the plan as Design reads it. Tables (labels filled in, a hash per column), pages from the sitemap, features, flows (frames with arrows, as nodes and edges) and notes, in reading order. Use it from another project to read the plan."
    )]
    async fn get_board_model(&self, Parameters(p): Parameters<BoardId>) -> ToolResult {
        let mut conn = self.conn().await?;
        ok(json!(
            crate::model::load(&mut conn, &p.board_id)
                .await
                .map_err(fail)?
        ))
    }

    #[tool(
        description = "The board's database schema: every table with its columns (ids and names), indexes, version, and the board's schema_version."
    )]
    async fn get_schema(&self, Parameters(p): Parameters<BoardId>) -> ToolResult {
        let mut conn = self.conn().await?;
        let board = boards::get(&mut conn, &p.board_id).await.map_err(fail)?;
        let list = tables::list_live(&mut conn, &p.board_id)
            .await
            .map_err(fail)?;
        ok(json!({ "schema_version": board.schema_version, "tables": list }))
    }

    #[tool(description = TABLE_DESCRIPTION)]
    async fn add_table(
        &self,
        Parameters(p): Parameters<AddTable>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let mut tx = db::write_tx(&self.state.pool)
            .await
            .map_err(|e| fail(e.into()))?;
        let written = tables::create(
            &mut tx,
            ctx(&self.state, who),
            &p.board_id,
            tables::NewTable {
                name: p.name,
                definition: p.definition.unwrap_or_else(|| json!({})),
                item_id: p.item_id,
            },
        )
        .await
        .map_err(fail)?;
        tx.commit().await.map_err(|e| fail(e.into()))?;
        crate::routes::schema::publish(&self.state, None, "table.created", &written);
        ok(json!(written))
    }

    #[tool(
        description = "Change a table's name or definition (same shape as add_table; the definition replaces the whole thing). Needs the table's current version. Columns keep their ids when you send the id back or keep the name; a rename must send the id. Removing a column other tables reference is refused."
    )]
    async fn update_table(
        &self,
        Parameters(p): Parameters<UpdateTable>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let mut tx = db::write_tx(&self.state.pool)
            .await
            .map_err(|e| fail(e.into()))?;
        let written = tables::update(
            &mut tx,
            ctx(&self.state, who),
            &p.table_id,
            tables::TablePatch {
                version: p.version,
                name: p.name,
                definition: p.definition,
            },
        )
        .await
        .map_err(fail)?;
        tx.commit().await.map_err(|e| fail(e.into()))?;
        crate::routes::schema::publish(&self.state, None, "table.updated", &written);
        ok(json!(written))
    }

    #[tool(
        description = "Move a table to the trash (the user can restore it). Refused while other tables reference it, or if the user edited it in the last 5 minutes, unless force is true; force also removes the references to it."
    )]
    async fn delete_table(
        &self,
        Parameters(p): Parameters<DeleteTable>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let mut tx = db::write_tx(&self.state.pool)
            .await
            .map_err(|e| fail(e.into()))?;
        if !p.force {
            let t = tables::get(&mut tx, &p.table_id).await.map_err(fail)?;
            let cutoff = self.state.clock.now().minus(USER_EDIT_GUARD).to_db();
            if t.updated_by == "user" && t.content_updated_at > cutoff {
                return Err(fail(AppError::State {
                    code: "recently_edited",
                    message: format!(
                        "the user edited `{}` in the last 5 minutes; check with them, or pass force: true",
                        t.name
                    ),
                }));
            }
        }
        let written = tables::delete(&mut tx, ctx(&self.state, who), &p.table_id, p.force)
            .await
            .map_err(fail)?;
        tx.commit().await.map_err(|e| fail(e.into()))?;
        crate::routes::schema::publish(&self.state, None, "table.deleted", &written);
        ok(json!(written))
    }

    #[tool(
        description = "Put an item (item_id) or table (table_id) back to an earlier revision. Needs its current version."
    )]
    async fn restore_revision(
        &self,
        Parameters(p): Parameters<RestoreRevision>,
        Extension(parts): Extension<Parts>,
    ) -> ToolResult {
        let who = caller(&parts)?;
        let mut tx = db::write_tx(&self.state.pool)
            .await
            .map_err(|e| fail(e.into()))?;
        let item_id = match (p.item_id, p.table_id) {
            (Some(item), None) => item,
            (None, Some(table)) => {
                let written = tables::restore(
                    &mut tx,
                    ctx(&self.state, who),
                    &table,
                    p.revision_id,
                    p.version,
                )
                .await
                .map_err(fail)?;
                tx.commit().await.map_err(|e| fail(e.into()))?;
                crate::routes::schema::publish(&self.state, None, "table.updated", &written);
                return ok(json!(written));
            }
            _ => {
                return Err(fail(AppError::invalid(
                    "item_id",
                    "give item_id or table_id",
                )));
            }
        };
        let item = items::restore(
            &mut tx,
            ctx(&self.state, who),
            &item_id,
            p.revision_id,
            p.version,
        )
        .await
        .map_err(fail)?;
        tx.commit().await.map_err(|e| fail(e.into()))?;
        self.state.hub.publish(
            &item.board_id,
            None,
            vec![crate::events::Change::item("item.updated", item.clone())],
        );
        ok(json!(item))
    }
}

impl BoardMcp {
    async fn board_of_item(&self, id: &str) -> Result<String, String> {
        let mut conn = self.conn().await?;
        items::board_of(&mut conn, id).await.map_err(fail)
    }

    async fn board_of_connector(&self, id: &str) -> Result<String, String> {
        let mut conn = self.conn().await?;
        connectors::board_of(&mut conn, id).await.map_err(fail)
    }

    async fn create_flow(&self, who: Caller, board_id: &str, req: FlowRequest) -> ToolResult {
        let mut tx = db::write_tx(&self.state.pool)
            .await
            .map_err(|e| fail(e.into()))?;
        let (created, changes) = flows::create(&mut tx, ctx(&self.state, who), board_id, req)
            .await
            .map_err(fail)?;
        tx.commit().await.map_err(|e| fail(e.into()))?;
        self.state.hub.publish(board_id, None, changes);
        ok(json!(created))
    }

    /// add_items with layout "auto": shapes become flow nodes, connectors its edges.
    async fn add_flow_from_items(&self, who: Caller, p: AddItems) -> ToolResult {
        let mut nodes = Vec::new();
        let mut refs: HashMap<String, String> = HashMap::new();
        for (i, item) in p.items.into_iter().enumerate() {
            if item.kind != "shape" {
                return Err(fail(AppError::invalid(
                    format!("items[{i}].kind"),
                    "layout \"auto\" lays out shapes only",
                )));
            }
            let shape =
                serde_json::from_value(item.content.get("shape").cloned().unwrap_or(json!("rect")))
                    .map_err(|e| {
                        fail(AppError::invalid(
                            format!("items[{i}].content.shape"),
                            e.to_string(),
                        ))
                    })?;
            let text = item
                .content
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let color = item
                .style
                .as_ref()
                .and_then(|s| s.get("color"))
                .and_then(|c| serde_json::from_value(c.clone()).ok());
            let reference = item.reference.unwrap_or_else(|| format!("item{i}"));
            refs.insert(reference.clone(), reference.clone());
            let page = item
                .content
                .get("page")
                .and_then(Value::as_str)
                .map(str::to_string);
            let page_id = item
                .content
                .get("page_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            nodes.push(FlowNode {
                reference,
                shape: Some(shape),
                text,
                color,
                page,
                page_id,
            });
        }
        let edges = p
            .connectors
            .into_iter()
            .map(|c| {
                let dashed = c
                    .style
                    .as_ref()
                    .and_then(|s| s.get("line"))
                    .and_then(Value::as_str)
                    == Some("dashed");
                let arrow = c
                    .style
                    .as_ref()
                    .and_then(|s| s.get("end"))
                    .and_then(Value::as_str)
                    != Some("none");
                FlowEdge {
                    from: c.from,
                    to: c.to,
                    label: c.label,
                    dashed,
                    arrow,
                }
            })
            .collect();
        let req = FlowRequest {
            mermaid: None,
            direction: p.direction,
            nodes,
            edges,
            frame_title: p.frame_title,
            near: p.near,
        };
        self.create_flow(who, &p.board_id, req).await
    }
}

/// An item for get_board, with long content cut down.
fn summary(i: &Item) -> Value {
    let mut v = json!({
        "id": i.id, "kind": i.kind, "card_type": i.card_type, "title": i.title,
        "content": i.content, "style": i.style, "x": i.x, "y": i.y, "w": i.w, "h": i.h,
        "frame_id": i.frame_id, "locked": i.locked, "version": i.version, "updated_by": i.updated_by,
    });
    let size = i.content.to_string().len();
    if size > TRUNCATE_BYTES {
        let text = i.content.to_string();
        let cut: String = text.chars().take(TRUNCATE_BYTES).collect();
        v["content"] = json!(cut);
        v["truncated"] = json!(true);
    }
    v
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for BoardMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("board", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// `/mcp`: the MCP service behind bearer-token auth (admin scope only).
pub fn service(state: AppState) -> Router {
    let mcp_state = state.clone();
    let mut hosts = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    // Behind a proxy, requests arrive for the public host; allow it (with and
    // without its port), or rmcp's DNS-rebinding guard would refuse them.
    if let Some(authority) = state.config.public_origin.split("://").nth(1) {
        hosts.push(authority.to_string());
        if let Some((host, _port)) = authority.split_once(':') {
            hosts.push(host.to_string());
        }
    }
    let config = StreamableHttpServerConfig::default().with_allowed_hosts(hosts);
    let mcp = StreamableHttpService::new(
        move || Ok(BoardMcp::new(mcp_state.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    Router::new()
        .fallback_service(mcp)
        .layer(middleware::from_fn(require_admin))
        .layer(middleware::from_fn_with_state(state, auth::require_caller))
}

async fn require_admin(
    AxumExtension(caller): AxumExtension<Caller>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    if caller.scope != Scope::Admin {
        return Err(AppError::Forbidden("MCP needs an admin token"));
    }
    Ok(next.run(request).await)
}
