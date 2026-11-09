//! Board's published read contract (spec section 9): the board model, and
//! the schema and sitemap types inside it.
//!
//! Design depends on this crate (or on the JSON Schema generated from it), so
//! it holds plain types with no I/O. Board stores table definitions and
//! sitemap nodes in exactly these shapes; reads add names next to ids
//! (`references.table`, `indexes[].columns`) so readers never have to look
//! them up.
//!
//! Evolution: additive changes are fine, breaking ones bump the contract
//! version, and readers ignore fields they don't know.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The logical column types (spec 5.4). Every stack can express them, and
/// Design maps each to a form input. Stack-specific detail goes in
/// `native_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ColumnType {
    String,
    Text,
    Integer,
    Bigint,
    Decimal,
    Boolean,
    Date,
    Time,
    Datetime,
    Uuid,
    Json,
    Enum,
    Binary,
}

impl ColumnType {
    pub const ALL: [ColumnType; 13] = [
        Self::String,
        Self::Text,
        Self::Integer,
        Self::Bigint,
        Self::Decimal,
        Self::Boolean,
        Self::Date,
        Self::Time,
        Self::Datetime,
        Self::Uuid,
        Self::Json,
        Self::Enum,
        Self::Binary,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Bigint => "bigint",
            Self::Decimal => "decimal",
            Self::Boolean => "boolean",
            Self::Date => "date",
            Self::Time => "time",
            Self::Datetime => "datetime",
            Self::Uuid => "uuid",
            Self::Json => "json",
            Self::Enum => "enum",
            Self::Binary => "binary",
        }
    }
}

/// Values the database fills in itself. Design leaves these columns out of
/// create forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Generated {
    /// Auto-increment (integer and bigint).
    Increment,
    /// A generated UUID (uuid and string).
    Uuid,
    /// The current time on insert.
    Now,
    /// The current time on insert and on every update.
    NowOnUpdate,
}

/// A foreign key: the referenced table and column, by id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Reference {
    pub table_id: String,
    pub column_id: String,
    /// The referenced table's current name. Filled in on read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    /// The referenced column's current name. Filled in on read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
}

/// One column. `id` is stable: it survives renames and type changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchemaColumn {
    pub id: String,
    /// The database identifier.
    pub name: String,
    #[serde(rename = "type")]
    pub column_type: ColumnType,
    /// `string` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<u32>,
    /// `decimal` only: total digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<u32>,
    /// `decimal` only: digits after the point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<u32>,
    /// `enum` only: the allowed values, in order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    #[serde(default)]
    pub nullable: bool,
    /// A literal default, as a string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Part of the primary key (several columns make a composite key).
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated: Option<Generated>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub references: Option<Reference>,
    /// A human label for forms and docs. Defaults to the name, humanised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// A stack-specific type when it matters (`jsonb`, `varchar(191)`).
    /// Recorded, not validated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SchemaIndex {
    pub column_ids: Vec<String>,
    #[serde(default)]
    pub unique: bool,
    /// The columns' current names, in the same order. Filled in on read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<String>>,
}

/// What Board stores for a table, besides its name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TableDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default)]
    pub columns: Vec<SchemaColumn>,
    #[serde(default)]
    pub indexes: Vec<SchemaIndex>,
}

impl TableDefinition {
    pub fn column(&self, id: &str) -> Option<&SchemaColumn> {
        self.columns.iter().find(|c| c.id == id)
    }

    /// The single primary-key column, if the key isn't composite.
    pub fn primary_key(&self) -> Option<&SchemaColumn> {
        let mut keys = self.columns.iter().filter(|c| c.primary);
        match (keys.next(), keys.next()) {
            (Some(only), None) => Some(only),
            _ => None,
        }
    }
}

/// A page in the sitemap. `id` is stable; Design keys its frames by it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SitemapNode {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// What the page is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<SitemapNode>,
}

impl SitemapNode {
    /// This node and everything under it, depth first.
    pub fn walk(&self) -> Vec<&SitemapNode> {
        let mut out = vec![self];
        for child in &self.children {
            out.extend(child.walk());
        }
        out
    }
}

