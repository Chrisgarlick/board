//! A Mermaid flowchart subset, in both directions (spec section 5.3).
//!
//! Supported:
//! - `flowchart TD|TB|LR|RL|BT` (or `graph …`); optional, defaults to TB
//! - Nodes: `A[text]` rect, `A(text)` rounded, `A{text}` diamond,
//!   `A((text))` ellipse, `A[/text/]` parallelogram, `A[(text)]` cylinder.
//!   Text may be quoted: `A["text, with punctuation"]`. A bare `A` is a
//!   rect labelled `A`.
//! - Edges: `-->`, `---` (no arrow), `-.->` / `-.-` (dashed), `==>` / `===`
//!   (solid), labels as `-->|text|` or `-- text -->`, and chains
//!   (`A --> B --> C`)
//! - `subgraph Title … end`, one level deep, becomes a frame
//! - `%%` comments; `;` separates statements
//!
//! Anything else is an error naming the line, so the caller can fix it.

use std::collections::HashMap;

use crate::content::ShapeKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    TopBottom,
    BottomTop,
    LeftRight,
    RightLeft,
}

impl Direction {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "TB" | "TD" => Some(Self::TopBottom),
            "BT" => Some(Self::BottomTop),
            "LR" => Some(Self::LeftRight),
            "RL" => Some(Self::RightLeft),
            _ => None,
        }
    }

    pub fn as_mermaid(self) -> &'static str {
        match self {
            Self::TopBottom => "TB",
            Self::BottomTop => "BT",
            Self::LeftRight => "LR",
            Self::RightLeft => "RL",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// The Mermaid id (`A` in `A[text]`).
    pub id: String,
    pub shape: ShapeKind,
    pub text: String,
    /// Index into `Flow::frames`.
    pub frame: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    Solid,
    Dashed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub label: Option<String>,
    pub line: Line,
    pub arrow: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Flow {
    pub direction: Direction,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Frame titles, from subgraphs.
    pub frames: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    /// 1-based line number.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

pub const MAX_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_NODES: usize = 500;

pub fn parse(source: &str) -> Result<Flow, ParseError> {
    if source.len() > MAX_INPUT_BYTES {
        return Err(ParseError {
            line: 1,
            message: format!("input is larger than {} KB", MAX_INPUT_BYTES / 1024),
        });
    }

    let mut flow = Flow {
        direction: Direction::TopBottom,
        nodes: Vec::new(),
        edges: Vec::new(),
        frames: Vec::new(),
    };
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut frame: Option<usize> = None;
    let mut seen_header = false;

    for (n, raw) in source.lines().enumerate() {
        let line = n + 1;
        let err = |message: String| ParseError { line, message };
        let text = match raw.find("%%") {
            Some(at) => &raw[..at],
            None => raw,
        };
        for stmt in split_statements(text) {
            let stmt = stmt.trim();
            if stmt.is_empty() {
                continue;
            }
            let first = stmt.split_whitespace().next().unwrap_or("");

            if !seen_header && (first == "flowchart" || first == "graph") {
                seen_header = true;
                let dir = stmt.split_whitespace().nth(1).unwrap_or("TB");
                flow.direction = Direction::parse(dir).ok_or_else(|| {
                    err(format!(
                        "unknown direction `{dir}`; use TD, TB, LR, RL or BT"
                    ))
                })?;
                continue;
            }
            seen_header = true;

            if first == "subgraph" {
                if frame.is_some() {
                    return Err(err(
                        "subgraphs can't nest (frames are one level deep)".into()
                    ));
                }
                let title = subgraph_title(stmt["subgraph".len()..].trim());
                if title.is_empty() {
                    return Err(err("a subgraph needs a title".into()));
                }
                flow.frames.push(title);
                frame = Some(flow.frames.len() - 1);
                continue;
            }
            if stmt == "end" {
                if frame.is_none() {
                    return Err(err("`end` without a `subgraph`".into()));
                }
                frame = None;
                continue;
            }
            if matches!(
                first,
                "classDef" | "class" | "style" | "linkStyle" | "click" | "direction"
            ) {
                return Err(err(format!(
                    "`{first}` isn't supported; remove styling and interaction lines"
                )));
            }

            parse_chain(stmt, &mut flow, &mut index, frame).map_err(err)?;
            if flow.nodes.len() > MAX_NODES {
                return Err(err(format!("at most {MAX_NODES} nodes")));
            }
        }
    }
    if frame.is_some() {
        return Err(ParseError {
            line: source.lines().count().max(1),
            message: "a `subgraph` is missing its `end`".into(),
        });
    }
    if flow.nodes.is_empty() {
        return Err(ParseError {
            line: 1,
            message: "no nodes found".into(),
        });
    }
    Ok(flow)
}

/// Split on `;` outside brackets and quotes.
fn split_statements(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut quoted, mut start) = (0i32, false, 0);
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '[' | '(' | '{' if !quoted => depth += 1,
            ']' | ')' | '}' if !quoted => depth -= 1,
            ';' if !quoted && depth <= 0 => {
                out.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&line[start..]);
    out
}

fn subgraph_title(rest: &str) -> String {
    // `subgraph id [Title]`, `subgraph "Title"` or `subgraph Title`.
    if let (Some(open), Some(close)) = (rest.find('['), rest.rfind(']'))
        && open < close
    {
        return unquote(rest[open + 1..close].trim());
    }
    unquote(rest.trim())
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

struct Cursor<'a> {
    s: &'a str,
    at: usize,
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.at..]
    }
    fn skip_ws(&mut self) {
        let trimmed = self.rest().trim_start();
        self.at = self.s.len() - trimmed.len();
    }
    fn eat(&mut self, token: &str) -> bool {
        if self.rest().starts_with(token) {
            self.at += token.len();
            true
        } else {
            false
        }
    }
    fn done(&self) -> bool {
        self.rest().trim().is_empty()
    }
}

