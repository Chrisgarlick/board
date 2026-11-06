//! What a schema card shows: its tables, the tables they reference from
//! outside the card (stubs), and the foreign keys between them. Used by the
//! ER renderer and for Mermaid `erDiagram` source.
//!
//! Cardinality (spec 8.4): the child end is "many" unless the foreign key
//! column is unique (one-to-one); the parent end is optional when the
//! foreign key column is nullable.

use std::fmt::Write as _;

use super::BoardTable;

/// One foreign key in the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    /// Index into [`View::tables`] of the referenced (parent) table.
    pub parent: usize,
    /// Index of the referencing (child) table.
    pub child: usize,
    /// The foreign key column's name.
    pub column: String,
    /// Child end: at most one row per parent (the FK column is unique).
    pub one_to_one: bool,
    /// Parent end: a child may have no parent (the FK column is nullable).
    pub optional: bool,
}

#[derive(Debug, Clone)]
pub struct View<'a> {
    /// The card's live tables in card order, then stubs for referenced tables
    /// outside the card.
    pub tables: Vec<&'a BoardTable>,
    /// How many of `tables` are on the card; the rest are stubs.
    pub shown: usize,
    pub relations: Vec<Relation>,
}

impl View<'_> {
    pub fn is_stub(&self, index: usize) -> bool {
        index >= self.shown
    }
}

/// The view for a card showing `table_ids`. Trashed or unknown ids are skipped.
pub fn view<'a>(table_ids: &[String], board: &'a [BoardTable]) -> View<'a> {
    let mut tables: Vec<&BoardTable> = Vec::new();
    for id in table_ids {
        if let Some(t) = board.iter().find(|t| &t.id == id && t.live)
            && !tables.iter().any(|s| s.id == t.id)
        {
            tables.push(t);
        }
    }
    let shown = tables.len();
    let mut relations = Vec::new();
    for child in 0..shown {
        let c = tables[child];
        for col in &c.definition.columns {
            let Some(r) = &col.references else { continue };
            let parent = match tables.iter().position(|t| t.id == r.table_id) {
                Some(p) => p,
                None => match board.iter().find(|t| t.id == r.table_id && t.live) {
                    Some(t) => {
                        tables.push(t);
                        tables.len() - 1
                    }
                    None => continue,
                },
            };
            relations.push(Relation {
                parent,
                child,
                column: col.name.clone(),
                one_to_one: col.unique || (col.primary && single_key(c)),
                optional: col.nullable,
            });
        }
    }
    View {
        tables,
        shown,
        relations,
    }
}

fn single_key(t: &BoardTable) -> bool {
    t.definition.columns.iter().filter(|c| c.primary).count() == 1
}

/// Mermaid `erDiagram` source for a view (spec section 14): logical types as
/// attribute types, `PK, FK` when a column is both, stubs as bare entities.
pub fn mermaid(view: &View) -> String {
    let mut out = String::from("erDiagram\n");
    for (i, t) in view.tables.iter().enumerate() {
        if view.is_stub(i) {
            let _ = writeln!(out, "    {}", t.name);
            continue;
        }
        let _ = writeln!(out, "    {} {{", t.name);
        for c in &t.definition.columns {
            let mut keys = Vec::new();
            if c.primary {
                keys.push("PK");
            }
            if c.references.is_some() {
                keys.push("FK");
            }
            if c.unique && !c.primary {
                keys.push("UK");
            }
            let ty = c.column_type.as_str();
            let _ = write!(out, "        {ty} {}", c.name);
            if !keys.is_empty() {
                let _ = write!(out, " {}", keys.join(", "));
            }
            out.push('\n');
        }
        out.push_str("    }\n");
    }
    for r in &view.relations {
        let parent_end = if r.optional { "|o" } else { "||" };
        let child_end = if r.one_to_one { "o|" } else { "o{" };
        let _ = writeln!(
            out,
            "    {} {parent_end}--{child_end} {} : \"{}\"",
            view.tables[r.parent].name, view.tables[r.child].name, r.column
        );
    }
    out
}
