//! Flows from Claude (spec section 5.3): nodes and edges (or Mermaid) in,
//! laid-out shapes, connectors and frames out, as one batch. And the reverse:
//! a frame's drawing as Mermaid, the compact way to read a hand-drawn flow.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::SqliteConnection;

use crate::content::{Color, ShapeContent, ShapeKind};
use crate::error::{AppError, AppResult};
use crate::events::Change;
use crate::flow::layout::{self, Size};
use crate::flow::mermaid::{self, Direction, Flow, Line, OutEdge, OutNode};
use crate::service::batch::{self, Batch, Op};
use crate::service::connectors;
use crate::service::items::{self, Item};
use crate::service::placement::{self, Rect};
use crate::types::Ctx;

/// Padding inside a frame wrapped round a flow.
const FRAME_PAD: f64 = 40.0;
/// Space between a `near` item and the new flow.
const NEAR_GAP: f64 = 120.0;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowRequest {
    /// A Mermaid flowchart. Give this or `nodes`, not both.
    #[serde(default)]
    pub mermaid: Option<String>,
    /// `TB` (default), `LR`, `BT` or `RL`, for `nodes`.
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub nodes: Vec<FlowNode>,
    #[serde(default)]
    pub edges: Vec<FlowEdge>,
    /// Wrap the flow in a frame with this title (ignored when Mermaid
    /// subgraphs already make frames: frames don't nest).
    #[serde(default)]
    pub frame_title: Option<String>,
    /// Place the flow beside this item instead of in the next free space.
    #[serde(default)]
    pub near: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowNode {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default)]
    pub shape: Option<ShapeKind>,
    pub text: String,
    #[serde(default)]
    pub color: Option<Color>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowEdge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub dashed: bool,
    #[serde(default = "yes")]
    pub arrow: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct FlowCreated {
    /// Frames created (one per subgraph, or the `frame_title` frame).
    pub frames: Vec<String>,
    /// New item ids, keyed by node ref (or Mermaid id).
    pub items: HashMap<String, String>,
    pub connectors: Vec<String>,
}

/// Lay out and create a flow. Returns what was created and the changes to publish.
pub async fn create(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    board_id: &str,
    req: FlowRequest,
) -> AppResult<(FlowCreated, Vec<Change>)> {
    let flow = to_flow(&req)?;
    let colors: HashMap<&str, Color> = req
        .nodes
        .iter()
        .filter_map(|n| n.color.map(|c| (n.reference.as_str(), c)))
        .collect();

    // Sizes: shape defaults, widened for long labels.
    let sizes: Vec<Size> = flow
        .nodes
        .iter()
        .map(|n| {
            let (w, h) = n.shape.default_size();
            let chars = n.text.chars().count() as f64;
            // About two lines of semibold text at the canvas size.
            let wanted = (chars * 8.5 / 2.0 + 48.0).min(300.0);
            Size {
                w: w.max(wanted).round(),
                h,
            }
        })
        .collect();
    let index: HashMap<&str, usize> = flow
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let edges: Vec<(usize, usize)> = flow
        .edges
        .iter()
        .map(|e| (index[e.from.as_str()], index[e.to.as_str()]))
        .collect();
    let positions = layout::layout(&sizes, &edges, flow.direction);

    // Frames: one per subgraph, or one round everything for `frame_title`.
    let mut frames: Vec<(String, Rect)> = Vec::new();
    for (f, title) in flow.frames.iter().enumerate() {
        let members: Vec<usize> = (0..flow.nodes.len())
            .filter(|&i| flow.nodes[i].frame == Some(f))
            .collect();
        if let Some(r) = bounds(&members, &positions, &sizes, FRAME_PAD) {
            frames.push((title.clone(), r));
        }
    }
    if frames.is_empty()
        && let Some(title) = req.frame_title.as_deref().filter(|t| !t.trim().is_empty())
    {
        let all: Vec<usize> = (0..flow.nodes.len()).collect();
        if let Some(r) = bounds(&all, &positions, &sizes, FRAME_PAD) {
            frames.push((title.trim().to_string(), r));
        }
    }

    // The whole group's box, relative to the layout origin.
    let mut group = bounds(
        &(0..flow.nodes.len()).collect::<Vec<_>>(),
        &positions,
        &sizes,
        0.0,
    )
    .expect("a flow has nodes");
    for (_, r) in &frames {
        group = union(&group, r);
    }
    let (ox, oy) = origin(conn, board_id, req.near.as_deref(), &group).await?;
    let (dx, dy) = (ox - group.x, oy - group.y);

    // One batch: frames, then nodes (so they join their frames), then connectors.
    let mut ops = Vec::new();
    for (k, (title, r)) in frames.iter().enumerate() {
        ops.push(Op::CreateItem {
            reference: Some(format!("frame{k}")),
            kind: "frame".into(),
            card_type: None,
            title: Some(title.clone()),
            content: json!({}),
            style: None,
            x: Some(r.x + dx),
            y: Some(r.y + dy),
            w: Some(r.w),
            h: Some(r.h),
            z: None,
        });
    }
    for (i, n) in flow.nodes.iter().enumerate() {
        let content = serde_json::to_value(ShapeContent {
            shape: n.shape,
            text: n.text.clone(),
        })
        .map_err(|e| AppError::Internal(e.into()))?;
        ops.push(Op::CreateItem {
            reference: Some(format!("n{i}")),
            kind: "shape".into(),
            card_type: None,
            title: None,
            content,
            style: colors.get(n.id.as_str()).map(|c| json!({ "color": c })),
            x: Some(positions[i].0 + dx),
            y: Some(positions[i].1 + dy),
            w: Some(sizes[i].w),
            h: Some(sizes[i].h),
            z: None,
        });
    }
    for (k, e) in flow.edges.iter().enumerate() {
        let mut style = serde_json::Map::new();
        if e.line == Line::Dashed {
            style.insert("line".into(), json!("dashed"));
        }
        if !e.arrow {
            style.insert("end".into(), json!("none"));
        }
        ops.push(Op::CreateConnector {
            reference: Some(format!("c{k}")),
            from: format!("n{}", index[e.from.as_str()]),
            to: format!("n{}", index[e.to.as_str()]),
            from_anchor: None,
            to_anchor: None,
            label: e.label.clone(),
            style: if style.is_empty() {
                None
            } else {
                Some(Value::Object(style))
            },
        });
    }

    let result = batch::apply(conn, ctx, board_id, Batch { ops }).await?;
    let created = FlowCreated {
        frames: (0..frames.len())
            .map(|k| result.refs[&format!("frame{k}")].clone())
            .collect(),
        items: flow
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), result.refs[&format!("n{i}")].clone()))
            .collect(),
        connectors: (0..flow.edges.len())
            .map(|k| result.refs[&format!("c{k}")].clone())
            .collect(),
    };
    Ok((created, result.changes))
}

