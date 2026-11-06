//! The board schema's rules (spec sections 5.4 and 5.5), as pure functions so
//! every rule is testable without a database.
//!
//! [`prepare`] turns a table as a client sent it (references by name, columns
//! without ids) into what Board stores: every column has a stable id, every
//! reference is `{ table_id, column_id }`, and every semantic rule has been
//! checked. Problems that shouldn't block a write come back as warnings.

pub mod er;

use std::collections::{HashMap, HashSet};

use board_model::{
    ColumnType, Generated, Reference, SchemaColumn, SchemaIndex, SitemapNode, TableDefinition,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::content::{self, MAX_CONTENT_BYTES};
use crate::error::{AppError, AppResult};
use crate::types::new_id;

/// MySQL's identifier limit, the tightest of the common databases.
pub const MAX_NAME_CHARS: usize = 64;
const MAX_COLUMNS: usize = 500;
const MAX_LENGTH: u32 = 65_535;
const MAX_PRECISION: u32 = 65;
const MAX_SCALE: u32 = 30;

/// A table already on the board, as the rules need it.
#[derive(Debug, Clone)]
pub struct BoardTable {
    pub id: String,
    pub name: String,
    pub definition: TableDefinition,
    pub live: bool,
}

/// Something worth knowing that doesn't block the write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Warning {
    pub path: String,
    pub message: String,
}

fn warn(path: impl Into<String>, message: impl Into<String>) -> Warning {
    Warning {
        path: path.into(),
        message: message.into(),
    }
}

// ---------- Input ----------

/// A table definition as clients send it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionInput {
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    columns: Vec<ColumnInput>,
    #[serde(default)]
    indexes: Vec<IndexInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColumnInput {
    /// Leave out for new columns; send it back for existing ones (or rely on
    /// the name matching).
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(rename = "type")]
    column_type: ColumnType,
    #[serde(default)]
    length: Option<u32>,
    #[serde(default)]
    precision: Option<u32>,
    #[serde(default)]
    scale: Option<u32>,
    #[serde(default)]
    values: Option<Vec<String>>,
    #[serde(default)]
    nullable: bool,
    #[serde(default, deserialize_with = "literal")]
    default: Option<String>,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    unique: bool,
    #[serde(default)]
    generated: Option<Generated>,
    #[serde(default)]
    references: Option<ReferenceInput>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    native_type: Option<String>,
}

/// A reference by id (`table_id`, `column_id`) or by name (`table`,
/// `column`). Ids win when both are given, so a definition read from the API
/// can be sent straight back. Without a column, the table's primary key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReferenceInput {
    #[serde(default)]
    table_id: Option<String>,
    #[serde(default)]
    column_id: Option<String>,
    #[serde(default)]
    table: Option<String>,
    #[serde(default)]
    column: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexInput {
    #[serde(default)]
    column_ids: Option<Vec<String>>,
    /// Column names, as an alternative to `column_ids`.
    #[serde(default)]
    columns: Option<Vec<String>>,
    #[serde(default)]
    unique: bool,
}

/// Defaults are strings, but `0` and `false` are accepted and written as text.
fn literal<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s),
        Some(Value::Bool(b)) => Some(b.to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(_) => {
            return Err(serde::de::Error::custom(
                "a default is a literal: a string, number or true/false",
            ));
        }
    })
}

// ---------- Ids and names ----------

/// `tbl_`, `col_` or `pg_` followed by a ULID.
pub fn is_id(s: &str, prefix: &str) -> bool {
    s.strip_prefix(prefix).is_some_and(|rest| {
        rest.len() == 26
            && rest
                .bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
    })
}

pub fn table_id() -> String {
    format!("tbl_{}", new_id())
}

fn column_id() -> String {
    format!("col_{}", new_id())
}

pub fn page_id() -> String {
    format!("pg_{}", new_id())
}