/// `A[x] --> B{y} -->|no| C`
fn parse_chain(
    stmt: &str,
    flow: &mut Flow,
    index: &mut HashMap<String, usize>,
    frame: Option<usize>,
) -> Result<(), String> {
    let mut c = Cursor { s: stmt, at: 0 };
    let mut prev = parse_node(&mut c, flow, index, frame)?;
    loop {
        c.skip_ws();
        if c.done() {
            return Ok(());
        }
        let (line, arrow, label) = parse_edge(&mut c)?;
        c.skip_ws();
        let next = parse_node(&mut c, flow, index, frame)?;
        flow.edges.push(Edge {
            from: prev.clone(),
            to: next.clone(),
            label,
            line,
            arrow,
        });
        prev = next;
    }
}

const SHAPES: [(&str, &str, ShapeKind); 6] = [
    ("((", "))", ShapeKind::Ellipse),
    ("[(", ")]", ShapeKind::Cylinder),
    ("[/", "/]", ShapeKind::Parallelogram),
    ("[", "]", ShapeKind::Rect),
    ("(", ")", ShapeKind::Rounded),
    ("{", "}", ShapeKind::Diamond),
];

fn parse_node(
    c: &mut Cursor,
    flow: &mut Flow,
    index: &mut HashMap<String, usize>,
    frame: Option<usize>,
) -> Result<String, String> {
    c.skip_ws();
    let rest = c.rest();
    let id_len = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-'))
        .unwrap_or(rest.len());
    // An id stops where an edge starts: `A-->B`, `A---B`, `A-.->B`.
    let id_len = ["--", "-."]
        .iter()
        .filter_map(|t| rest.find(t))
        .filter(|&p| p < id_len)
        .min()
        .unwrap_or(id_len);
    if id_len == 0 {
        return Err(format!(
            "expected a node id at `{}`",
            rest.chars().take(20).collect::<String>()
        ));
    }
    let id = rest[..id_len].to_string();
    c.at += id_len;

    let mut shape_text: Option<(ShapeKind, String)> = None;
    for (open, close, shape) in SHAPES {
        if c.rest().starts_with(open) {
            c.at += open.len();
            let body = c.rest();
            let end = find_close(body, close)
                .ok_or_else(|| format!("`{id}{open}` is missing its closing `{close}`"))?;
            shape_text = Some((shape, unquote(&body[..end])));
            c.at += end + close.len();
            break;
        }
    }

    match index.get(&id) {
        Some(&i) => {
            if let Some((shape, text)) = shape_text {
                let node = &mut flow.nodes[i];
                node.shape = shape;
                node.text = text;
            }
        }
        None => {
            let (shape, text) = shape_text.unwrap_or((ShapeKind::Rect, id.clone()));
            index.insert(id.clone(), flow.nodes.len());
            flow.nodes.push(Node {
                id: id.clone(),
                shape,
                text,
                frame,
            });
        }
    }
    Ok(id)
}

/// The position of `close` in `body`, skipping anything inside quotes.
fn find_close(body: &str, close: &str) -> Option<usize> {
    let mut quoted = false;
    for (i, ch) in body.char_indices() {
        if ch == '"' {
            quoted = !quoted;
        } else if !quoted && body[i..].starts_with(close) {
            return Some(i);
        }
    }
    None
}

/// Edge tokens: (token, line, arrow). Longest first.
const EDGES: [(&str, Line, bool); 6] = [
    ("-.->", Line::Dashed, true),
    ("-.-", Line::Dashed, false),
    ("-->", Line::Solid, true),
    ("---", Line::Solid, false),
    ("==>", Line::Solid, true),
    ("===", Line::Solid, false),
];

