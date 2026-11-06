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
}

impl CardType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Checklist => "checklist",
        }
    }

    pub fn parse(s: &str) -> AppResult<Self> {
        match s {
            "note" => Ok(Self::Note),
            "checklist" => Ok(Self::Checklist),
            "feature" | "schema" | "sitemap" | "diagram" => Err(AppError::invalid(
                "card_type",
                format!(
                    "`{s}` cards arrive in a later phase; `note` and `checklist` are supported so far"
                ),
            )),
            _ => Err(AppError::invalid(
                "card_type",
                format!("unknown card type `{s}`, expected `note` or `checklist`"),
            )),
        }
    }

    /// Card types that refer to their board's tables or pages and so can't
    /// move to another board (spec section 6). None exist yet; schema,
    /// feature and sitemap cards will.
    pub fn can_move_between_boards(self) -> bool {
        match self {
            Self::Note | Self::Checklist => true,
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
}

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
        (Kind::Card, None) => {
            return Err(AppError::invalid("card_type", "cards need a `card_type`"));
        }
        (Kind::Shape, _) => {
            let shape: ShapeContent = parse("content", content)?;
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
fn parse<T: DeserializeOwned>(prefix: &str, value: &Value) -> AppResult<T> {
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