/// Database identifiers: `^[A-Za-z_][A-Za-z0-9_]*$`, at most 64 characters.
pub fn check_identifier(path: &str, name: &str, what: &str) -> AppResult<()> {
    let mut chars = name.chars();
    let starts_well = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !starts_well || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(AppError::invalid(
            path,
            format!(
                "`{name}` isn't a valid {what} name: use letters, digits and underscores, not starting with a digit"
            ),
        ));
    }
    if name.len() > MAX_NAME_CHARS {
        return Err(AppError::invalid(
            path,
            format!("{what} names are at most {MAX_NAME_CHARS} characters"),
        ));
    }
    Ok(())
}

fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

// ---------- Prepare ----------

/// A table ready to store.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub name: String,
    pub definition: TableDefinition,
    pub warnings: Vec<Warning>,
}

/// Check and canonicalise a table. `id` is the table's id (new or existing),
/// `existing` its current definition when updating, and `board` every table
/// on the board, live or in the trash, including this one.
pub fn prepare(
    id: &str,
    name: &str,
    input: &Value,
    existing: Option<&TableDefinition>,
    board: &[BoardTable],
) -> AppResult<Prepared> {
    check_identifier("name", name, "table")?;
    if let Some(clash) = board
        .iter()
        .find(|t| t.live && t.id != id && same_name(&t.name, name))
    {
        return Err(AppError::invalid(
            "name",
            format!(
                "there's already a table called `{}` on this board",
                clash.name
            ),
        ));
    }

    let input: DefinitionInput = content::parse("definition", input)?;
    if input.columns.len() > MAX_COLUMNS {
        return Err(AppError::invalid(
            "definition.columns",
            format!("at most {MAX_COLUMNS} columns per table"),
        ));
    }
    let mut warnings = Vec::new();
    let ids = assign_column_ids(id, &input.columns, existing, board)?;

    // Columns, without references (they may point at columns later in the list).
    let mut columns = Vec::with_capacity(input.columns.len());
    let mut seen: Vec<&str> = Vec::new();
    for (i, (c, col_id)) in input.columns.iter().zip(&ids).enumerate() {
        let path = format!("definition.columns[{i}]");
        check_identifier(&format!("{path}.name"), &c.name, "column")?;
        if let Some(dup) = seen.iter().find(|n| same_name(n, &c.name)) {
            return Err(AppError::invalid(
                format!("{path}.name"),
                format!("there's already a column called `{dup}` in this table"),
            ));
        }
        seen.push(&c.name);
        check_column(&path, c)?;
        columns.push(SchemaColumn {
            id: col_id.clone(),
            name: c.name.clone(),
            column_type: c.column_type,
            length: c.length,
            precision: c.precision,
            scale: c.scale,
            values: c.values.clone(),
            nullable: c.nullable,
            default: c.default.clone(),
            primary: c.primary,
            unique: c.unique,
            generated: c.generated,
            references: None,
            label: tidy(c.label.as_deref()),
            comment: tidy(c.comment.as_deref()),
            native_type: tidy(c.native_type.as_deref()),
        });
    }

    // References, resolved against the board and this table as it will be.
    let this = BoardTable {
        id: id.to_string(),
        name: name.to_string(),
        definition: TableDefinition {
            comment: None,
            columns: columns.clone(),
            indexes: Vec::new(),
        },
        live: true,
    };
    for (i, c) in input.columns.iter().enumerate() {
        let Some(r) = &c.references else { continue };
        let path = format!("definition.columns[{i}].references");
        let (target, column) = resolve_reference(&path, r, &this, board)?;
        let from = &columns[i];
        if from.column_type != column.column_type {
            warnings.push(warn(
                &path,
                format!(
                    "`{}` is {} but `{}.{}` is {}",
                    from.name,
                    from.column_type.as_str(),
                    target.name,
                    column.name,
                    column.column_type.as_str()
                ),
            ));
        }
        if !column.primary && !column.unique {
            warnings.push(warn(
                &path,
                format!(
                    "`{}.{}` isn't a primary key or unique, so it may not identify one row",
                    target.name, column.name
                ),
            ));
        }
        columns[i].references = Some(Reference {
            table_id: target.id.clone(),
            column_id: column.id.clone(),
            table: None,
            column: None,
        });
    }

    let indexes = input
        .indexes
        .iter()
        .enumerate()
        .map(|(k, ix)| resolve_index(&format!("definition.indexes[{k}]"), ix, &columns))
        .collect::<AppResult<Vec<_>>>()?;

    if !columns.is_empty() && !columns.iter().any(|c| c.primary) {
        warnings.push(warn(
            "definition.columns",
            format!("`{name}` has no primary key"),
        ));
    }

    // Other tables may point at columns this update removes.
    let kept: HashSet<&str> = columns.iter().map(|c| c.id.as_str()).collect();
    let dangling: Vec<String> = referencing(id, board)
        .into_iter()
        .filter(|r| !kept.contains(r.to_column.as_str()))
        .map(|r| {
            let gone = existing
                .and_then(|d| d.column(&r.to_column))
                .map(|c| c.name.clone())
                .unwrap_or_default();
            format!("`{}` (used by {})", gone, r.from)
        })
        .collect();
    if !dangling.is_empty() {
        return Err(AppError::invalid(
            "definition.columns",
            format!(
                "other tables refer to columns this would remove: {}; change those references first",
                dangling.join(", ")
            ),
        ));
    }

    let definition = TableDefinition {
        comment: tidy(input.comment.as_deref()),
        columns,
        indexes,
    };
    let size = serde_json::to_string(&definition)
        .map_err(|e| AppError::Internal(e.into()))?
        .len();
    if size > MAX_CONTENT_BYTES {
        return Err(AppError::invalid(
            "definition",
            format!(
                "a table definition is at most {} KB",
                MAX_CONTENT_BYTES / 1024
            ),
        ));
    }
    Ok(Prepared {
        name: name.to_string(),
        definition,
        warnings,
    })
}