/// Turn the request into a parsed flow, from Mermaid or from nodes and edges.
fn to_flow(req: &FlowRequest) -> AppResult<Flow> {
    match (&req.mermaid, req.nodes.is_empty()) {
        (Some(_), false) => Err(AppError::invalid(
            "mermaid",
            "give `mermaid` or `nodes`, not both",
        )),
        (None, true) => Err(AppError::invalid(
            "nodes",
            "a flow needs `nodes` (or `mermaid`)",
        )),
        (Some(src), true) => {
            mermaid::parse(src).map_err(|e| AppError::invalid("mermaid", e.to_string()))
        }
        (None, false) => {
            if req.nodes.len() > mermaid::MAX_NODES {
                return Err(AppError::invalid(
                    "nodes",
                    format!("at most {} nodes", mermaid::MAX_NODES),
                ));
            }
            let direction = match req.direction.as_deref() {
                None => Direction::TopBottom,
                Some(d) => Direction::parse(d)
                    .ok_or_else(|| AppError::invalid("direction", "use TB, LR, BT or RL"))?,
            };
            let mut seen = std::collections::HashSet::new();
            let mut nodes = Vec::with_capacity(req.nodes.len());
            for (i, n) in req.nodes.iter().enumerate() {
                if !seen.insert(n.reference.as_str()) {
                    return Err(AppError::invalid(
                        format!("nodes[{i}].ref"),
                        format!("ref `{}` is used twice", n.reference),
                    ));
                }
                nodes.push(mermaid::Node {
                    id: n.reference.clone(),
                    shape: n.shape.unwrap_or(ShapeKind::Rect),
                    text: n.text.clone(),
                    frame: None,
                });
            }
            let mut edges = Vec::with_capacity(req.edges.len());
            for (i, e) in req.edges.iter().enumerate() {
                for (field, r) in [("from", &e.from), ("to", &e.to)] {
                    if !seen.contains(r.as_str()) {
                        return Err(AppError::invalid(
                            format!("edges[{i}].{field}"),
                            format!("no node with ref `{r}`"),
                        ));
                    }
                }
                edges.push(mermaid::Edge {
                    from: e.from.clone(),
                    to: e.to.clone(),
                    label: e.label.clone(),
                    line: if e.dashed { Line::Dashed } else { Line::Solid },
                    arrow: e.arrow,
                });
            }
            Ok(Flow {
                direction,
                nodes,
                edges,
                frames: Vec::new(),
            })
        }
    }
}

