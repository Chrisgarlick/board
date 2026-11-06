//! Item content: the typed shapes stored in `items.content`, and their
//! validation (spec sections 5.1, 5.2 and 5.5).
//!
//! Phase 1 supports note and checklist cards. Other kinds and card types are
//! added in later phases by extending the enums here.
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
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Card => "card",
        }
    }

    pub fn parse(s: &str) -> AppResult<Self> {
        match s {
            "card" => Ok(Self::Card),
            "shape" | "sticky" | "text" | "frame" => Err(AppError::invalid(
                "kind",
                format!("`{s}` items arrive in phase 2; only `card` is supported so far"),
            )),
            _ => Err(AppError::invalid(
                "kind",
                format!("unknown kind `{s}`, expected `card`"),
            )),
        }
    }

    /// Default size when a client doesn't give one.
    pub fn default_size(self) -> (f64, f64) {
        match self {
            Self::Card => (320.0, 240.0),
        }
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

    let (content, search_body) = match (kind, card_type) {
        (Kind::Card, Some(CardType::Note)) => {
            let note: NoteContent = parse("content", content)?;
            (to_json(&note)?, note.markdown.clone())
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
            (to_json(&list)?, body)
        }
        (Kind::Card, None) => {
            return Err(AppError::invalid("card_type", "cards need a `card_type`"));
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
            if kind == Kind::Card && style.align.is_some() {
                return Err(AppError::invalid(
                    "style.align",
                    "`align` applies to shapes and text, not cards",
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
    })
}

fn validate_title(kind: Kind, title: Option<&str>) -> AppResult<Option<String>> {
    match kind {
        Kind::Card => {
            let title = title.ok_or_else(|| AppError::invalid("title", "cards need a title"))?;
            if title.chars().count() > MAX_TITLE_CHARS {
                return Err(AppError::invalid(
                    "title",
                    format!("titles are at most {MAX_TITLE_CHARS} characters"),
                ));
            }
            Ok(Some(title.to_string()))
        }
    }
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
        let bad_color = validate(
            Kind::Card,
            Some(CardType::Note),
            Some("n"),
            &json!({ "markdown": "" }),
            Some(&json!({ "color": "teal" })),
        );
        assert!(
            matches!(bad_color, Err(AppError::Invalid { ref path, .. }) if path == "style.color")
        );

        let align_on_card = validate(
            Kind::Card,
            Some(CardType::Note),
            Some("n"),
            &json!({ "markdown": "" }),
            Some(&json!({ "align": "left" })),
        );
        assert!(
            matches!(align_on_card, Err(AppError::Invalid { ref path, .. }) if path == "style.align")
        );

        // An empty style is stored as no style.
        let empty = validate(
            Kind::Card,
            Some(CardType::Note),
            Some("n"),
            &json!({ "markdown": "" }),
            Some(&json!({})),
        )
        .unwrap();
        assert_eq!(empty.style, None);
    }

    #[test]
    fn cards_need_a_title_and_a_type() {
        assert!(matches!(
            validate(Kind::Card, Some(CardType::Note), None, &json!({ "markdown": "" }), None),
            Err(AppError::Invalid { ref path, .. }) if path == "title"
        ));
        assert!(matches!(
            validate(Kind::Card, None, Some("t"), &json!({}), None),
            Err(AppError::Invalid { ref path, .. }) if path == "card_type"
        ));
    }

    #[test]
    fn later_kinds_say_when_they_arrive() {
        let AppError::Invalid { message, .. } = Kind::parse("shape").unwrap_err() else {
            panic!()
        };
        assert!(message.contains("phase 2"));
    }

    #[test]
    fn content_size_is_capped() {
        let big = "x".repeat(MAX_CONTENT_BYTES + 1);
        assert!(note(json!({ "markdown": big })).is_err());
    }
}
