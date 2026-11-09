//! Boards as Typeset documents (spec section 8): Typeset's JSON input
//! (`"schema": 2`, pages of blocks), so Typeset can make a planning PDF in a
//! client's brand. The same document can be joined with Design's (its A4
//! frames are `page` blocks) into one client proposal.
//!
//! What goes in (spec 6, "Export selection"): frames and cards. By default
//! every frame and every card outside a frame, in reading order; a frame
//! brings its drawing, then the cards inside it. Each one starts a section.
//!
//! This module is pure: images come from a function the caller passes in
//! (data URIs for a PDF, signed links for a document Claude will pass on).

use serde::Deserialize;
use serde_json::{Value, json};

use crate::content::{FeatureContent, SitemapContent};
use crate::error::{AppError, AppResult};
use crate::model::reading_order;
use crate::schema::{BoardTable, type_label};
use crate::service::boards::Board;
use crate::service::items::Item;
use board_model::{FeatureStatus, SitemapNode, TableDefinition};

/// Typeset's limits (its API.md): images per document, bytes per image,
/// bytes per request.
pub const MAX_IMAGES: usize = 24;
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_REQUEST_BYTES: usize = 25 * 1024 * 1024;

/// What to put in the document and on its cover.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    /// Frames and cards to include, in this order. Empty: everything, in reading order.
    #[serde(default)]
    pub item_ids: Vec<String>,
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
    /// The small label above the title (default "Planning document").
    #[serde(default)]
    pub eyebrow: Option<String>,
    /// The client whose brand styles it; the board's client by default.
    #[serde(default)]
    pub client: Option<String>,
    /// For a combined document (e.g. after Design's screens): start with a
    /// `part` divider of this title instead of a contents page.
    #[serde(default)]
    pub part: Option<String>,
}

/// One section of the document.
#[derive(Debug)]
pub enum Unit<'a> {
    Frame {
        frame: &'a Item,
        cards: Vec<&'a Item>,
    },
    Card(&'a Item),
}

/// Frames and cards to export, in order (spec 6, "Export selection").
pub fn select<'a>(items: &'a [Item], ids: &[String]) -> AppResult<Vec<Unit<'a>>> {
    let unit = |i: &'a Item| -> Unit<'a> {
        if i.kind == "frame" {
            let cards = reading_order(
                items
                    .iter()
                    .filter(|m| m.kind == "card" && m.frame_id.as_deref() == Some(i.id.as_str())),
            );
            Unit::Frame { frame: i, cards }
        } else {
            Unit::Card(i)
        }
    };
    if ids.is_empty() {
        return Ok(reading_order(
            items
                .iter()
                .filter(|i| i.kind == "frame" || (i.kind == "card" && i.frame_id.is_none())),
        )
        .into_iter()
        .map(unit)
        .collect());
    }
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for (k, id) in ids.iter().enumerate() {
        if seen.contains(&id.as_str()) {
            continue; // duplicates are dropped after their first appearance
        }
        seen.push(id);
        let item = items.iter().find(|i| &i.id == id).ok_or_else(|| {
            AppError::invalid(format!("item_ids[{k}]"), "no such item on this board")
        })?;
        if item.kind != "frame" && item.kind != "card" {
            return Err(AppError::invalid(
                format!("item_ids[{k}]"),
                "only frames and cards are exported; put drawings in a frame",
            ));
        }
        out.push(unit(item));
    }
    Ok(out)
}

/// What a frame or schema card looks like as an image: a URL Typeset can
/// use (a data URI or a signed link), or `None` if it couldn't be drawn.
pub struct Image {
    pub url: String,
}

/// The finished document, with anything the reader should know.
#[derive(Debug)]
pub struct Document {
    pub json: Value,
    pub warnings: Vec<String>,
    pub images: usize,
}

