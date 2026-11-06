//! Board cards -> Typeset document.
//!
//! Pure functions: no I/O, no clock, no DB. The HTTP layer loads the cards,
//! calls `to_typeset_document`, and either POSTs the result to Typeset (push)
//! or returns it from `GET /api/boards/:id/typeset` (pull).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use typeset_doc::{Block, ChecklistEntry, ChecklistGroup, Meta, TreeNode, TypesetDocument};

// ---------- Card content (stored as JSON in cards.content) ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "content", rename_all = "snake_case")]
pub enum CardContent {
    Note(NoteContent),
    Schema(SchemaContent),
    Sitemap(SitemapContent),
    Checklist(ChecklistContent),
    Diagram(DiagramContent),
}

impl CardContent {
    /// Build from the two DB columns (`type`, `content`). This is also the
    /// validation step on writes: bad shapes fail here with a serde error
    /// that can be returned verbatim to the REST caller or the MCP client.
    pub fn from_parts(
        card_type: &str,
        content: serde_json::Value,
    ) -> Result<Self, serde_json::Error> {
        serde_json::from_value(serde_json::json!({ "type": card_type, "content": content }))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteContent {
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaContent {
    pub tables: Vec<SchemaTable>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaTable {
    pub name: String,
    #[serde(default)]
    pub comment: Option<String>,
    pub columns: Vec<SchemaColumn>,
    #[serde(default)]
    pub indexes: Vec<SchemaIndex>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub col_type: String,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub references: Option<ColumnRef>,
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnRef {
    pub table: String,
    pub column: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaIndex {
    pub columns: Vec<String>,
    #[serde(default)]
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SitemapContent {
    pub root: SitemapNode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SitemapNode {
    pub title: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub children: Vec<SitemapNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChecklistContent {
    pub items: Vec<ChecklistItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub text: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagramContent {
    pub syntax: DiagramSyntax,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagramSyntax {
    Mermaid,
}

// ---------- Board / card rows ----------

#[derive(Debug, Clone)]
pub struct Board {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
}

/// A browser-rendered SVG cached on the card (cards.rendered_svg, cards.svg_for_version).
#[derive(Debug, Clone)]
pub struct StoredSvg {
    pub svg: String,
    /// The card `version` this SVG was rendered from.
    pub for_version: i64,
}

#[derive(Debug, Clone)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub version: i64,
    pub content: CardContent,
    pub svg: Option<StoredSvg>,
}

impl Card {
    /// The cached SVG, only if it was rendered from the current content.
    pub fn fresh_svg(&self) -> Option<&str> {
        self.svg
            .as_ref()
            .filter(|s| s.for_version == self.version)
            .map(|s| s.svg.as_str())
    }
}

// ---------- Export options ----------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportMeta {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub client: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportOptions {
    /// Cards to include, in output order. Unknown ids are skipped.
    pub card_ids: Vec<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub meta: ExportMeta,
    /// Fresh SVGs from the browser, overriding any cached SVG.
    #[serde(default)]
    pub svg_overrides: HashMap<String, String>,
    #[serde(default)]
    pub page_break_between_cards: bool,
}

// ---------- Main entry ----------

pub fn to_typeset_document(
    board: &Board,
    cards: &[Card],
    opts: &ExportOptions,
    today: &str,
) -> TypesetDocument {
    let by_id: HashMap<&str, &Card> = cards.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut blocks = Vec::new();
    let mut first = true;

    for id in &opts.card_ids {
        let Some(card) = by_id.get(id.as_str()) else {
            continue;
        };

        if !first && opts.page_break_between_cards {
            blocks.push(Block::PageBreak);
        }
        first = false;

        blocks.push(Block::Heading {
            level: 2,
            text: card.title.clone(),
        });

        let svg = opts
            .svg_overrides
            .get(&card.id)
            .map(String::as_str)
            .or_else(|| card.fresh_svg());
        blocks.extend(card_blocks(&card.content, svg));
    }

    let m = &opts.meta;
    TypesetDocument {
        template: opts
            .template
            .clone()
            .unwrap_or_else(|| "planning-doc".to_string()),
        meta: Meta {
            title: m.title.clone().unwrap_or_else(|| board.title.clone()),
            subtitle: m.subtitle.clone().or_else(|| board.description.clone()),
            client: m.client.clone(),
            author: m.author.clone(),
            date: today.to_string(),
            version: m.version.clone(),
        },
        blocks,
    }
}

fn card_blocks(content: &CardContent, svg: Option<&str>) -> Vec<Block> {
    match content {
        CardContent::Note(c) => note_blocks(c),
        CardContent::Schema(c) => schema_blocks(c, svg),
        CardContent::Sitemap(c) => sitemap_blocks(c),
        CardContent::Checklist(c) => checklist_blocks(c),
        CardContent::Diagram(c) => diagram_blocks(c, svg),
    }
}

// ---------- Per-type mappers ----------

fn note_blocks(c: &NoteContent) -> Vec<Block> {
    if c.markdown.trim().is_empty() {
        return vec![];
    }
    // The card title is an h2, so demote headings inside the note by one level.
    vec![Block::Markdown {
        text: c.markdown.clone(),
        heading_offset: 1,
    }]
}

fn schema_blocks(c: &SchemaContent, svg: Option<&str>) -> Vec<Block> {
    let mut blocks = Vec::new();

    if let Some(svg) = svg {
        blocks.push(Block::Svg {
            svg: svg.to_string(),
            caption: Some("Entity relationship diagram".into()),
        });
    }

    for table in &c.tables {
        blocks.push(Block::Heading {
            level: 3,
            text: table.name.clone(),
        });

        if let Some(comment) = table.comment.as_deref().filter(|s| !s.trim().is_empty()) {
            blocks.push(Block::Markdown {
                text: comment.to_string(),
                heading_offset: 0,
            });
        }

        blocks.push(Block::Table {
            columns: ["Column", "Type", "Null", "Default", "Key", "Notes"]
                .map(String::from)
                .to_vec(),
            column_widths: Some(vec![3, 2, 1, 2, 3, 4]),
            caption: None,
            rows: table
                .columns
                .iter()
                .map(|col| {
                    vec![
                        col.name.clone(),
                        col.col_type.clone(),
                        if col.nullable { "yes" } else { "no" }.to_string(),
                        col.default.clone().unwrap_or_default(),
                        key_label(col),
                        col.comment.clone().unwrap_or_default(),
                    ]
                })
                .collect(),
        });

        if !table.indexes.is_empty() {
            blocks.push(Block::Table {
                columns: vec!["Columns".into(), "Unique".into()],
                column_widths: None,
                caption: Some("Indexes".into()),
                rows: table
                    .indexes
                    .iter()
                    .map(|ix| {
                        vec![
                            ix.columns.join(", "),
                            if ix.unique { "yes" } else { "no" }.to_string(),
                        ]
                    })
                    .collect(),
            });
        }
    }

    blocks
}

fn key_label(col: &SchemaColumn) -> String {
    let mut parts: Vec<String> = Vec::new();
    if col.primary {
        parts.push("PK".into());
    }
    if col.unique {
        parts.push("UQ".into());
    }
    if let Some(r) = &col.references {
        parts.push(format!("FK \u{2192} {}.{}", r.table, r.column));
    }
    parts.join(", ")
}

fn sitemap_blocks(c: &SitemapContent) -> Vec<Block> {
    vec![Block::Tree {
        nodes: vec![sitemap_node(&c.root)],
        caption: None,
    }]
}

fn sitemap_node(n: &SitemapNode) -> TreeNode {
    TreeNode {
        label: match &n.slug {
            Some(slug) => format!("{} ({})", n.title, slug),
            None => n.title.clone(),
        },
        note: n.purpose.clone(),
        children: n.children.iter().map(sitemap_node).collect(),
    }
}

fn checklist_blocks(c: &ChecklistContent) -> Vec<Block> {
    // Preserve first-seen group order.
    let mut groups: Vec<(String, Vec<ChecklistEntry>)> = Vec::new();

    for item in &c.items {
        let key = item.group.clone().unwrap_or_default();
        let entry = ChecklistEntry {
            text: item.text.clone(),
            done: item.done,
        };
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, items)) => items.push(entry),
            None => groups.push((key, vec![entry])),
        }
    }

    vec![Block::Checklist {
        groups: groups
            .into_iter()
            .map(|(title, items)| ChecklistGroup {
                title: if title.is_empty() { None } else { Some(title) },
                items,
            })
            .collect(),
    }]
}

fn diagram_blocks(c: &DiagramContent, svg: Option<&str>) -> Vec<Block> {
    match svg {
        Some(svg) => vec![Block::Svg {
            svg: svg.to_string(),
            caption: None,
        }],
        // No rendered SVG available (e.g. card created via API/MCP and never
        // opened in the UI): ship the source so nothing is silently lost.
        None => vec![Block::Code {
            lang: Some("mermaid".into()),
            text: c.source.clone(),
            caption: Some("Diagram source (not rendered)".into()),
        }],
    }
}

// ---------- ER diagram source (rendered to SVG in the browser) ----------

/// Mermaid `erDiagram` source for a schema card.
pub fn schema_to_mermaid(c: &SchemaContent) -> String {
    let mut out = String::from("erDiagram\n");

    for t in &c.tables {
        out.push_str(&format!("  {} {{\n", t.name));
        for col in &t.columns {
            let tag = if col.primary {
                " PK"
            } else if col.references.is_some() {
                " FK"
            } else if col.unique {
                " UK"
            } else {
                ""
            };
            let ty: String = col
                .col_type
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("_");
            out.push_str(&format!("    {} {}{}\n", ty, col.name, tag));
        }
        out.push_str("  }\n");
    }

    for t in &c.tables {
        for col in &t.columns {
            if let Some(r) = &col.references {
                out.push_str(&format!(
                    "  {} ||--o{{ {} : \"{}\"\n",
                    r.table, t.name, col.name
                ));
            }
        }
    }

    out
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn board() -> Board {
        Board {
            id: "b1".into(),
            title: "Acme Rebuild".into(),
            description: Some("Planning pack".into()),
        }
    }

    fn card(id: &str, title: &str, content: CardContent) -> Card {
        Card {
            id: id.into(),
            title: title.into(),
            version: 3,
            content,
            svg: None,
        }
    }

    fn schema() -> SchemaContent {
        serde_json::from_value(json!({
            "tables": [
                { "name": "users", "columns": [
                    { "name": "id", "type": "bigint", "primary": true } ] },
                { "name": "orders",
                  "comment": "Customer orders",
                  "columns": [
                    { "name": "id", "type": "bigint", "primary": true },
                    { "name": "user_id", "type": "bigint",
                      "references": { "table": "users", "column": "id" } },
                    { "name": "status", "type": "varchar(20)", "default": "pending" } ],
                  "indexes": [ { "columns": ["user_id", "status"] } ] }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn from_parts_validates_shape() {
        let ok = CardContent::from_parts("note", json!({ "markdown": "hi" }));
        assert!(ok.is_ok());
        let bad = CardContent::from_parts("note", json!({ "text": "hi" }));
        assert!(bad.is_err());
        let unknown = CardContent::from_parts("whiteboard", json!({}));
        assert!(unknown.is_err());
    }

    #[test]
    fn follows_requested_order_and_skips_unknown_ids() {
        let cards = vec![
            card(
                "a",
                "First",
                CardContent::Note(NoteContent {
                    markdown: "# Hi".into(),
                }),
            ),
            card(
                "b",
                "Second",
                CardContent::Note(NoteContent {
                    markdown: "x".into(),
                }),
            ),
        ];
        let opts = ExportOptions {
            card_ids: vec!["b".into(), "missing".into(), "a".into()],
            page_break_between_cards: true,
            ..Default::default()
        };
        let doc = to_typeset_document(&board(), &cards, &opts, "2026-11-03");

        let headings: Vec<&str> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Heading { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(headings, vec!["Second", "First"]);
        assert_eq!(
            doc.blocks
                .iter()
                .filter(|b| matches!(b, Block::PageBreak))
                .count(),
            1
        );
        assert_eq!(doc.meta.title, "Acme Rebuild");
        assert_eq!(doc.meta.subtitle.as_deref(), Some("Planning pack"));
    }

    #[test]
    fn schema_maps_tables_keys_and_indexes() {
        let cards = vec![card("s", "Data model", CardContent::Schema(schema()))];
        let opts = ExportOptions {
            card_ids: vec!["s".into()],
            ..Default::default()
        };
        let doc = to_typeset_document(&board(), &cards, &opts, "2026-11-03");

        // heading, then per table: h3 (+comment) + table (+indexes)
        let orders_table = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Table {
                    rows,
                    caption: None,
                    ..
                } if rows.len() == 3 => Some(rows),
                _ => None,
            })
            .expect("orders table");
        assert_eq!(orders_table[1][4], "FK \u{2192} users.id");
        assert_eq!(orders_table[2][3], "pending");
        assert!(
            doc.blocks
                .iter()
                .any(|b| matches!(b, Block::Table { caption: Some(c), .. } if c == "Indexes"))
        );
        // No SVG cached, so no ER diagram block.
        assert!(!doc.blocks.iter().any(|b| matches!(b, Block::Svg { .. })));
    }

    #[test]
    fn stale_svg_is_ignored_fresh_is_used() {
        let mut c = card(
            "d",
            "Flow",
            CardContent::Diagram(DiagramContent {
                syntax: DiagramSyntax::Mermaid,
                source: "flowchart TD\n A-->B".into(),
            }),
        );
        let opts = ExportOptions {
            card_ids: vec!["d".into()],
            ..Default::default()
        };

        c.svg = Some(StoredSvg {
            svg: "<svg/>".into(),
            for_version: 2,
        }); // stale (card is v3)
        let doc = to_typeset_document(&board(), std::slice::from_ref(&c), &opts, "2026-11-03");
        assert!(matches!(doc.blocks[1], Block::Code { .. }));

        c.svg = Some(StoredSvg {
            svg: "<svg/>".into(),
            for_version: 3,
        });
        let doc = to_typeset_document(&board(), std::slice::from_ref(&c), &opts, "2026-11-03");
        assert!(matches!(doc.blocks[1], Block::Svg { .. }));
    }

    #[test]
    fn checklist_groups_keep_first_seen_order() {
        let c = ChecklistContent {
            items: vec![
                ChecklistItem {
                    text: "a".into(),
                    done: true,
                    group: Some("Kickoff".into()),
                },
                ChecklistItem {
                    text: "b".into(),
                    done: false,
                    group: None,
                },
                ChecklistItem {
                    text: "c".into(),
                    done: false,
                    group: Some("Kickoff".into()),
                },
            ],
        };
        let Block::Checklist { groups } = &checklist_blocks(&c)[0] else {
            panic!("expected checklist");
        };
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].title.as_deref(), Some("Kickoff"));
        assert_eq!(groups[0].items.len(), 2);
        assert_eq!(groups[1].title, None);
    }

    #[test]
    fn sitemap_nests_children() {
        let c = SitemapContent {
            root: SitemapNode {
                title: "Home".into(),
                slug: Some("/".into()),
                purpose: None,
                children: vec![SitemapNode {
                    title: "Services".into(),
                    slug: Some("/services".into()),
                    purpose: Some("Explain offer".into()),
                    children: vec![],
                }],
            },
        };
        let Block::Tree { nodes, .. } = &sitemap_blocks(&c)[0] else {
            panic!("expected tree");
        };
        assert_eq!(nodes[0].label, "Home (/)");
        assert_eq!(nodes[0].children[0].note.as_deref(), Some("Explain offer"));
    }

    #[test]
    fn json_shape_is_stable() {
        let cards = vec![card(
            "n",
            "Notes",
            CardContent::Note(NoteContent {
                markdown: "# A".into(),
            }),
        )];
        let opts = ExportOptions {
            card_ids: vec!["n".into()],
            ..Default::default()
        };
        let doc = to_typeset_document(&board(), &cards, &opts, "2026-11-03");
        let v = serde_json::to_value(&doc).unwrap();
        assert_eq!(v["template"], "planning-doc");
        assert_eq!(
            v["blocks"][0],
            json!({ "type": "heading", "level": 2, "text": "Notes" })
        );
        assert_eq!(
            v["blocks"][1],
            json!({ "type": "markdown", "text": "# A", "heading_offset": 1 })
        );
        assert!(v["meta"].get("client").is_none());
    }

    #[test]
    fn mermaid_er_source_links_foreign_keys() {
        let src = schema_to_mermaid(&schema());
        assert!(src.starts_with("erDiagram"));
        assert!(src.contains("users ||--o{ orders : \"user_id\""));
        assert!(src.contains("varchar(20) status"));
    }
}