fn parse_edge(c: &mut Cursor) -> Result<(Line, bool, Option<String>), String> {
    for (token, line, arrow) in EDGES {
        if c.eat(token) {
            // `-->|label|`
            c.skip_ws();
            let label = if c.eat("|") {
                let rest = c.rest();
                let end = rest
                    .find('|')
                    .ok_or("a `|label|` is missing its closing `|`")?;
                let label = unquote(&rest[..end]);
                c.at += end + 1;
                Some(label).filter(|l| !l.is_empty())
            } else {
                None
            };
            return Ok((line, arrow, label));
        }
    }
    // `-- label -->`, `-. label .->`, `== label ==>`
    for (open, endings) in [
        ("--", [("-->", true), ("---", false)]),
        ("-.", [(".->", true), (".-", false)]),
        ("==", [("==>", true), ("===", false)]),
    ] {
        if c.rest().starts_with(open) && c.rest()[open.len()..].starts_with(char::is_whitespace) {
            c.at += open.len();
            let rest = c.rest();
            let (pos, token, arrow) = endings
                .iter()
                .filter_map(|(t, a)| rest.find(t).map(|p| (p, *t, *a)))
                .min_by_key(|(p, _, _)| *p)
                .ok_or_else(|| format!("an edge label after `{open}` has no end"))?;
            let label = unquote(rest[..pos].trim());
            c.at += pos + token.len();
            let line = if open == "-." {
                Line::Dashed
            } else {
                Line::Solid
            };
            return Ok((line, arrow, Some(label).filter(|l| !l.is_empty())));
        }
    }
    Err(format!(
        "expected an edge (like `-->`) at `{}`",
        c.rest().trim().chars().take(20).collect::<String>()
    ))
}

// ---------- Back to Mermaid ----------

/// A node for [`to_mermaid`].
pub struct OutNode<'a> {
    pub id: &'a str,
    pub shape: ShapeKind,
    pub text: &'a str,
}

/// An edge for [`to_mermaid`].
pub struct OutEdge<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub label: Option<&'a str>,
    pub dashed: bool,
    pub arrow: bool,
}