/// Build the document. `image` draws a frame or a schema card.
pub fn document(
    board: &Board,
    items: &[Item],
    tables: &[BoardTable],
    opts: &Options,
    mut image: impl FnMut(&Item) -> Option<Image>,
) -> AppResult<Document> {
    let units = select(items, &opts.item_ids)?;
    let mut b = Builder {
        tables,
        items,
        images: 0,
        bytes: 0,
        warnings: Vec::new(),
        image: &mut image,
    };
    let mut pages: Vec<Value> = Vec::new();
    if let Some(part) = &opts.part {
        pages.push(json!({ "blocks": [{ "type": "part", "title": esc(part) }] }));
    } else if units.len() >= 4 {
        pages.push(json!({ "blocks": [{ "type": "contents" }] }));
    }
    if units.is_empty() {
        b.warnings.push(
            "Nothing to export: this board has no frames or cards. Put drawings in a frame.".into(),
        );
    }
    for u in &units {
        let mut blocks = Vec::new();
        match u {
            Unit::Frame { frame, cards } => {
                let mut opener = json!({ "type": "opener", "title": esc(title_of(frame)) });
                if let Some(d) = frame.content.get("description").and_then(Value::as_str) {
                    opener["intro"] = json!(d);
                }
                blocks.push(opener);
                // The drawing, unless the frame only holds cards (which follow anyway).
                let drawn = items
                    .iter()
                    .any(|m| m.frame_id.as_deref() == Some(frame.id.as_str()) && m.kind != "card");
                if drawn {
                    b.picture(&mut blocks, frame, &flow_alt(frame, items));
                }
                for card in cards {
                    b.card(&mut blocks, card, 3);
                }
            }
            Unit::Card(card) => {
                blocks.push(json!({ "type": "opener", "title": esc(title_of(card)) }));
                b.card(&mut blocks, card, 3);
            }
        }
        pages.push(json!({ "blocks": blocks }));
    }

    let mut front = json!({
        "title": opts.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| board.title.clone()),
        "eyebrow": opts.eyebrow.clone().unwrap_or_else(|| "Planning document".into()),
    });
    let subtitle = opts.subtitle.clone().or_else(|| {
        board
            .description
            .as_deref()
            .and_then(|d| d.lines().next())
            .map(str::to_string)
    });
    for (key, value) in [
        ("subtitle", subtitle),
        ("recipient", opts.recipient.clone()),
        ("author", opts.author.clone()),
        ("date", opts.date.clone()),
    ] {
        if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
            front[key] = json!(v);
        }
    }
    Ok(Document {
        json: json!({ "schema": 2, "frontmatter": front, "pages": pages }),
        warnings: b.warnings,
        images: b.images,
    })
}

struct Builder<'a, 'f> {
    tables: &'a [BoardTable],
    items: &'a [Item],
    images: usize,
    bytes: usize,
    warnings: Vec<String>,
    image: &'f mut dyn FnMut(&Item) -> Option<Image>,
}

impl Builder<'_, '_> {
    /// An image block for a frame or schema card, within Typeset's limits.
    fn picture(&mut self, blocks: &mut Vec<Value>, item: &Item, alt: &str) {
        let name = title_of(item);
        if self.images >= MAX_IMAGES {
            self.warnings.push(format!(
                "\"{name}\" was left out: Typeset takes {MAX_IMAGES} images per document"
            ));
            return;
        }
        let Some(img) = (self.image)(item) else {
            self.warnings
                .push(format!("\"{name}\" couldn't be drawn, so it's left out"));
            return;
        };
        if img.url.len() > MAX_IMAGE_BYTES * 4 / 3 + 64 {
            self.warnings.push(format!(
                "\"{name}\" is too big an image for Typeset (5 MB each), so it's left out"
            ));
            return;
        }
        if self.bytes + img.url.len() > MAX_REQUEST_BYTES {
            self.warnings.push(format!(
                "\"{name}\" was left out: the images add up to more than Typeset's 25 MB"
            ));
            return;
        }
        self.images += 1;
        self.bytes += img.url.len();
        blocks.push(json!({ "type": "image", "url": img.url, "alt": alt, "width": "full" }));
    }

    /// A card's content. `level` is the heading level for parts within it.
    fn card(&mut self, blocks: &mut Vec<Value>, card: &Item, level: u8) {
        // Cards inside a frame get a heading of their own.
        if card.frame_id.is_some() {
            blocks.push(json!({ "type": "heading", "level": level, "text": esc(title_of(card)) }));
        }
        let sub = if card.frame_id.is_some() {
            level + 1
        } else {
            level
        };
        match card.card_type.as_deref() {
            Some("note") => {
                let md = card
                    .content
                    .get("markdown")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !md.trim().is_empty() {
                    blocks.push(json!({ "type": "markdown", "content": demote(md, sub) }));
                }
            }
            Some("checklist") => self.checklist(blocks, card, sub),
            Some("feature") => self.feature(blocks, card, sub),
            Some("schema") => self.schema(blocks, card, sub),
            Some("sitemap") => {
                if let Ok(c) = serde_json::from_value::<SitemapContent>(card.content.clone()) {
                    blocks.push(json!({ "type": "tree", "items": [sitemap_tree(&c.root)] }));
                }
            }
            _ => {}
        }
    }

