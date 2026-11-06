use serde_json::{Value, json};

use super::er;
use super::*;

fn invalid(r: AppResult<Prepared>) -> (String, String) {
    match r {
        Err(AppError::Invalid { path, message }) => (path, message),
        Err(other) => panic!("expected invalid, got {other:?}"),
        Ok(p) => panic!("expected invalid, got {:?}", p.definition),
    }
}

/// Prepare a new table on `board` and add it, returning its id.
fn add(board: &mut Vec<BoardTable>, name: &str, def: Value) -> String {
    let id = table_id();
    let p = prepare(&id, name, &def, None, board).unwrap();
    board.push(BoardTable {
        id: id.clone(),
        name: name.into(),
        definition: p.definition,
        live: true,
    });
    id
}

fn users(board: &mut Vec<BoardTable>) -> String {
    add(
        board,
        "users",
        json!({ "columns": [
            { "name": "id", "type": "bigint", "primary": true, "generated": "increment" },
            { "name": "email", "type": "string", "length": 191, "unique": true },
        ] }),
    )
}

fn new(name: &str, def: Value, board: &[BoardTable]) -> AppResult<Prepared> {
    prepare(&table_id(), name, &def, None, board)
}

#[test]
fn assigns_ids_and_canonicalises() {
    let p = new(
        "orders",
        json!({ "comment": "  Customer orders ", "columns": [
            { "name": "id", "type": "bigint", "primary": true },
            { "name": "total", "type": "decimal", "precision": 10, "scale": 2, "default": 0 },
            { "name": "paid", "type": "boolean", "default": false, "label": " " },
        ] }),
        &[],
    )
    .unwrap();
    let d = &p.definition;
    assert_eq!(d.comment.as_deref(), Some("Customer orders"));
    assert!(d.columns.iter().all(|c| is_id(&c.id, "col_")));
    assert_eq!(
        d.columns[1].default.as_deref(),
        Some("0"),
        "numbers become text"
    );
    assert_eq!(d.columns[2].default.as_deref(), Some("false"));
    assert_eq!(d.columns[2].label, None, "blank labels dropped");
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
}

#[test]
fn ids_survive_renames_and_name_matches() {
    let mut board = Vec::new();
    let id = users(&mut board);
    let before = board[0].definition.clone();
    let (id_col, email_col) = (&before.columns[0].id, &before.columns[1].id);

    // `email` sent without its id keeps it (matched by name, any case);
    // `id` renamed to `user_id` keeps its id because the id was sent.
    let p = prepare(
        &id,
        "users",
        &json!({ "columns": [
            { "name": "EMAIL", "type": "string" },
            { "id": id_col, "name": "user_id", "type": "bigint", "primary": true },
            { "name": "name", "type": "string" },
        ] }),
        Some(&before),
        &board,
    )
    .unwrap();
    let cols = &p.definition.columns;
    assert_eq!(&cols[0].id, email_col);
    assert_eq!(&cols[1].id, id_col);
    assert!(
        cols[2].id != *id_col && cols[2].id != *email_col,
        "new column, new id"
    );
}

#[test]
fn bad_or_borrowed_ids_are_refused() {
    let mut board = Vec::new();
    users(&mut board);
    let theirs = board[0].definition.columns[0].id.clone();
    let (path, _) = invalid(new(
        "orders",
        json!({ "columns": [{ "id": "col_1", "name": "id", "type": "bigint" }] }),
        &board,
    ));
    assert_eq!(path, "definition.columns[0].id");
    let (path, message) = invalid(new(
        "orders",
        json!({ "columns": [{ "id": theirs, "name": "id", "type": "bigint" }] }),
        &board,
    ));
    assert_eq!(path, "definition.columns[0].id");
    assert!(message.contains("another table"), "{message}");
}

#[test]
fn names_follow_the_identifier_rules() {
    assert_eq!(invalid(new("2fa", json!({}), &[])).0, "name");
    assert_eq!(invalid(new("order items", json!({}), &[])).0, "name");
    assert_eq!(invalid(new(&"x".repeat(65), json!({}), &[])).0, "name");
    let (path, _) = invalid(new(
        "orders",
        json!({ "columns": [{ "name": "total-price", "type": "decimal" }] }),
        &[],
    ));
    assert_eq!(path, "definition.columns[0].name");
    assert!(new("_meta", json!({}), &[]).is_ok());
}