fn tidy(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The id for each input column (spec 5.4, "Ids and input by name"): the id
/// it was sent with; else the id of the existing column with the same name;
/// else a new one.
fn assign_column_ids(
    table: &str,
    columns: &[ColumnInput],
    existing: Option<&TableDefinition>,
    board: &[BoardTable],
) -> AppResult<Vec<String>> {
    let elsewhere: HashSet<&str> = board
        .iter()
        .filter(|t| t.id != table)
        .flat_map(|t| t.definition.columns.iter().map(|c| c.id.as_str()))
        .collect();
    let mut claimed: HashSet<String> = HashSet::new();
    for (i, c) in columns.iter().enumerate() {
        let Some(id) = &c.id else { continue };
        let path = format!("definition.columns[{i}].id");
        if !is_id(id, "col_") {
            return Err(AppError::invalid(
                path,
                "column ids look like `col_` and a ULID; leave `id` out for a new column",
            ));
        }
        if elsewhere.contains(id.as_str()) {
            return Err(AppError::invalid(
                path,
                "that id belongs to a column in another table; leave `id` out for a new column",
            ));
        }
        if !claimed.insert(id.clone()) {
            return Err(AppError::invalid(path, "two columns have the same id"));
        }
    }
    Ok(columns
        .iter()
        .map(|c| {
            if let Some(id) = &c.id {
                return id.clone();
            }
            let matched = existing.and_then(|d| {
                d.columns
                    .iter()
                    .find(|e| same_name(&e.name, &c.name) && !claimed.contains(&e.id))
            });
            match matched {
                Some(e) => {
                    claimed.insert(e.id.clone());
                    e.id.clone()
                }
                None => column_id(),
            }
        })
        .collect())
}

/// Extras only on their type, and defaults that make sense for it.
fn check_column(path: &str, c: &ColumnInput) -> AppResult<()> {
    use ColumnType as T;
    let t = c.column_type;
    let only = |field: &str, on: &str| {
        AppError::invalid(
            format!("{path}.{field}"),
            format!("`{field}` applies to {on} columns only"),
        )
    };
    if c.length.is_some() && t != T::String {
        return Err(only("length", "string"));
    }
    if let Some(len) = c.length
        && !(1..=MAX_LENGTH).contains(&len)
    {
        return Err(AppError::invalid(
            format!("{path}.length"),
            format!("between 1 and {MAX_LENGTH}"),
        ));
    }
    if t != T::Decimal {
        if c.precision.is_some() {
            return Err(only("precision", "decimal"));
        }
        if c.scale.is_some() {
            return Err(only("scale", "decimal"));
        }
    }
    if let Some(p) = c.precision
        && !(1..=MAX_PRECISION).contains(&p)
    {
        return Err(AppError::invalid(
            format!("{path}.precision"),
            format!("between 1 and {MAX_PRECISION}"),
        ));
    }
    if let Some(s) = c.scale {
        let Some(p) = c.precision else {
            return Err(AppError::invalid(
                format!("{path}.scale"),
                "give `precision` too",
            ));
        };
        if s > p || s > MAX_SCALE {
            return Err(AppError::invalid(
                format!("{path}.scale"),
                format!("`scale` ({s}) can't be more than `precision` ({p})"),
            ));
        }
    }
    match (&c.values, t) {
        (Some(_), t) if t != T::Enum => return Err(only("values", "enum")),
        (None, T::Enum) => {
            return Err(AppError::invalid(
                format!("{path}.values"),
                "enum columns need `values`",
            ));
        }
        (Some(values), _) => {
            if values.is_empty() {
                return Err(AppError::invalid(
                    format!("{path}.values"),
                    "give at least one value",
                ));
            }
            let mut seen = HashSet::new();
            for (k, v) in values.iter().enumerate() {
                if v.is_empty() {
                    return Err(AppError::invalid(
                        format!("{path}.values[{k}]"),
                        "values can't be empty",
                    ));
                }
                if !seen.insert(v) {
                    return Err(AppError::invalid(
                        format!("{path}.values[{k}]"),
                        format!("`{v}` is listed twice"),
                    ));
                }
            }
        }
        (None, _) => {}
    }

    if let Some(g) = c.generated {
        let fits = match g {
            Generated::Increment => matches!(t, T::Integer | T::Bigint),
            Generated::Uuid => matches!(t, T::Uuid | T::String),
            Generated::Now | Generated::NowOnUpdate => matches!(t, T::Datetime | T::Date | T::Time),
        };
        if !fits {
            return Err(AppError::invalid(
                format!("{path}.generated"),
                format!(
                    "`{}` doesn't fit a {} column",
                    serde_json::to_value(g)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default(),
                    t.as_str()
                ),
            ));
        }
        if c.default.is_some() {
            return Err(AppError::invalid(
                format!("{path}.default"),
                "a generated column can't also have a default",
            ));
        }
    }
    if c.primary && c.nullable {
        return Err(AppError::invalid(
            format!("{path}.nullable"),
            "a primary key column can't be nullable",
        ));
    }

    if let Some(d) = &c.default {
        let bad = |why: String| AppError::invalid(format!("{path}.default"), why);
        match t {
            T::Enum => {
                let values = c.values.as_deref().unwrap_or_default();
                if !values.iter().any(|v| v == d) {
                    return Err(bad(format!(
                        "`{d}` isn't one of the values: {}",
                        values.join(", ")
                    )));
                }
            }
            T::Boolean if d != "true" && d != "false" => {
                return Err(bad("a boolean default is `true` or `false`".into()));
            }
            T::Integer | T::Bigint if d.parse::<i64>().is_err() => {
                return Err(bad(format!("`{d}` isn't a whole number")));
            }
            T::Decimal if d.parse::<f64>().map(|f| !f.is_finite()).unwrap_or(true) => {
                return Err(bad(format!("`{d}` isn't a number")));
            }
            T::String => {
                if let Some(len) = c.length
                    && d.chars().count() > len as usize
                {
                    return Err(bad(format!("longer than the column's length ({len})")));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn resolve_reference<'a>(
    path: &str,
    r: &ReferenceInput,
    this: &'a BoardTable,
    board: &'a [BoardTable],
) -> AppResult<(&'a BoardTable, &'a SchemaColumn)> {
    let live = || {
        board
            .iter()
            .filter(|t| t.live && t.id != this.id)
            .chain(std::iter::once(this))
    };
    let target = match (&r.table_id, &r.table) {
        (Some(id), _) => live().find(|t| &t.id == id).ok_or_else(|| {
            AppError::invalid(
                format!("{path}.table_id"),
                "no table with this id on the board",
            )
        })?,
        (None, Some(name)) => live().find(|t| same_name(&t.name, name)).ok_or_else(|| {
            let mut names: Vec<&str> = live().map(|t| t.name.as_str()).collect();
            names.sort_unstable();
            AppError::invalid(
                format!("{path}.table"),
                format!(
                    "no table called `{name}` on this board (tables: {})",
                    names.join(", ")
                ),
            )
        })?,
        (None, None) => {
            return Err(AppError::invalid(
                path,
                "give the referenced `table` (by name) or `table_id`",
            ));
        }
    };
    let def = &target.definition;
    let column = match (&r.column_id, &r.column) {
        (Some(id), _) => def.column(id).ok_or_else(|| {
            AppError::invalid(
                format!("{path}.column_id"),
                format!("`{}` has no column with this id", target.name),
            )
        })?,
        (None, Some(name)) => def
            .columns
            .iter()
            .find(|c| same_name(&c.name, name))
            .ok_or_else(|| {
                AppError::invalid(
                    format!("{path}.column"),
                    format!("`{}` has no column called `{name}`", target.name),
                )
            })?,
        (None, None) => def.primary_key().ok_or_else(|| {
            AppError::invalid(
                format!("{path}.column"),
                format!(
                    "`{}` doesn't have a single primary key column; name the referenced `column`",
                    target.name
                ),
            )
        })?,
    };
    Ok((target, column))
}

fn resolve_index(path: &str, ix: &IndexInput, columns: &[SchemaColumn]) -> AppResult<SchemaIndex> {
    let ids: Vec<String> = match (&ix.column_ids, &ix.columns) {
        (Some(ids), _) => ids
            .iter()
            .enumerate()
            .map(|(k, id)| {
                columns
                    .iter()
                    .find(|c| &c.id == id)
                    .map(|c| c.id.clone())
                    .ok_or_else(|| {
                        AppError::invalid(
                            format!("{path}.column_ids[{k}]"),
                            "no column with this id in the table",
                        )
                    })
            })
            .collect::<AppResult<_>>()?,
        (None, Some(names)) => names
            .iter()
            .enumerate()
            .map(|(k, name)| {
                columns
                    .iter()
                    .find(|c| same_name(&c.name, name))
                    .map(|c| c.id.clone())
                    .ok_or_else(|| {
                        AppError::invalid(
                            format!("{path}.columns[{k}]"),
                            format!("no column called `{name}` in the table"),
                        )
                    })
            })
            .collect::<AppResult<_>>()?,
        (None, None) => {
            return Err(AppError::invalid(
                path,
                "give the index's `columns` (names) or `column_ids`",
            ));
        }
    };
    if ids.is_empty() {
        return Err(AppError::invalid(
            path,
            "an index needs at least one column",
        ));
    }
    let mut seen = HashSet::new();
    if let Some(dup) = ids.iter().find(|id| !seen.insert(*id)) {
        let name = columns
            .iter()
            .find(|c| &c.id == dup)
            .map(|c| c.name.as_str())
            .unwrap_or("");
        return Err(AppError::invalid(
            path,
            format!("`{name}` is in the index twice"),
        ));
    }
    Ok(SchemaIndex {
        column_ids: ids,
        unique: ix.unique,
        columns: None,
    })
}

// ---------- References between tables ----------

/// A column in another live table that references `table_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referrer {
    pub table_id: String,
    pub column_id: String,
    /// The referenced column.
    pub to_column: String,
    /// `table.column`, for messages.
    pub from: String,
}

/// Live columns elsewhere on the board that reference `table_id`
/// (self-references don't count).
pub fn referencing(table_id: &str, board: &[BoardTable]) -> Vec<Referrer> {
    board
        .iter()
        .filter(|t| t.live && t.id != table_id)
        .flat_map(|t| {
            t.definition.columns.iter().filter_map(move |c| {
                let r = c.references.as_ref()?;
                (r.table_id == table_id).then(|| Referrer {
                    table_id: t.id.clone(),
                    column_id: c.id.clone(),
                    to_column: r.column_id.clone(),
                    from: format!("{}.{}", t.name, c.name),
                })
            })
        })
        .collect()
}

/// Tables `def` references that aren't live (blocks an undelete).
pub fn missing_targets(table_id: &str, def: &TableDefinition, board: &[BoardTable]) -> Vec<String> {
    let mut missing: Vec<String> = def
        .columns
        .iter()
        .filter_map(|c| c.references.as_ref())
        .filter(|r| r.table_id != table_id)
        .filter_map(|r| {
            let t = board.iter().find(|t| t.id == r.table_id);
            match t {
                Some(t) if t.live => None,
                Some(t) => Some(t.name.clone()),
                None => Some(r.table_id.clone()),
            }
        })
        .collect();
    missing.sort();
    missing.dedup();
    missing
}

/// `def` with names next to ids, for responses: `references.table` and
/// `.column`, and each index's `columns`.
pub fn with_names(
    table_id: &str,
    table_name: &str,
    def: &TableDefinition,
    board: &[BoardTable],
) -> TableDefinition {
    let by_id: HashMap<&str, &BoardTable> = board.iter().map(|t| (t.id.as_str(), t)).collect();
    let mut out = def.clone();
    for c in &mut out.columns {
        let Some(r) = &mut c.references else { continue };
        let target = if r.table_id == table_id {
            Some((def, table_name))
        } else {
            by_id
                .get(r.table_id.as_str())
                .map(|t| (&t.definition, t.name.as_str()))
        };
        if let Some((tdef, tname)) = target {
            r.table = Some(tname.to_string());
            r.column = tdef.column(&r.column_id).map(|c| c.name.clone());
        }
    }
    for ix in &mut out.indexes {
        ix.columns = Some(
            ix.column_ids
                .iter()
                .map(|id| def.column(id).map(|c| c.name.clone()).unwrap_or_default())
                .collect(),
        );
    }
    out
}

/// Remove every reference to `table_id` from `def`. Returns whether anything changed.
pub fn drop_references_to(table_id: &str, def: &mut TableDefinition) -> bool {
    let mut changed = false;
    for c in &mut def.columns {
        if c.references
            .as_ref()
            .is_some_and(|r| r.table_id == table_id)
        {
            c.references = None;
            changed = true;
        }
    }
    changed
}

/// A column's type as people write it: `string(120)`, `decimal(10,2)`, or
/// the native type when one is recorded.
pub fn type_label(c: &SchemaColumn) -> String {
    if let Some(native) = &c.native_type {
        return native.clone();
    }
    match (c.column_type, c.length, c.precision, c.scale) {
        (ColumnType::String, Some(len), _, _) => format!("string({len})"),
        (ColumnType::Decimal, _, Some(p), Some(s)) => format!("decimal({p},{s})"),
        (ColumnType::Decimal, _, Some(p), None) => format!("decimal({p})"),
        (t, ..) => t.as_str().to_string(),
    }
}

/// Text for search: the comment, then each column's name, label and comment.
pub fn search_body(def: &TableDefinition) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(c) = &def.comment {
        parts.push(c);
    }
    for c in &def.columns {
        parts.push(&c.name);
        parts.extend(c.label.as_deref());
        parts.extend(c.comment.as_deref());
    }
    parts.join("\n")
}

/// Every page in a sitemap tree.
pub fn pages(root: &SitemapNode) -> Vec<&SitemapNode> {
    root.walk()
}

#[cfg(test)]
mod tests;