    fn checklist(&mut self, blocks: &mut Vec<Value>, card: &Item, level: u8) {
        let items = card
            .content
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // Groups in first-seen order, ungrouped first (as on the canvas).
        let mut groups: Vec<(String, Vec<&Value>)> = vec![(String::new(), Vec::new())];
        for i in &items {
            let g = i
                .get("group")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            match groups.iter_mut().find(|(name, _)| *name == g) {
                Some((_, list)) => list.push(i),
                None => groups.push((g, vec![i])),
            }
        }
        for (name, list) in groups.iter().filter(|(_, l)| !l.is_empty()) {
            if !name.is_empty() {
                blocks.push(json!({ "type": "heading", "level": level, "text": esc(name) }));
            }
            blocks.push(json!({
                "type": "checklist",
                "items": list.iter().map(|i| json!({
                    "text": esc(i.get("text").and_then(Value::as_str).unwrap_or("")),
                    "done": i.get("done").and_then(Value::as_bool).unwrap_or(false),
                })).collect::<Vec<_>>(),
            }));
        }
    }

    fn feature(&mut self, blocks: &mut Vec<Value>, card: &Item, level: u8) {
        let Ok(f) = serde_json::from_value::<FeatureContent>(card.content.clone()) else {
            return;
        };
        let status = match f.status {
            FeatureStatus::Idea => "Idea",
            FeatureStatus::Planned => "Planned",
            FeatureStatus::InProgress => "In progress",
            FeatureStatus::Done => "Done",
        };
        let mut facts = vec![json!({ "label": "Status", "value": status })];
        let tables: Vec<String> = f
            .table_ids
            .iter()
            .filter_map(|id| self.tables.iter().find(|t| &t.id == id && t.live))
            .map(|t| esc(&t.name))
            .collect();
        if !tables.is_empty() {
            facts.push(json!({ "label": "Tables", "value": tables.join(", ") }));
        }
        let pages = page_names(self.items, &f.page_ids);
        if !pages.is_empty() {
            facts.push(json!({ "label": "Pages", "value": pages.join(", ") }));
        }
        blocks.push(json!({ "type": "facts", "items": facts }));
        if !f.summary.trim().is_empty() {
            blocks.push(json!({ "type": "markdown", "content": demote(&f.summary, level) }));
        }
        let stories: Vec<&String> = f.stories.iter().filter(|s| !s.trim().is_empty()).collect();
        if !stories.is_empty() {
            blocks.push(json!({ "type": "heading", "level": level, "text": "Stories" }));
            blocks.push(json!({ "type": "bullet_list", "items": stories.iter().map(|s| esc(s)).collect::<Vec<_>>() }));
        }
        let checks: Vec<_> = f
            .acceptance
            .iter()
            .filter(|a| !a.text.trim().is_empty())
            .collect();
        if !checks.is_empty() {
            blocks.push(json!({ "type": "heading", "level": level, "text": "Done when" }));
            blocks.push(json!({
                "type": "checklist",
                "items": checks.iter().map(|a| json!({ "text": esc(&a.text), "done": a.done })).collect::<Vec<_>>(),
            }));
        }
    }

    fn schema(&mut self, blocks: &mut Vec<Value>, card: &Item, level: u8) {
        let ids = crate::render::card_tables(card);
        let shown: Vec<&BoardTable> = ids
            .iter()
            .filter_map(|id| self.tables.iter().find(|t| &t.id == id && t.live))
            .collect();
        if shown.is_empty() {
            blocks.push(json!({ "type": "paragraph", "text": "No tables on this card yet." }));
            return;
        }
        let names: Vec<&str> = shown.iter().map(|t| t.name.as_str()).collect();
        let alt = format!("Entity-relationship diagram of {}", names.join(", "));
        self.picture(blocks, card, &alt);
        for t in shown {
            blocks.push(json!({ "type": "heading", "level": level, "text": esc(&t.name) }));
            if let Some(c) = &t.definition.comment {
                blocks.push(json!({ "type": "paragraph", "text": esc(c) }));
            }
            blocks.push(table_block(t, self.tables));
        }
    }
}