/// Write nodes and edges as Mermaid. Ids are replaced with `n1`, `n2`… so
/// the output is compact; text is always quoted.
pub fn to_mermaid(direction: Direction, nodes: &[OutNode], edges: &[OutEdge]) -> String {
    let short: HashMap<&str, String> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, format!("n{}", i + 1)))
        .collect();
    let mut out = format!("flowchart {}\n", direction.as_mermaid());
    for n in nodes {
        let (open, close) = match n.shape {
            ShapeKind::Rect | ShapeKind::Document => ("[", "]"),
            ShapeKind::Rounded => ("(", ")"),
            ShapeKind::Diamond => ("{", "}"),
            ShapeKind::Ellipse => ("((", "))"),
            ShapeKind::Parallelogram => ("[/", "/]"),
            ShapeKind::Cylinder => ("[(", ")]"),
        };
        let text = n.text.replace('"', "'").replace('\n', " ");
        out.push_str(&format!("  {}{open}\"{text}\"{close}\n", short[n.id]));
    }
    for e in edges {
        let (Some(from), Some(to)) = (short.get(e.from), short.get(e.to)) else {
            continue;
        };
        let token = match (e.dashed, e.arrow) {
            (true, true) => "-.->",
            (true, false) => "-.-",
            (false, true) => "-->",
            (false, false) => "---",
        };
        match e.label {
            Some(l) if !l.is_empty() => {
                let l = l.replace('|', "/").replace('"', "'");
                out.push_str(&format!("  {from} {token}|{l}| {to}\n"));
            }
            _ => out.push_str(&format!("  {from} {token} {to}\n")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node<'a>(f: &'a Flow, id: &str) -> &'a Node {
        f.nodes.iter().find(|n| n.id == id).unwrap()
    }

    #[test]
    fn every_node_shape() {
        let f = parse(
            "flowchart LR\n  a[Rect] --> b(Rounded) --> c{Decision}\n  d((Event)) --> e[/In/] --> f[(Store)]",
        )
        .unwrap();
        assert_eq!(f.direction, Direction::LeftRight);
        let shapes: Vec<_> = f.nodes.iter().map(|n| (n.id.as_str(), n.shape)).collect();
        assert_eq!(
            shapes,
            vec![
                ("a", ShapeKind::Rect),
                ("b", ShapeKind::Rounded),
                ("c", ShapeKind::Diamond),
                ("d", ShapeKind::Ellipse),
                ("e", ShapeKind::Parallelogram),
                ("f", ShapeKind::Cylinder),
            ]
        );
        assert_eq!(node(&f, "c").text, "Decision");
        assert_eq!(f.edges.len(), 4, "chains make one edge per link");
    }

    #[test]
    fn edge_styles_and_labels() {
        let f = parse(
            "graph TD\nA --> B\nB -->|yes| C\nB -- no --> D\nC -.-> E\nD --- E\nE ==> F\nF -. maybe .-> A",
        )
        .unwrap();
        let e = |from: &str, to: &str| {
            f.edges
                .iter()
                .find(|e| e.from == from && e.to == to)
                .unwrap()
        };
        assert_eq!(e("B", "C").label.as_deref(), Some("yes"));
        assert_eq!(e("B", "D").label.as_deref(), Some("no"));
        assert_eq!(e("C", "E").line, Line::Dashed);
        assert!(!e("D", "E").arrow);
        assert!(e("E", "F").arrow);
        let back = e("F", "A");
        assert_eq!(
            (back.line, back.label.as_deref()),
            (Line::Dashed, Some("maybe"))
        );
        assert_eq!(
            node(&f, "A").text,
            "A",
            "bare ids are labelled with themselves"
        );
    }

    #[test]
    fn quoted_text_and_compact_edges() {
        let f =
            parse("flowchart TB\n  a[\"Pay (card, PayPal)\"]-->b{\"Paid?\"}; b-->c; c---d; d-.->e")
                .unwrap();
        assert_eq!(node(&f, "a").text, "Pay (card, PayPal)");
        assert_eq!(f.edges.len(), 4);
        assert_eq!(f.nodes.len(), 5, "compact edges don't swallow ids");
    }

    #[test]
    fn later_definitions_update_a_node() {
        let f = parse("A --> B\nB{Logged in?}").unwrap();
        assert_eq!(node(&f, "B").shape, ShapeKind::Diamond);
        assert_eq!(node(&f, "B").text, "Logged in?");
        assert_eq!(f.direction, Direction::TopBottom, "header is optional");
    }

    #[test]
    fn subgraphs_become_frames() {
        let f = parse(
            "flowchart TB\nsubgraph Checkout\n  a[Basket] --> b[Pay]\nend\nsubgraph s2 [Account]\n  c[Log in]\nend\nb --> c",
        )
        .unwrap();
        assert_eq!(f.frames, vec!["Checkout", "Account"]);
        assert_eq!(node(&f, "a").frame, Some(0));
        assert_eq!(node(&f, "c").frame, Some(1));
    }

    #[test]
    fn errors_name_the_line() {
        let cases = [
            ("flowchart XY\nA-->B", 1, "unknown direction"),
            ("A --> B\nstyle A fill:#f00", 2, "`style` isn't supported"),
            ("A --> B\nA & C --> D", 2, "expected an edge"),
            ("A[unclosed --> B", 1, "missing its closing"),
            ("subgraph X\nsubgraph Y\nend\nend", 2, "can't nest"),
            ("subgraph X\nA", 2, "missing its `end`"),
            ("A -->", 1, "expected a node id"),
            ("%% just a comment", 1, "no nodes"),
        ];
        for (src, line, msg) in cases {
            let err = parse(src).unwrap_err();
            assert_eq!(err.line, line, "{src:?}: {err}");
            assert!(err.message.contains(msg), "{src:?}: {err}");
        }
    }

    #[test]
    fn round_trip() {
        let nodes = [
            OutNode {
                id: "x1",
                shape: ShapeKind::Rounded,
                text: "Start",
            },
            OutNode {
                id: "x2",
                shape: ShapeKind::Diamond,
                text: "Say \"hi\"?",
            },
            OutNode {
                id: "x3",
                shape: ShapeKind::Cylinder,
                text: "DB",
            },
        ];
        let edges = [
            OutEdge {
                from: "x1",
                to: "x2",
                label: None,
                dashed: false,
                arrow: true,
            },
            OutEdge {
                from: "x2",
                to: "x3",
                label: Some("yes"),
                dashed: true,
                arrow: true,
            },
        ];
        let text = to_mermaid(Direction::LeftRight, &nodes, &edges);
        let back = parse(&text).unwrap();
        assert_eq!(back.direction, Direction::LeftRight);
        assert_eq!(back.nodes[1].text, "Say 'hi'?");
        assert_eq!(back.nodes[2].shape, ShapeKind::Cylinder);
        assert_eq!(back.edges[1].label.as_deref(), Some("yes"));
        assert_eq!(back.edges[1].line, Line::Dashed);
    }

    #[test]
    fn caps_input() {
        assert!(parse(&"A-->B\n".repeat(20_000)).is_err());
    }
}
