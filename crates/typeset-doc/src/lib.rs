//! Typeset document contract (PROPOSED).
//!
//! Temporary home: this crate moves to the Typeset repo in phase 7 and Board
//! will depend on it by git tag (spec section 2).
//!
//! Intended to live in a small shared crate (`typeset-doc`) that both Typeset
//! and the board app depend on, so the payload can't drift between them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypesetDocument {
    pub template: String,
    pub meta: Meta,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// ISO date, e.g. "2026-11-03".
    pub date: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Heading {
        level: u8,
        text: String,
    },
    Markdown {
        text: String,
        /// Demote headings inside the markdown by this many levels.
        #[serde(default, skip_serializing_if = "is_zero")]
        heading_offset: u8,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        /// Relative column weights.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        column_widths: Option<Vec<u8>>,
    },
    Tree {
        nodes: Vec<TreeNode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
    },
    Checklist {
        groups: Vec<ChecklistGroup>,
    },
    Svg {
        svg: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
    },
    Code {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lang: Option<String>,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
    },
    Callout {
        variant: CalloutVariant,
        text: String,
    },
    PageBreak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalloutVariant {
    Info,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeNode {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChecklistGroup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub items: Vec<ChecklistEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChecklistEntry {
    pub text: String,
    pub done: bool,
}

fn is_zero(n: &u8) -> bool {
    *n == 0
}