#[test]
fn names_are_unique_ignoring_case() {
    let mut board = Vec::new();
    users(&mut board);
    let (path, message) = invalid(new("Users", json!({}), &board));
    assert_eq!(path, "name");
    assert!(message.contains("already a table"), "{message}");

    // A trashed table's name is free.
    board[0].live = false;
    assert!(new("users", json!({}), &board).is_ok());

    let (path, _) = invalid(new(
        "orders",
        json!({ "columns": [{ "name": "total", "type": "integer" }, { "name": "Total", "type": "integer" }] }),
        &[],
    ));
    assert_eq!(path, "definition.columns[1].name");
}

#[test]
fn typos_and_unknown_types_name_the_field() {
    let (path, message) = invalid(new(
        "orders",
        json!({ "columns": [{ "name": "a", "type": "string", "nulable": true }] }),
        &[],
    ));
    assert_eq!(path, "definition.columns[0].nulable");
    assert!(message.contains("unknown field"), "{message}");
    let (path, message) = invalid(new(
        "orders",
        json!({ "columns": [{ "name": "a", "type": "varchar(20)" }] }),
        &[],
    ));
    assert_eq!(path, "definition.columns[0].type");
    assert!(
        message.contains("string"),
        "lists the logical types: {message}"
    );
}

#[test]
fn extras_only_on_their_type() {
    let col = |c: Value| new("t", json!({ "columns": [c] }), &[]);
    let path = |c: Value| invalid(col(c)).0;
    assert_eq!(
        path(json!({ "name": "a", "type": "text", "length": 10 })),
        "definition.columns[0].length"
    );
    assert_eq!(
        path(json!({ "name": "a", "type": "integer", "precision": 10 })),
        "definition.columns[0].precision"
    );
    assert_eq!(
        path(json!({ "name": "a", "type": "decimal", "precision": 4, "scale": 6 })),
        "definition.columns[0].scale"
    );
    assert_eq!(
        path(json!({ "name": "a", "type": "decimal", "scale": 2 })),
        "definition.columns[0].scale"
    );
    assert_eq!(
        path(json!({ "name": "a", "type": "string", "values": ["x"] })),
        "definition.columns[0].values"
    );
    assert_eq!(
        path(json!({ "name": "a", "type": "enum" })),
        "definition.columns[0].values"
    );
    assert_eq!(
        path(json!({ "name": "a", "type": "enum", "values": [] })),
        "definition.columns[0].values"
    );
    assert_eq!(
        path(json!({ "name": "a", "type": "enum", "values": ["x", "x"] })),
        "definition.columns[0].values[1]"
    );
    assert_eq!(
        path(json!({ "name": "a", "type": "string", "length": 0 })),
        "definition.columns[0].length"
    );
    assert!(col(json!({ "name": "a", "type": "decimal", "precision": 8, "scale": 2 })).is_ok());
}

#[test]
fn defaults_must_fit_the_type() {
    let path = |c: Value| invalid(new("t", json!({ "columns": [c] }), &[])).0;
    let ok = |c: Value| new("t", json!({ "columns": [c] }), &[]).is_ok();
    assert_eq!(
        path(json!({ "name": "s", "type": "enum", "values": ["a", "b"], "default": "c" })),
        "definition.columns[0].default"
    );
    assert!(ok(
        json!({ "name": "s", "type": "enum", "values": ["a", "b"], "default": "b" })
    ));
    assert_eq!(
        path(json!({ "name": "b", "type": "boolean", "default": "yes" })),
        "definition.columns[0].default"
    );
    assert_eq!(
        path(json!({ "name": "n", "type": "integer", "default": "1.5" })),
        "definition.columns[0].default"
    );
    assert!(ok(json!({ "name": "n", "type": "bigint", "default": -3 })));
    assert_eq!(
        path(json!({ "name": "d", "type": "decimal", "default": "abc" })),
        "definition.columns[0].default"
    );
    assert_eq!(
        path(json!({ "name": "s", "type": "string", "length": 2, "default": "abc" })),
        "definition.columns[0].default"
    );
    // Not checkable: accepted as written.
    assert!(ok(
        json!({ "name": "d", "type": "date", "default": "CURRENT_DATE" })
    ));
    assert_eq!(
        path(json!({ "name": "j", "type": "json", "default": { "a": 1 } })),
        "definition.columns[0].default"
    );
}