fn bounds(members: &[usize], pos: &[(f64, f64)], sizes: &[Size], pad: f64) -> Option<Rect> {
    if members.is_empty() {
        return None;
    }
    let x = members.iter().map(|&i| pos[i].0).fold(f64::MAX, f64::min) - pad;
    let y = members.iter().map(|&i| pos[i].1).fold(f64::MAX, f64::min) - pad;
    let r = members
        .iter()
        .map(|&i| pos[i].0 + sizes[i].w)
        .fold(f64::MIN, f64::max)
        + pad;
    let b = members
        .iter()
        .map(|&i| pos[i].1 + sizes[i].h)
        .fold(f64::MIN, f64::max)
        + pad;
    Some(Rect {
        x,
        y,
        w: r - x,
        h: b - y,
    })
}

fn union(a: &Rect, b: &Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect {
        x,
        y,
        w: (a.x + a.w).max(b.x + b.w) - x,
        h: (a.y + a.h).max(b.y + b.h) - y,
    }
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Where the group's top-left goes: beside `near` (right, then below) if
/// that's free, otherwise the next free space (spec section 7).
async fn origin(
    conn: &mut SqliteConnection,
    board_id: &str,
    near: Option<&str>,
    group: &Rect,
) -> AppResult<(f64, f64)> {
    let existing: Vec<Rect> = items::list_live(conn, board_id)
        .await?
        .iter()
        .map(|i| Rect {
            x: i.x,
            y: i.y,
            w: i.w,
            h: i.h,
        })
        .collect();
    if let Some(near_id) = near {
        let n = items::get(conn, near_id)
            .await
            .map_err(|_| AppError::invalid("near", "no such item"))?;
        if n.board_id != board_id {
            return Err(AppError::invalid("near", "that item is on another board"));
        }
        for (x, y) in [(n.x + n.w + NEAR_GAP, n.y), (n.x, n.y + n.h + NEAR_GAP)] {
            let candidate = Rect {
                x,
                y,
                w: group.w,
                h: group.h,
            };
            if !existing.iter().any(|r| overlaps(r, &candidate)) {
                return Ok((x.round(), y.round()));
            }
        }
    }
    Ok(placement::place(&existing, group.w, group.h))
}

/// A frame's shapes and the connectors between them, as Mermaid.
pub async fn frame_to_mermaid(
    conn: &mut SqliteConnection,
    board_id: &str,
    frame_id: &str,
) -> AppResult<String> {
    let frame = items::get(conn, frame_id).await?;
    if frame.board_id != board_id || frame.kind != "frame" {
        return Err(AppError::NotFound("frame"));
    }
    let members: Vec<Item> = items::list_live(conn, board_id)
        .await?
        .into_iter()
        .filter(|i| i.frame_id.as_deref() == Some(frame_id))
        .collect();
    let ids: std::collections::HashSet<&str> = members.iter().map(|i| i.id.as_str()).collect();
    let conns: Vec<connectors::Connector> = connectors::list_live(conn, board_id)
        .await?
        .into_iter()
        .filter(|c| ids.contains(c.from_item.as_str()) && ids.contains(c.to_item.as_str()))
        .collect();

    // Shapes keep their shape; anything else becomes a box with its text or title.
    let texts: Vec<(ShapeKind, String)> = members.iter().map(node_of).collect();
    let nodes: Vec<OutNode> = members
        .iter()
        .zip(&texts)
        .map(|(i, (shape, text))| OutNode {
            id: &i.id,
            shape: *shape,
            text,
        })
        .collect();
    let edges: Vec<OutEdge> = conns
        .iter()
        .map(|c| {
            let style = c.style.as_ref();
            OutEdge {
                from: &c.from_item,
                to: &c.to_item,
                label: c.label.as_deref(),
                dashed: style.and_then(|s| s.get("line")).and_then(Value::as_str) == Some("dashed"),
                arrow: style.and_then(|s| s.get("end")).and_then(Value::as_str) != Some("none"),
            }
        })
        .collect();

    // Guess the direction from how the connectors run.
    let pos: HashMap<&str, &Item> = members.iter().map(|i| (i.id.as_str(), i)).collect();
    let (mut across, mut down) = (0.0, 0.0);
    for c in &conns {
        let (a, b) = (pos[c.from_item.as_str()], pos[c.to_item.as_str()]);
        across += ((b.x + b.w / 2.0) - (a.x + a.w / 2.0)).abs();
        down += ((b.y + b.h / 2.0) - (a.y + a.h / 2.0)).abs();
    }
    let direction = if across > down {
        Direction::LeftRight
    } else {
        Direction::TopBottom
    };
    Ok(mermaid::to_mermaid(direction, &nodes, &edges))
}

fn node_of(i: &Item) -> (ShapeKind, String) {
    if i.kind == "shape"
        && let Ok(c) = serde_json::from_value::<ShapeContent>(i.content.clone())
    {
        return (c.shape, c.text);
    }
    let text = i
        .title
        .clone()
        .or_else(|| {
            i.content
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    (ShapeKind::Rect, text)
}
