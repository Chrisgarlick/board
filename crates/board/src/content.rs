//! Item content: the typed shapes stored in `items.content`, and their
//! validation (spec sections 5.1, 5.2 and 5.5).
//!
//! Validation parses the JSON into typed structs with
//! `deny_unknown_fields` (so typos are errors, not silently dropped) via
//! `serde_path_to_error` (so errors name the field). The typed value is then
//! written back out, which gives one canonical JSON form with defaults filled in.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::schema;
use board_model::SitemapNode;

/// Largest allowed content, as serialised JSON.
pub const MAX_CONTENT_BYTES: usize = 512 * 1024;
pub const MAX_TITLE_CHARS: usize = 200;

// ---------- Kinds ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Card,
    Shape,
    Sticky,
    Text,
    Frame,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Card => "card",
            Self::Shape => "shape",
            Self::Sticky => "sticky",
            Self::Text => "text",
            Self::Frame => "frame",
        }
    }

    pub fn parse(s: &str) -> AppResult<Self> {
        match s {
            "card" => Ok(Self::Card),
            "shape" => Ok(Self::Shape),
            "sticky" => Ok(Self::Sticky),
            "text" => Ok(Self::Text),
            "frame" => Ok(Self::Frame),
            _ => Err(AppError::invalid(
                "kind",
                format!("unknown kind `{s}`, expected card, shape, sticky, text or frame"),
            )),
        }
    }

    /// Cards and frames have a title; the others hold their text in content.
    fn has_title(self) -> bool {
        matches!(self, Self::Card | Self::Frame)
    }

    /// `align` is meaningful for shapes and free text only (spec 5.2).
    fn allows_align(self) -> bool {
        matches!(self, Self::Shape | Self::Text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardType {
    Note,
    Checklist,
    Schema,
    Sitemap,
}

impl CardType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Checklist => "checklist",
            Self::Schema => "schema",
            Self::Sitemap => "sitemap",
        }
    }

    pub fn parse(s: &str) -> AppResult<Self> {
        match s {
            "note" => Ok(Self::Note),
            "checklist" => Ok(Self::Checklist),
            "schema" => Ok(Self::Schema),
            "sitemap" => Ok(Self::Sitemap),
            "feature" | "diagram" => Err(AppError::invalid(
                "card_type",
                format!(
                    "`{s}` cards arrive in a later phase; note, checklist, schema and sitemap are supported so far"
                ),
            )),
            _ => Err(AppError::invalid(
                "card_type",
                format!("unknown card type `{s}`, expected note, checklist, schema or sitemap"),
            )),
        }
    }

    /// Schema and sitemap cards refer to their board's tables and pages, so
    /// they can't move to another board (spec section 6).
    pub fn can_move_between_boards(self) -> bool {
        match self {
            Self::Note | Self::Checklist => true,
            Self::Schema | Self::Sitemap => false,
        }
    }
}