#[test]
fn generated_and_key_rules() {
    let path = |c: Value| invalid(new("t", json!({ "columns": [c] }), &[])).0;
    assert_eq!(
        path(json!({ "name": "id", "type": "string", "generated": "increment" })),
        "definition.columns[0].generated"
    );
    assert_eq!(
        path(json!({ "name": "at", "type": "boolean", "generated": "now" })),
        "definition.columns[0].generated"
    );
    assert_eq!(
        path(json!({ "name": "at", "type": "datetime", "generated": "now", "default": "x" })),
        "definition.columns[0].default"
    );
    assert_eq!(
        path(json!({ "name": "id", "type": "uuid", "primary": true, "nullable": true })),
        "definition.columns[0].nullable"
    );
    let p = new(
        "t",
        json!({ "columns": [{ "name": "id", "type": "uuid", "generated": "uuid", "primary": true }] }),
        &[],
    );
    assert!(p.is_ok());
}

#[test]
fn references_resolve_by_name_or_id() {
    let mut board = Vec::new();
    let users_id = users(&mut board);
    let pk = board[0].definition.columns[0].id.clone();

    let p = new(
        "orders",
        json!({ "columns": [
            { "name": "id", "type": "bigint", "primary": true },
            { "name": "user_id", "type": "bigint", "references": { "table": "Users", "column": "id" } },
            { "name": "owner_id", "type": "bigint", "references": { "table": "users" } },
            { "name": "buyer_id", "type": "bigint", "references": { "table_id": users_id, "column_id": pk, "table": "stale name" } },
        ] }),
        &board,
    )
    .unwrap();
    for c in &p.definition.columns[1..] {
        let r = c.references.as_ref().unwrap();
        assert_eq!((&r.table_id, &r.column_id), (&users_id, &pk), "{}", c.name);
        assert_eq!(r.table, None, "names aren't stored");
    }
}

#[test]
fn references_can_point_at_the_same_table() {
    let p = new(
        "categories",
        json!({ "columns": [
            { "name": "parent_id", "type": "bigint", "nullable": true, "references": { "table": "categories", "column": "id" } },
            { "name": "id", "type": "bigint", "primary": true },
        ] }),
        &[],
    )
    .unwrap();
    let cols = &p.definition.columns;
    assert_eq!(cols[0].references.as_ref().unwrap().column_id, cols[1].id);
}

#[test]
fn bad_references_explain_themselves() {
    let mut board = Vec::new();
    users(&mut board);
    let path = |r: Value| {
        invalid(new(
            "orders",
            json!({ "columns": [{ "name": "user_id", "type": "bigint", "references": r }] }),
            &board,
        ))
    };
    let (p, m) = path(json!({ "table": "customers" }));
    assert_eq!(p, "definition.columns[0].references.table");
    assert!(
        m.contains("tables: orders, users"),
        "lists what exists: {m}"
    );
    assert_eq!(
        path(json!({ "table": "users", "column": "nope" })).0,
        "definition.columns[0].references.column"
    );
    assert_eq!(
        path(json!({ "table_id": "tbl_x" })).0,
        "definition.columns[0].references.table_id"
    );
    assert_eq!(path(json!({})).0, "definition.columns[0].references");

    // A trashed table can't be referenced.
    let mut trashed = board.clone();
    trashed[0].live = false;
    let r = new(
        "orders",
        json!({ "columns": [{ "name": "user_id", "type": "bigint", "references": { "table": "users" } }] }),
        &trashed,
    );
    assert_eq!(invalid(r).0, "definition.columns[0].references.table");
}