/// A table's columns as a Typeset table: name, type, required, key, notes.
fn table_block(t: &BoardTable, all: &[BoardTable]) -> Value {
    let def: &TableDefinition = &t.definition;
    let rows: Vec<Vec<String>> = def
        .columns
        .iter()
        .map(|c| {
            let key = match (c.primary, c.references.is_some(), c.unique) {
                (true, true, _) => "PK, FK",
                (true, false, _) => "PK",
                (false, true, true) => "FK, unique",
                (false, true, false) => "FK",
                (false, false, true) => "Unique",
                _ => "",
            };
            let mut notes: Vec<String> = Vec::new();
            if let Some(r) = &c.references {
                let target = all.iter().find(|x| x.id == r.table_id);
                let column = target.and_then(|x| x.definition.column(&r.column_id));
                if let (Some(x), Some(col)) = (target, column) {
                    notes.push(format!("References `{}.{}`", x.name, col.name));
                }
            }
            if let Some(values) = &c.values {
                notes.push(format!(
                    "One of {}",
                    values
                        .iter()
                        .map(|v| format!("`{v}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if let Some(d) = &c.default {
                notes.push(format!("Default `{d}`"));
            }
            if let Some(g) = c.generated {
                notes.push(
                    match g {
                        board_model::Generated::Increment => "Auto-increments",
                        board_model::Generated::Uuid => "Generated UUID",
                        board_model::Generated::Now => "Set when created",
                        board_model::Generated::NowOnUpdate => "Set on every change",
                    }
                    .to_string(),
                );
            }
            if let Some(comment) = &c.comment {
                notes.push(esc(comment));
            }
            vec![
                format!("`{}`", c.name),
                format!("`{}`", type_label(c)),
                if c.nullable {
                    "No".into()
                } else {
                    "Yes".into()
                },
                key.to_string(),
                notes.join(". "),
            ]
        })
        .collect();
    json!({
        "type": "table",
        "headers": ["Column", "Type", "Required", "Key", "Notes"],
        "rows": rows,
    })
}

/// The pages a feature links to, as their slugs (or titles), for the facts list.
fn page_names(items: &[Item], ids: &[String]) -> Vec<String> {
    let pages: Vec<SitemapNode> = items
        .iter()
        .filter(|i| i.card_type.as_deref() == Some("sitemap"))
        .filter_map(|i| serde_json::from_value::<SitemapContent>(i.content.clone()).ok())
        .flat_map(|c| c.root.walk().into_iter().cloned().collect::<Vec<_>>())
        .collect();
    ids.iter()
        .filter_map(|id| pages.iter().find(|p| &p.id == id))
        .map(|p| match &p.slug {
            Some(s) => format!("{} ({})", esc(&p.title), esc(s)),
            None => esc(&p.title),
        })
        .collect()
}

/// A sitemap page as a tree item: **title** `slug`: purpose.
fn sitemap_tree(n: &SitemapNode) -> Value {
    let mut text = format!(
        "**{}**",
        esc(if n.title.is_empty() {
            "Untitled"
        } else {
            &n.title
        })
    );
    if let Some(s) = &n.slug {
        text.push_str(&format!(" `{s}`"));
    }
    if let Some(p) = &n.purpose {
        text.push_str(&format!(": {}", esc(p)));
    }
    let mut item = json!({ "text": text });
    if !n.children.is_empty() {
        item["children"] = json!(n.children.iter().map(sitemap_tree).collect::<Vec<_>>());
    }
    item
}

/// Alt text for a drawn flow: its steps in reading order.
fn flow_alt(frame: &Item, items: &[Item]) -> String {
    let steps: Vec<String> = reading_order(
        items
            .iter()
            .filter(|i| i.frame_id.as_deref() == Some(frame.id.as_str()) && i.kind == "shape"),
    )
    .iter()
    .filter_map(|i| i.content.get("text").and_then(Value::as_str))
    .filter(|t| !t.trim().is_empty())
    .map(|t| t.replace('\n', " "))
    .collect();
    let name = title_of(frame);
    if steps.is_empty() {
        return format!("Drawing: {name}");
    }
    let mut alt = format!("Flowchart, {name}: {}", steps.join(", then "));
    if alt.chars().count() > 600 {
        alt = alt.chars().take(597).collect::<String>() + "…";
    }
    alt
}

fn title_of(i: &Item) -> &str {
    i.title
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or("Untitled")
}

/// Plain text made safe for Typeset's inline markdown.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(
            ch,
            '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '>' | '#' | '|' | '~'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Markdown with its headings moved down so the first level is `level`
/// (a card's `# Title` sits under the section opener, not beside the cover).
pub fn demote(md: &str, level: u8) -> String {
    let shift = level.saturating_sub(1) as usize;
    let mut fenced = false;
    md.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fenced = !fenced;
            }
            if fenced {
                return line.to_string();
            }
            let hashes = trimmed.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
                let to = (hashes + shift).min(6);
                format!("{}{}", "#".repeat(to), &trimmed[hashes..])
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests;