// ---------- Features ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeatureStatus {
    #[default]
    Idea,
    Planned,
    InProgress,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AcceptanceItem {
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

// ---------- The board model (spec 9.2) ----------

/// The contract name in every model.
pub const CONTRACT: &str = "board-model";
/// Bumped on breaking changes only; additive changes keep it.
pub const CONTRACT_VERSION: u32 = 1;

/// Everything Design (or anything else) needs to know about a board's plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BoardModel {
    /// Always `board-model`.
    pub contract: String,
    pub contract_version: u32,
    pub board: ModelBoard,
    /// Bumps on every table change; cheap to compare.
    pub schema_version: i64,
    /// Every live table, in name order.
    pub tables: Vec<ModelTable>,
    /// The sitemap's top-level pages (one per sitemap card, in reading order).
    pub pages: Vec<SitemapNode>,
    /// Feature cards, in reading order.
    pub features: Vec<Feature>,
    /// Frames containing at least one connector, in reading order.
    pub flows: Vec<Flow>,
    /// Note cards, in reading order.
    pub notes: Vec<Note>,
    /// Things a reader should know (e.g. more than one sitemap).
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelBoard {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The stack this job uses, e.g. "Laravel + MySQL".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    /// The client this board is for: their slug in Clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelTable {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub version: i64,
    /// Labels are always filled in; references and indexes carry names.
    pub columns: Vec<ModelColumn>,
    pub indexes: Vec<SchemaIndex>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelColumn {
    #[serde(flatten)]
    pub column: SchemaColumn,
    /// A short hash of the column's definition (everything except its id
    /// and the names filled in on read). Changes exactly when the column does.
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Feature {
    pub id: String,
    pub title: String,
    pub status: FeatureStatus,
    /// Markdown.
    pub summary: String,
    #[serde(default)]
    pub stories: Vec<String>,
    #[serde(default)]
    pub acceptance: Vec<AcceptanceItem>,
    /// Live tables only.
    #[serde(default)]
    pub table_ids: Vec<String>,
    /// Pages that exist in the sitemap only.
    #[serde(default)]
    pub page_ids: Vec<String>,
}

/// A drawn flow: its steps and the arrows between them. No coordinates;
/// layout is Board's concern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Flow {
    /// The frame's id.
    pub id: String,
    pub title: String,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FlowNode {
    pub id: String,
    /// `shape` or `card`.
    pub kind: String,
    /// For shapes: rect, rounded, diamond, ellipse, parallelogram, cylinder, document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    /// For shapes: the words in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_type: Option<String>,
    /// For cards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The sitemap page this step happens on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FlowEdge {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub markdown: String,
}

/// `name` as a label: `created_at` → "Created at", `userId` → "User id".
pub fn humanise(name: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut prev_lower = false;
    for ch in name.chars() {
        if ch == '_' || ch == '-' || ch == ' ' {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            prev_lower = false;
            continue;
        }
        if ch.is_uppercase() && prev_lower && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        prev_lower = ch.is_lowercase() || ch.is_ascii_digit();
        word.extend(ch.to_lowercase());
    }
    if !word.is_empty() {
        words.push(word);
    }
    let mut label = words.join(" ");
    if let Some(first) = label.chars().next() {
        let upper: String = first.to_uppercase().collect();
        label.replace_range(..first.len_utf8(), &upper);
    }
    // Common abbreviations read better in capitals.
    label
        .split(' ')
        .map(|w| match w {
            "id" => "ID",
            "Id" => "ID",
            "url" => "URL",
            "Url" => "URL",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_humanised() {
        assert_eq!(humanise("created_at"), "Created at");
        assert_eq!(humanise("userId"), "User ID");
        assert_eq!(humanise("id"), "ID");
        assert_eq!(humanise("avatar_url"), "Avatar URL");
        assert_eq!(humanise("__x"), "X");
        assert_eq!(humanise(""), "");
    }

    #[test]
    fn columns_round_trip_with_defaults_filled() {
        let c: SchemaColumn = serde_json::from_str(
            r#"{ "id": "col_1", "name": "status", "type": "enum", "values": ["a", "b"] }"#,
        )
        .unwrap();
        assert_eq!(c.column_type, ColumnType::Enum);
        assert!(!c.nullable && !c.primary);
        let back = serde_json::to_string(&c).unwrap();
        assert_eq!(
            back,
            r#"{"id":"col_1","name":"status","type":"enum","values":["a","b"],"nullable":false,"primary":false,"unique":false}"#
        );
    }

    #[test]
    fn walks_the_sitemap_depth_first() {
        let tree: SitemapNode = serde_json::from_str(
            r#"{ "id": "pg_1", "title": "Home", "children": [
                 { "id": "pg_2", "title": "Shop", "children": [{ "id": "pg_3", "title": "Item" }] },
                 { "id": "pg_4", "title": "About" } ] }"#,
        )
        .unwrap();
        let ids: Vec<&str> = tree.walk().iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["pg_1", "pg_2", "pg_3", "pg_4"]);
    }

    #[test]
    fn model_columns_flatten_with_their_hash() {
        let c: ModelColumn = serde_json::from_str(
            r#"{ "id": "col_1", "name": "a", "type": "text", "label": "A", "hash": "abc" }"#,
        )
        .unwrap();
        assert_eq!(c.hash, "abc");
        assert_eq!(c.column.label.as_deref(), Some("A"));
        let schema = serde_json::to_string(&schemars::schema_for!(BoardModel)).unwrap();
        assert!(schema.contains("\"hash\"") && schema.contains("in_progress"));
    }

    #[test]
    fn the_json_schema_names_every_logical_type() {
        let schema = serde_json::to_string(&schemars::schema_for!(SchemaColumn)).unwrap();
        for t in ColumnType::ALL {
            assert!(schema.contains(&format!("\"{}\"", t.as_str())), "{t:?}");
        }
    }
}