#[test]
fn reference_mismatches_are_warnings() {
    let mut board = Vec::new();
    users(&mut board);
    let p = new(
        "orders",
        json!({ "columns": [
            { "name": "id", "type": "bigint", "primary": true },
            { "name": "user_id", "type": "integer", "references": { "table": "users" } },
            { "name": "user_email", "type": "string", "references": { "table": "users", "column": "email" } },
        ] }),
        &board,
    )
    .unwrap();
    assert_eq!(p.warnings.len(), 1, "{:?}", p.warnings);
    assert_eq!(p.warnings[0].path, "definition.columns[1].references");
    assert!(p.warnings[0].message.contains("integer"));

    let mut loose = board.clone();
    loose[0].definition.columns[1].unique = false;
    let p = new(
        "orders",
        json!({ "columns": [
            { "name": "id", "type": "bigint", "primary": true },
            { "name": "user_email", "type": "string", "references": { "table": "users", "column": "email" } },
        ] }),
        &loose,
    )
    .unwrap();
    assert!(
        p.warnings[0]
            .message
            .contains("isn't a primary key or unique")
    );
}

#[test]
fn a_table_without_a_primary_key_is_a_warning() {
    let p = new(
        "log",
        json!({ "columns": [{ "name": "line", "type": "text" }] }),
        &[],
    )
    .unwrap();
    assert_eq!(p.warnings[0].path, "definition.columns");
    assert!(new("draft", json!({}), &[]).unwrap().warnings.is_empty());
}

#[test]
fn indexes_resolve_and_must_exist() {
    let def = |ix: Value| {
        json!({ "columns": [
            { "name": "id", "type": "bigint", "primary": true },
            { "name": "a", "type": "string" }, { "name": "b", "type": "string" },
        ], "indexes": [ix] })
    };
    let p = new(
        "t",
        def(json!({ "columns": ["a", "B"], "unique": true })),
        &[],
    )
    .unwrap();
    let ix = &p.definition.indexes[0];
    assert_eq!(
        ix.column_ids,
        [
            p.definition.columns[1].id.clone(),
            p.definition.columns[2].id.clone()
        ]
    );
    assert!(ix.unique);

    let path = |ix: Value| invalid(new("t", def(ix), &[])).0;
    assert_eq!(
        path(json!({ "columns": ["c"] })),
        "definition.indexes[0].columns[0]"
    );
    assert_eq!(
        path(json!({ "column_ids": ["col_01JAAAAAAAAAAAAAAAAAAAAAAA"] })),
        "definition.indexes[0].column_ids[0]"
    );
    assert_eq!(path(json!({ "columns": [] })), "definition.indexes[0]");
    assert_eq!(
        path(json!({ "columns": ["a", "a"] })),
        "definition.indexes[0]"
    );
    assert_eq!(path(json!({ "unique": true })), "definition.indexes[0]");
}

#[test]
fn removing_a_referenced_column_is_refused() {
    let mut board = Vec::new();
    let users_id = users(&mut board);
    add(
        &mut board,
        "orders",
        json!({ "columns": [{ "name": "user_id", "type": "bigint", "references": { "table": "users" } }] }),
    );
    let current = board[0].definition.clone();
    let (path, message) = invalid(prepare(
        &users_id,
        "users",
        &json!({ "columns": [{ "name": "email", "type": "string" }] }),
        Some(&current),
        &board,
    ));
    assert_eq!(path, "definition.columns");
    assert!(
        message.contains("`id` (used by orders.user_id)"),
        "{message}"
    );

    // Renaming it is fine: the id survives, so the reference does too.
    let renamed = prepare(
        &users_id,
        "users",
        &json!({ "columns": [{ "id": current.columns[0].id, "name": "user_id", "type": "bigint", "primary": true }] }),
        Some(&current),
        &board,
    );
    assert!(renamed.is_ok());
}