// ---------- Content shapes ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteContent {
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecklistContent {
    pub items: Vec<ChecklistItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecklistItem {
    pub text: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// The flowchart shape set (spec 5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Rect,
    Rounded,
    Diamond,
    Ellipse,
    Parallelogram,
    Cylinder,
    Document,
}

impl ShapeKind {
    pub fn default_size(self) -> (f64, f64) {
        match self {
            Self::Rect | Self::Rounded | Self::Parallelogram | Self::Document => (160.0, 72.0),
            Self::Diamond => (150.0, 110.0),
            Self::Ellipse => (150.0, 90.0),
            Self::Cylinder => (130.0, 100.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShapeContent {
    pub shape: ShapeKind,
    #[serde(default)]
    pub text: String,
    /// The sitemap page this step of a journey is on (spec 5.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    /// Input only: the page by slug, resolved to `page_id`.
    #[serde(default, skip_serializing)]
    pub page: Option<String>,
}

/// A schema card: a view onto some of the board's tables (spec 5.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaCardContent {
    #[serde(default)]
    pub table_ids: Vec<String>,
    /// Input only: tables by name, resolved and added to `table_ids`.
    #[serde(default, skip_serializing)]
    pub tables: Vec<String>,
}

/// A sitemap card's content as sent: node ids are optional.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SitemapInput {
    root: PageInput,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageInput {
    #[serde(default)]
    id: Option<String>,
    title: String,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    children: Vec<PageInput>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SitemapContent {
    pub root: SitemapNode,
}

const MAX_PAGES: usize = 500;
const MAX_PAGE_DEPTH: usize = 12;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StickyContent {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextSize {
    S,
    #[default]
    M,
    L,
    Xl,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextContent {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub size: TextSize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ---------- Style ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    Grey,
    Blue,
    Green,
    Yellow,
    Orange,
    Red,
    Purple,
    Pink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Style {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
}

// ---------- Validation ----------

/// What content may refer to on its board: tables (for schema cards) and
/// sitemap pages (for shapes and other sitemap cards). Loaded by the service
/// layer for the kinds that need it.
#[derive(Debug, Clone, Default)]
pub struct Refs {
    /// Every table on the board: (id, name, live).
    pub tables: Vec<(String, String, bool)>,
    /// Pages on the board's live sitemap cards.
    pub pages: Vec<PageRef>,
    /// The item being updated, if it exists already.
    pub item_id: Option<String>,
    /// Its current content. References it already holds aren't re-checked,
    /// so a page or table removed since doesn't block an unrelated edit.
    pub current: Option<Value>,
    /// Stored content being re-read (an undelete): skip reference checks.
    pub trusted: bool,
}

#[derive(Debug, Clone)]
pub struct PageRef {
    pub id: String,
    pub slug: Option<String>,
    pub title: String,
    /// The sitemap card it's on.
    pub item_id: String,
}

impl Refs {
    fn current_str(&self, key: &str) -> Option<&str> {
        self.current.as_ref()?.get(key)?.as_str()
    }

    fn current_list(&self, key: &str) -> Vec<&str> {
        self.current
            .as_ref()
            .and_then(|c| c.get(key))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    /// Pages on other sitemap cards.
    fn other_pages(&self) -> impl Iterator<Item = &PageRef> {
        self.pages
            .iter()
            .filter(move |p| Some(&p.item_id) != self.item_id.as_ref())
    }
}

/// Slugs are paths: trimmed, with a leading `/`, no spaces.
pub fn normalise_slug(path: &str, slug: &str) -> AppResult<Option<String>> {
    let slug = slug.trim();
    if slug.is_empty() {
        return Ok(None);
    }
    if slug.chars().any(char::is_whitespace) {
        return Err(AppError::invalid(
            path,
            format!("`{slug}` has spaces; slugs are URL paths like `/about-us`"),
        ));
    }
    Ok(Some(if slug.starts_with('/') {
        slug.to_string()
    } else {
        format!("/{slug}")
    }))
}

/// A validated item, ready to store.
#[derive(Debug, Clone)]
pub struct Valid {
    pub kind: Kind,
    pub card_type: Option<CardType>,
    pub title: Option<String>,
    /// Canonical JSON for `items.content`.
    pub content: String,
    /// Canonical JSON for `items.style`, or `None` when empty.
    pub style: Option<String>,
    /// Flattened text for search.
    pub search_body: String,
    /// Size to use when the client doesn't give one.
    pub default_size: (f64, f64),
}

/// Validate everything about an item's title, content and style.
pub fn validate(
    kind: Kind,
    card_type: Option<CardType>,
    title: Option<&str>,
    content: &Value,
    style: Option<&Value>,
    refs: &Refs,
) -> AppResult<Valid> {
    let title = validate_title(kind, title)?;

    if kind != Kind::Card && card_type.is_some() {
        return Err(AppError::invalid(
            "card_type",
            "only cards have a `card_type`",
        ));
    }

    let (content, search_body, default_size) = match (kind, card_type) {
        (Kind::Card, Some(CardType::Note)) => {
            let note: NoteContent = parse("content", content)?;
            (to_json(&note)?, note.markdown.clone(), CARD_SIZE)
        }
        (Kind::Card, Some(CardType::Checklist)) => {
            let list: ChecklistContent = parse("content", content)?;
            let body = list
                .items
                .iter()
                .flat_map(|i| [i.group.as_deref().unwrap_or(""), i.text.as_str()])
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            (to_json(&list)?, body, CARD_SIZE)
        }
        (Kind::Card, Some(CardType::Schema)) => {
            let card: SchemaCardContent = parse("content", content)?;
            let card = schema_card(card, refs)?;
            let names = card
                .table_ids
                .iter()
                .filter_map(|id| refs.tables.iter().find(|t| &t.0 == id))
                .map(|t| t.1.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            (to_json(&card)?, names, (640.0, 440.0))
        }
        (Kind::Card, Some(CardType::Sitemap)) => {
            let input: SitemapInput = parse("content", content)?;
            let root = sitemap(input.root, refs)?;
            let body = root
                .walk()
                .iter()
                .flat_map(|p| {
                    [
                        Some(p.title.as_str()),
                        p.slug.as_deref(),
                        p.purpose.as_deref(),
                    ]
                })
                .flatten()
                .collect::<Vec<_>>()
                .join("\n");
            (to_json(&SitemapContent { root })?, body, (360.0, 440.0))
        }
        (Kind::Card, None) => {
            return Err(AppError::invalid("card_type", "cards need a `card_type`"));
        }
        (Kind::Shape, _) => {
            let shape: ShapeContent = parse("content", content)?;
            let shape = shape_page(shape, refs)?;
            (
                to_json(&shape)?,
                shape.text.clone(),
                shape.shape.default_size(),
            )
        }
        (Kind::Sticky, _) => {
            let sticky: StickyContent = parse("content", content)?;
            (to_json(&sticky)?, sticky.text.clone(), (180.0, 180.0))
        }
        (Kind::Text, _) => {
            let text: TextContent = parse("content", content)?;
            (to_json(&text)?, text.text.clone(), (240.0, 48.0))
        }
        (Kind::Frame, _) => {
            let frame: FrameContent = parse("content", content)?;
            (
                to_json(&frame)?,
                frame.description.clone().unwrap_or_default(),
                (800.0, 600.0),
            )
        }
    };
    if content.len() > MAX_CONTENT_BYTES {
        return Err(AppError::invalid(
            "content",
            format!("content is larger than {} KB", MAX_CONTENT_BYTES / 1024),
        ));
    }

    let style = match style {
        None | Some(Value::Null) => None,
        Some(value) => {
            let style: Style = parse("style", value)?;
            if style.align.is_some() && !kind.allows_align() {
                return Err(AppError::invalid(
                    "style.align",
                    format!("`align` applies to shapes and text, not {}s", kind.as_str()),
                ));
            }
            if style == Style::default() {
                None
            } else {
                Some(to_json(&style)?)
            }
        }
    };

    Ok(Valid {
        kind,
        card_type,
        title,
        content,
        style,
        search_body,
        default_size,
    })
}

const CARD_SIZE: (f64, f64) = (320.0, 240.0);

/// Resolve table names to ids and check new ids are tables on the board.
fn schema_card(card: SchemaCardContent, refs: &Refs) -> AppResult<SchemaCardContent> {
    let held = refs.current_list("table_ids");
    let mut ids: Vec<String> = Vec::new();
    for (i, id) in card.table_ids.iter().enumerate() {
        let known = refs.trusted
            || held.contains(&id.as_str())
            || refs.tables.iter().any(|t| &t.0 == id && t.2);
        if !known {
            return Err(AppError::invalid(
                format!("content.table_ids[{i}]"),
                "no table with this id on the board",
            ));
        }
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    for (i, name) in card.tables.iter().enumerate() {
        let Some(t) = refs
            .tables
            .iter()
            .find(|t| t.2 && t.1.eq_ignore_ascii_case(name))
        else {
            let mut names: Vec<&str> = refs
                .tables
                .iter()
                .filter(|t| t.2)
                .map(|t| t.1.as_str())
                .collect();
            names.sort_unstable();
            return Err(AppError::invalid(
                format!("content.tables[{i}]"),
                format!(
                    "no table called `{name}` on this board{}",
                    if names.is_empty() {
                        " (it has none yet; create tables with the tables API)".to_string()
                    } else {
                        format!(" (tables: {})", names.join(", "))
                    }
                ),
            ));
        };
        if !ids.contains(&t.0) {
            ids.push(t.0.clone());
        }
    }
    Ok(SchemaCardContent {
        table_ids: ids,
        tables: Vec::new(),
    })
}

/// Resolve a shape's `page` (by slug) to `page_id`, and check a new `page_id`.
fn shape_page(mut shape: ShapeContent, refs: &Refs) -> AppResult<ShapeContent> {
    if let Some(slug) = shape.page.take() {
        let wanted = normalise_slug("content.page", &slug)?;
        let found = refs
            .pages
            .iter()
            .find(|p| p.slug.is_some() && p.slug == wanted);
        let Some(page) = found else {
            let mut slugs: Vec<&str> = refs
                .pages
                .iter()
                .filter_map(|p| p.slug.as_deref())
                .collect();
            slugs.sort_unstable();
            return Err(AppError::invalid(
                "content.page",
                format!(
                    "no page with the slug `{slug}` in this board's sitemap{}",
                    if slugs.is_empty() {
                        " (add a sitemap card first)".to_string()
                    } else {
                        format!(" (pages: {})", slugs.join(", "))
                    }
                ),
            ));
        };
        shape.page_id = Some(page.id.clone());
    }
    if let Some(id) = &shape.page_id {
        let known = refs.trusted
            || refs.current_str("page_id") == Some(id.as_str())
            || refs.pages.iter().any(|p| &p.id == id);
        if !known {
            return Err(AppError::invalid(
                "content.page_id",
                "no page with this id in this board's sitemap",
            ));
        }
    }
    Ok(shape)
}

/// Give every page an id (spec 5.4): the one it was sent with; else, for a
/// page whose slug matches one already on this card, that page's id; else a
/// new one. Ids and slugs are unique across the board's sitemaps.
fn sitemap(root: PageInput, refs: &Refs) -> AppResult<SitemapNode> {
    let current: Vec<SitemapNode> = refs
        .current
        .as_ref()
        .and_then(|c| serde_json::from_value::<SitemapContent>(c.clone()).ok())
        .map(|c| c.root.walk().into_iter().cloned().collect())
        .unwrap_or_default();

    // Pass 1: shape, slugs and explicit ids, by path.
    struct Flat<'a> {
        path: String,
        input: &'a PageInput,
        slug: Option<String>,
    }
    fn flatten<'a>(
        node: &'a PageInput,
        path: String,
        depth: usize,
        out: &mut Vec<Flat<'a>>,
    ) -> AppResult<()> {
        if depth > MAX_PAGE_DEPTH {
            return Err(AppError::invalid(
                path,
                format!("a sitemap nests at most {MAX_PAGE_DEPTH} levels deep"),
            ));
        }
        let slug = match &node.slug {
            Some(s) => normalise_slug(&format!("{path}.slug"), s)?,
            None => None,
        };
        out.push(Flat {
            path: path.clone(),
            input: node,
            slug,
        });
        for (i, child) in node.children.iter().enumerate() {
            flatten(child, format!("{path}.children[{i}]"), depth + 1, out)?;
        }
        Ok(())
    }
    let mut flat = Vec::new();
    flatten(&root, "content.root".into(), 0, &mut flat)?;
    if flat.len() > MAX_PAGES {
        return Err(AppError::invalid(
            "content.root",
            format!("a sitemap holds at most {MAX_PAGES} pages"),
        ));
    }

    let mut slugs: Vec<&str> = Vec::new();
    let mut claimed: Vec<String> = Vec::new();
    for f in &flat {
        if let Some(slug) = &f.slug {
            if slugs.contains(&slug.as_str()) {
                return Err(AppError::invalid(
                    format!("{}.slug", f.path),
                    format!("`{slug}` is used twice in this sitemap"),
                ));
            }
            if !refs.trusted
                && let Some(other) = refs.other_pages().find(|p| p.slug.as_deref() == Some(slug))
            {
                return Err(AppError::invalid(
                    format!("{}.slug", f.path),
                    format!(
                        "`{slug}` is already the page \"{}\" in another sitemap on this board",
                        other.title
                    ),
                ));
            }
            slugs.push(slug);
        }
        if let Some(id) = &f.input.id {
            let path = format!("{}.id", f.path);
            if !schema::is_id(id, "pg_") {
                return Err(AppError::invalid(
                    path,
                    "page ids look like `pg_` and a ULID; leave `id` out for a new page",
                ));
            }
            if claimed.contains(id) {
                return Err(AppError::invalid(path, "two pages have the same id"));
            }
            if !refs.trusted && refs.other_pages().any(|p| &p.id == id) {
                return Err(AppError::invalid(
                    path,
                    "that id belongs to a page in another sitemap; leave `id` out for a new page",
                ));
            }
            claimed.push(id.clone());
        }
    }

    // Pass 2: ids for pages sent without one.
    let mut ids: Vec<String> = Vec::with_capacity(flat.len());
    for f in &flat {
        let id = match &f.input.id {
            Some(id) => id.clone(),
            None => {
                let matched = f.slug.as_ref().and_then(|slug| {
                    current
                        .iter()
                        .find(|c| c.slug.as_ref() == Some(slug) && !claimed.contains(&c.id))
                });
                match matched {
                    Some(c) => c.id.clone(),
                    None => schema::page_id(),
                }
            }
        };
        claimed.push(id.clone());
        ids.push(id);
    }

    // Rebuild the tree in the same depth-first order.
    fn build(node: &PageInput, flat: &[Flat], ids: &[String], next: &mut usize) -> SitemapNode {
        let k = *next;
        *next += 1;
        let tidy = |s: &Option<String>| {
            s.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        SitemapNode {
            id: ids[k].clone(),
            title: node.title.trim().to_string(),
            slug: flat[k].slug.clone(),
            purpose: tidy(&node.purpose),
            children: node
                .children
                .iter()
                .map(|c| build(c, flat, ids, next))
                .collect(),
        }
    }
    let mut next = 0;
    Ok(build(&root, &flat, &ids, &mut next))
}

fn validate_title(kind: Kind, title: Option<&str>) -> AppResult<Option<String>> {
    if !kind.has_title() {
        if title.is_some() {
            return Err(AppError::invalid(
                "title",
                format!(
                    "{}s don't have a title; put the words in `content.text`",
                    kind.as_str()
                ),
            ));
        }
        return Ok(None);
    }
    let title = title
        .ok_or_else(|| AppError::invalid("title", format!("{}s need a title", kind.as_str())))?;
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(AppError::invalid(
            "title",
            format!("titles are at most {MAX_TITLE_CHARS} characters"),
        ));
    }
    Ok(Some(title.to_string()))
}

/// Deserialise `value` into `T`, with errors that name the failing field.
pub fn parse<T: DeserializeOwned>(prefix: &str, value: &Value) -> AppResult<T> {
    serde_path_to_error::deserialize(value).map_err(|err| {
        let inner = err.path().to_string();
        let path = if inner == "." {
            prefix.to_string()
        } else {
            format!("{prefix}.{inner}")
        };
        AppError::invalid(path, err.into_inner().to_string())
    })
}

fn to_json<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|e| AppError::Internal(e.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Validation with nothing on the board to refer to.
    fn validate(
        kind: Kind,
        card_type: Option<CardType>,
        title: Option<&str>,
        content: &Value,
        style: Option<&Value>,
    ) -> AppResult<Valid> {
        super::validate(kind, card_type, title, content, style, &Refs::default())
    }

    fn board_refs() -> Refs {
        Refs {
            tables: vec![
                ("tbl_users".into(), "users".into(), true),
                ("tbl_orders".into(), "orders".into(), true),
                ("tbl_old".into(), "old".into(), false),
            ],
            pages: vec![PageRef {
                id: "pg_01JAAAAAAAAAAAAAAAAAAAAAAA".into(),
                slug: Some("/login".into()),
                title: "Log in".into(),
                item_id: "other-card".into(),
            }],
            ..Refs::default()
        }
    }

    fn card(card_type: CardType, content: Value, refs: &Refs) -> AppResult<Valid> {
        super::validate(Kind::Card, Some(card_type), Some("c"), &content, None, refs)
    }

    #[test]
    fn schema_cards_resolve_tables_by_name() {
        let refs = board_refs();
        let v = card(
            CardType::Schema,
            json!({ "table_ids": ["tbl_orders"], "tables": ["Users", "orders"] }),
            &refs,
        )
        .unwrap();
        assert_eq!(v.content, r#"{"table_ids":["tbl_orders","tbl_users"]}"#);
        assert_eq!(v.search_body, "orders\nusers");

        let err = card(CardType::Schema, json!({ "tables": ["payments"] }), &refs);
        let AppError::Invalid { path, message } = err.unwrap_err() else {
            panic!()
        };
        assert_eq!(path, "content.tables[0]");
        assert!(message.contains("tables: orders, users"), "{message}");
        assert_eq!(
            invalid_path(card(
                CardType::Schema,
                json!({ "table_ids": ["tbl_old"] }),
                &refs
            )),
            "content.table_ids[0]",
            "can't add a trashed table"
        );

        // A card that already shows a table keeps it after it's trashed.
        let held = Refs {
            current: Some(json!({ "table_ids": ["tbl_old"] })),
            ..board_refs()
        };
        assert!(card(CardType::Schema, json!({ "table_ids": ["tbl_old"] }), &held).is_ok());
    }

    #[test]
    fn shapes_link_to_pages_by_slug() {
        let refs = board_refs();
        let shape = |content: Value, refs: &Refs| {
            super::validate(Kind::Shape, None, None, &content, None, refs)
        };
        let v = shape(
            json!({ "shape": "rect", "text": "Log in", "page": "login" }),
            &refs,
        )
        .unwrap();
        assert_eq!(
            v.content,
            r#"{"shape":"rect","text":"Log in","page_id":"pg_01JAAAAAAAAAAAAAAAAAAAAAAA"}"#
        );
        let err = shape(json!({ "shape": "rect", "page": "/basket" }), &refs).unwrap_err();
        let AppError::Invalid { path, message } = err else {
            panic!()
        };
        assert_eq!(path, "content.page");
        assert!(message.contains("pages: /login"), "{message}");
        assert_eq!(
            invalid_path(shape(
                json!({ "shape": "rect", "page_id": "pg_nope" }),
                &refs
            )),
            "content.page_id"
        );
        // A link to a page that has since gone doesn't block editing the text.
        let held = Refs {
            current: Some(json!({ "shape": "rect", "page_id": "pg_gone" })),
            ..board_refs()
        };
        assert!(
            shape(
                json!({ "shape": "rect", "text": "x", "page_id": "pg_gone" }),
                &held
            )
            .is_ok()
        );
    }

    #[test]
    fn sitemap_pages_get_ids_that_survive_edits() {
        let refs = board_refs();
        let v = card(
            CardType::Sitemap,
            json!({ "root": { "title": "Home", "slug": "/", "children": [
                { "title": "Shop", "slug": "shop", "purpose": " Browse ", "children": [{ "title": "Item" }] },
                { "title": "About", "slug": "/about" },
            ] } }),
            &refs,
        )
        .unwrap();
        let content: Value = serde_json::from_str(&v.content).unwrap();
        let root: SitemapNode = serde_json::from_value(content["root"].clone()).unwrap();
        let pages = root.walk();
        assert!(pages.iter().all(|p| schema::is_id(&p.id, "pg_")));
        assert_eq!(
            pages[1].slug.as_deref(),
            Some("/shop"),
            "leading slash added"
        );
        assert_eq!(pages[1].purpose.as_deref(), Some("Browse"));
        assert!(v.search_body.contains("About"));

        // Reordered, with Shop renamed and sent without ids: pages with a
        // matching slug keep their ids; pages without a slug get new ones.
        let edited = Refs {
            current: Some(content.clone()),
            item_id: Some("this-card".into()),
            ..board_refs()
        };
        let v2 = card(
            CardType::Sitemap,
            json!({ "root": { "title": "Home", "slug": "/", "children": [
                { "title": "About us", "slug": "/about" },
                { "title": "Store", "slug": "/shop", "children": [{ "id": pages[2].id, "title": "Product" }] },
            ] } }),
            &edited,
        )
        .unwrap();
        let root2: SitemapNode = serde_json::from_value(
            serde_json::from_str::<Value>(&v2.content).unwrap()["root"].clone(),
        )
        .unwrap();
        let after = root2.walk();
        assert_eq!(after[0].id, pages[0].id);
        assert_eq!(after[1].id, pages[3].id, "about kept its id");
        assert_eq!(after[2].id, pages[1].id, "shop kept its id after a rename");
        assert_eq!(after[3].id, pages[2].id, "explicit id kept");
    }

    #[test]
    fn sitemap_slugs_and_ids_are_unique_on_the_board() {
        let refs = board_refs();
        let sitemap = |root: Value| card(CardType::Sitemap, json!({ "root": root }), &refs);
        assert_eq!(
            invalid_path(sitemap(json!({ "title": "Home", "children": [
                { "title": "A", "slug": "/a" }, { "title": "B", "slug": "a" } ] }))),
            "content.root.children[1].slug"
        );
        let err = sitemap(json!({ "title": "Sign in", "slug": "/login" })).unwrap_err();
        let AppError::Invalid { path, message } = err else {
            panic!()
        };
        assert_eq!(path, "content.root.slug");
        assert!(message.contains("Log in"), "{message}");
        assert_eq!(
            invalid_path(sitemap(
                json!({ "id": "pg_01JAAAAAAAAAAAAAAAAAAAAAAA", "title": "x" })
            )),
            "content.root.id"
        );
        assert_eq!(
            invalid_path(sitemap(json!({ "id": "pg_1", "title": "x" }))),
            "content.root.id"
        );
        assert_eq!(
            invalid_path(sitemap(json!({ "title": "x", "slug": "/a b" }))),
            "content.root.slug"
        );
        assert_eq!(
            invalid_path(sitemap(json!({ "slug": "/" }))),
            "content.root"
        );
    }

    fn note(content: Value) -> AppResult<Valid> {
        validate(
            Kind::Card,
            Some(CardType::Note),
            Some("Notes"),
            &content,
            None,
        )
    }

    fn invalid_path<T: std::fmt::Debug>(r: AppResult<T>) -> String {
        match r {
            Err(AppError::Invalid { path, .. }) => path,
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    #[test]
    fn accepts_and_canonicalises() {
        let v = validate(
            Kind::Card,
            Some(CardType::Checklist),
            Some("Launch"),
            &json!({ "items": [{ "text": "DNS" }, { "text": "SSL", "done": true, "group": "Infra" }] }),
            Some(&json!({ "color": "blue" })),
        )
        .unwrap();
        assert_eq!(
            v.content,
            r#"{"items":[{"text":"DNS","done":false},{"text":"SSL","done":true,"group":"Infra"}]}"#
        );
        assert_eq!(v.style.as_deref(), Some(r#"{"color":"blue"}"#));
        assert_eq!(v.search_body, "DNS\nInfra\nSSL");
    }

    #[test]
    fn rejects_typos_with_a_path() {
        let err = validate(
            Kind::Card,
            Some(CardType::Checklist),
            Some("Launch"),
            &json!({ "items": [{ "text": "a" }, { "text": "b", "dnoe": true }] }),
            None,
        )
        .unwrap_err();
        let AppError::Invalid { path, message } = err else {
            panic!("expected invalid")
        };
        assert_eq!(path, "content.items[1].dnoe");
        assert!(message.contains("unknown field `dnoe`"), "{message}");
    }

    #[test]
    fn missing_field_is_reported() {
        let AppError::Invalid { path, message } = note(json!({})).unwrap_err() else {
            panic!("expected invalid")
        };
        assert_eq!(path, "content");
        assert!(message.contains("missing field `markdown`"), "{message}");
    }

    #[test]
    fn style_rules() {
        let card = |style: Value| {
            validate(
                Kind::Card,
                Some(CardType::Note),
                Some("n"),
                &json!({ "markdown": "" }),
                Some(&style),
            )
        };
        assert_eq!(
            invalid_path(card(json!({ "color": "teal" }))),
            "style.color"
        );
        assert_eq!(
            invalid_path(card(json!({ "align": "left" }))),
            "style.align"
        );
        assert_eq!(card(json!({})).unwrap().style, None);

        // Shapes and text may be aligned.
        let shape = validate(
            Kind::Shape,
            None,
            None,
            &json!({ "shape": "rect", "text": "Pay" }),
            Some(&json!({ "align": "left", "color": "green" })),
        )
        .unwrap();
        assert_eq!(
            shape.style.as_deref(),
            Some(r#"{"color":"green","align":"left"}"#)
        );
        let sticky = validate(
            Kind::Sticky,
            None,
            None,
            &json!({ "text": "idea" }),
            Some(&json!({ "align": "left" })),
        );
        assert_eq!(invalid_path(sticky), "style.align");
    }

    #[test]
    fn titles_belong_to_cards_and_frames_only() {
        assert_eq!(
            invalid_path(validate(
                Kind::Card,
                Some(CardType::Note),
                None,
                &json!({ "markdown": "" }),
                None
            )),
            "title"
        );
        assert_eq!(
            invalid_path(validate(Kind::Frame, None, None, &json!({}), None)),
            "title"
        );
        assert_eq!(
            invalid_path(validate(
                Kind::Shape,
                None,
                Some("Pay"),
                &json!({ "shape": "rect" }),
                None
            )),
            "title"
        );
        assert_eq!(
            invalid_path(validate(Kind::Card, None, Some("t"), &json!({}), None)),
            "card_type"
        );
        assert_eq!(
            invalid_path(validate(
                Kind::Sticky,
                Some(CardType::Note),
                None,
                &json!({}),
                None
            )),
            "card_type"
        );
    }

    #[test]
    fn shapes_have_their_own_default_sizes() {
        let size = |shape: &str| {
            validate(Kind::Shape, None, None, &json!({ "shape": shape }), None)
                .unwrap()
                .default_size
        };
        assert_eq!(size("rect"), (160.0, 72.0));
        assert_eq!(size("diamond"), (150.0, 110.0));
        assert_eq!(size("cylinder"), (130.0, 100.0));
        assert_eq!(
            invalid_path(validate(
                Kind::Shape,
                None,
                None,
                &json!({ "shape": "star" }),
                None
            )),
            "content.shape"
        );
    }

    #[test]
    fn text_and_sticky_defaults_are_filled_in() {
        let text = validate(Kind::Text, None, None, &json!({ "text": "Phase 1" }), None).unwrap();
        assert_eq!(text.content, r#"{"text":"Phase 1","size":"m"}"#);
        let sticky = validate(Kind::Sticky, None, None, &json!({}), None).unwrap();
        assert_eq!(sticky.content, r#"{"text":""}"#);
        let frame = validate(Kind::Frame, None, Some("Checkout"), &json!({}), None).unwrap();
        assert_eq!(frame.content, "{}");
        assert_eq!(frame.default_size, (800.0, 600.0));
    }

    #[test]
    fn content_size_is_capped() {
        let big = "x".repeat(MAX_CONTENT_BYTES + 1);
        assert!(note(json!({ "markdown": big })).is_err());
    }
}