#[test]
fn referrers_and_missing_targets() {
    let mut board = Vec::new();
    let users_id = users(&mut board);
    let orders_id = add(
        &mut board,
        "orders",
        json!({ "columns": [
            { "name": "id", "type": "bigint", "primary": true },
            { "name": "user_id", "type": "bigint", "references": { "table": "users" } },
            { "name": "parent_id", "type": "bigint", "references": { "table": "orders" } },
        ] }),
    );
    let refs = referencing(&users_id, &board);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].from, "orders.user_id");
    assert!(
        referencing(&orders_id, &board).is_empty(),
        "self-references don't count"
    );

    let orders_def = board[1].definition.clone();
    assert!(missing_targets(&orders_id, &orders_def, &board).is_empty());
    board[0].live = false;
    assert_eq!(missing_targets(&orders_id, &orders_def, &board), ["users"]);
    assert!(referencing(&orders_id, &board).is_empty());

    let mut def = orders_def.clone();
    assert!(drop_references_to(&users_id, &mut def));
    assert!(def.columns[1].references.is_none());
    assert!(def.columns[2].references.is_some(), "other references stay");
    assert!(!drop_references_to(&users_id, &mut def));
}

#[test]
fn reads_carry_names_next_to_ids() {
    let mut board = Vec::new();
    users(&mut board);
    let orders_id = add(
        &mut board,
        "orders",
        json!({ "columns": [
            { "name": "id", "type": "bigint", "primary": true },
            { "name": "user_id", "type": "bigint", "references": { "table": "users" } },
            { "name": "parent_id", "type": "bigint", "references": { "table": "orders" } },
        ], "indexes": [{ "columns": ["user_id"] }] }),
    );
    let named = with_names(&orders_id, "orders", &board[1].definition, &board);
    let r = named.columns[1].references.as_ref().unwrap();
    assert_eq!(
        (r.table.as_deref(), r.column.as_deref()),
        (Some("users"), Some("id"))
    );
    let own = named.columns[2].references.as_ref().unwrap();
    assert_eq!(own.table.as_deref(), Some("orders"));
    assert_eq!(
        named.indexes[0].columns.as_deref(),
        Some(&["user_id".to_string()][..])
    );
}

#[test]
fn type_labels() {
    let p = new(
        "t",
        json!({ "columns": [
            { "name": "a", "type": "string", "length": 120 },
            { "name": "b", "type": "decimal", "precision": 10, "scale": 2 },
            { "name": "c", "type": "json", "native_type": "jsonb" },
            { "name": "d", "type": "string" },
        ] }),
        &[],
    )
    .unwrap();
    let labels: Vec<String> = p.definition.columns.iter().map(type_label).collect();
    assert_eq!(labels, ["string(120)", "decimal(10,2)", "jsonb", "string"]);
}

#[test]
fn er_cardinality_follows_the_foreign_key() {
    let mut board = Vec::new();
    let users_id = users(&mut board);
    let profiles_id = add(
        &mut board,
        "profiles",
        json!({ "columns": [
            { "name": "user_id", "type": "bigint", "primary": true, "references": { "table": "users" } },
        ] }),
    );
    let orders_id = add(
        &mut board,
        "orders",
        json!({ "columns": [
            { "name": "id", "type": "bigint", "primary": true },
            { "name": "user_id", "type": "bigint", "references": { "table": "users" } },
            { "name": "coupon_owner_id", "type": "bigint", "nullable": true, "unique": true, "references": { "table": "users" } },
        ] }),
    );

    // A card showing profiles and orders: users is a stub.
    let v = er::view(
        &[profiles_id.clone(), orders_id.clone(), "tbl_gone".into()],
        &board,
    );
    assert_eq!(v.shown, 2);
    assert_eq!(v.tables[2].id, users_id);
    assert!(v.is_stub(2));
    let m = er::mermaid(&v);
    assert!(m.contains("    users\n"), "stub as a bare entity:\n{m}");
    assert!(m.contains("bigint user_id PK, FK"), "{m}");
    assert!(
        m.contains(r#"users ||--o| profiles : "user_id""#),
        "PK FK is one-to-one:\n{m}"
    );
    assert!(m.contains(r#"users ||--o{ orders : "user_id""#), "{m}");
    assert!(
        m.contains(r#"users |o--o| orders : "coupon_owner_id""#),
        "{m}"
    );
    assert!(m.contains("bigint coupon_owner_id FK, UK"), "{m}");

    // With users on the card, it isn't a stub.
    let v = er::view(&[users_id, orders_id], &board);
    assert_eq!((v.shown, v.tables.len()), (2, 2));
}
