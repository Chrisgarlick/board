use serde_json::{Value, json};

use super::*;
use crate::schema;

fn board() -> Board {
    Board {
        id: "b1".into(),
        title: "Acme".into(),
        description: None,
        stack: Some("Laravel + MySQL".into()),
        client: None,
        schema_version: 3,
        archived: false,
        created_at: "2026-11-06T00:00:00.000Z".into(),
        updated_at: "2026-11-06T00:00:00.000Z".into(),
    }
}

fn item(
    id: &str,
    kind: &str,
    card_type: Option<&str>,
    title: Option<&str>,
    content: Value,
    x: f64,
    y: f64,
) -> Item {
    serde_json::from_value(json!({
        "id": id, "board_id": "b1", "kind": kind, "card_type": card_type, "frame_id": null,
        "title": title, "content": content, "style": null, "x": x, "y": y, "w": 160, "h": 72,
        "z": 0, "locked": false, "version": 1, "created_by": "user", "updated_by": "user",
        "content_updated_at": "", "created_at": "", "updated_at": ""
    }))
    .unwrap()
}

fn in_frame(mut i: Item, frame: &str) -> Item {
    i.frame_id = Some(frame.into());
    i
}

fn connector(id: &str, from: &str, to: &str, label: Option<&str>, created: &str) -> Connector {
    serde_json::from_value(json!({
        "id": id, "board_id": "b1", "from_item": from, "to_item": to,
        "from_anchor": "auto", "to_anchor": "auto", "label": label, "style": null,
        "created_by": "user", "created_at": created, "updated_at": created
    }))
    .unwrap()
}

fn tables() -> Vec<BoardTable> {
    let mut out: Vec<BoardTable> = Vec::new();
    for (name, def) in [
        (
            "users",
            json!({ "columns": [
                { "name": "id", "type": "bigint", "primary": true },
                { "name": "created_at", "type": "datetime", "label": "Joined" },
            ] }),
        ),
        (
            "orders",
            json!({ "columns": [
                { "name": "id", "type": "bigint", "primary": true },
                { "name": "user_id", "type": "bigint", "references": { "table": "users" } },
            ] }),
        ),
    ] {
        let id = schema::table_id();
        let p = schema::prepare(&id, name, &def, None, &out).unwrap();
        out.push(BoardTable {
            id,
            name: name.into(),
            definition: p.definition,
            live: true,
        });
    }
    out
}

fn model(items: &[Item], connectors: &[Connector], tables: &[BoardTable]) -> BoardModel {
    board_model(&board(), items, connectors, tables, &HashMap::new())
}

#[test]
fn tables_come_in_name_order_with_labels_and_names_filled_in() {
    let t = tables();
    let m = model(&[], &[], &t);
    assert_eq!(m.contract, "board-model");
    assert_eq!(m.contract_version, 1);
    assert_eq!(m.schema_version, 3);
    assert_eq!(m.board.stack.as_deref(), Some("Laravel + MySQL"));
    let names: Vec<&str> = m.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["orders", "users"]);
    let users = &m.tables[1];
    assert_eq!(
        users.columns[0].column.label.as_deref(),
        Some("ID"),
        "humanised"
    );
    assert_eq!(
        users.columns[1].column.label.as_deref(),
        Some("Joined"),
        "kept"
    );
    let fk = m.tables[0].columns[1].column.references.as_ref().unwrap();
    assert_eq!(fk.table.as_deref(), Some("users"));
    assert!(
        m.tables
            .iter()
            .flat_map(|t| &t.columns)
            .all(|c| c.hash.len() == 8)
    );

    // Trashed tables are left out.
    let mut trashed = t.clone();
    trashed[0].live = false;
    assert_eq!(model(&[], &[], &trashed).tables.len(), 1);
}

#[test]
fn hashes_change_only_when_a_column_does() {
    let t = tables();
    let before = model(&[], &[], &t);
    let col = |m: &BoardModel| m.tables[1].columns[1].hash.clone();

    // Renaming the referenced table changes names in the read, not the hash.
    let mut renamed = t.clone();
    renamed[0].name = "accounts".into();
    let m = model(&[], &[], &renamed);
    let user_id = |m: &BoardModel| {
        let orders = m.tables.iter().find(|x| x.id == t[1].id).unwrap();
        (
            orders.columns[1].hash.clone(),
            orders.columns[1].column.references.clone().unwrap().table,
        )
    };
    assert_eq!(
        user_id(&m).0,
        user_id(&before).0,
        "orders.user_id keeps its hash"
    );
    assert_eq!(
        user_id(&m).1.as_deref(),
        Some("accounts"),
        "while its read shows the new name"
    );

    // Changing a column changes its hash, and only its hash.
    let mut changed = t.clone();
    changed[0].definition.columns[1].nullable = true;
    let after = model(&[], &[], &changed);
    assert_ne!(col(&after), col(&before));
    assert_eq!(
        after.tables[1].columns[0].hash,
        before.tables[1].columns[0].hash
    );

    // The id isn't part of it: the same definition hashes the same.
    let mut c = t[0].definition.columns[1].clone();
    let h = column_hash(&c);
    c.id = "col_other".into();
    assert_eq!(column_hash(&c), h);
}

#[test]
fn features_flows_and_notes_are_in_reading_order() {
    let t = tables();
    let orders = t[1].id.clone();
    let items = vec![
        item(
            "n2",
            "card",
            Some("note"),
            Some("Second"),
            json!({ "markdown": "b" }),
            500.0,
            10.0,
        ),
        item(
            "n1",
            "card",
            Some("note"),
            Some("First"),
            json!({ "markdown": "a" }),
            0.0,
            150.0,
        ),
        item(
            "n3",
            "card",
            Some("note"),
            Some("Third"),
            json!({ "markdown": "c" }),
            0.0,
            400.0,
        ),
        item(
            "f1",
            "card",
            Some("feature"),
            Some("Checkout"),
            json!({ "status": "planned", "summary": "Pay", "stories": [], "acceptance": [{ "text": "Cards", "done": false }],
                    "table_ids": [orders, "tbl_trashed"], "page_ids": ["pg_gone"] }),
            0.0,
            900.0,
        ),
    ];
    let m = model(&items, &[], &t);
    let titles: Vec<&str> = m.notes.iter().map(|n| n.title.as_str()).collect();
    assert_eq!(
        titles,
        ["First", "Second", "Third"],
        "a 200-unit band is one row"
    );
    assert_eq!(m.features[0].status, board_model::FeatureStatus::Planned);
    assert_eq!(m.features[0].table_ids, [orders], "trashed tables left out");
    assert!(m.features[0].page_ids.is_empty(), "missing pages left out");
}

#[test]
fn flows_are_frames_with_connectors() {
    let sitemap = item(
        "s1",
        "card",
        Some("sitemap"),
        Some("Sitemap"),
        json!({ "root": { "id": "pg_home", "title": "Home", "slug": "/", "children": [{ "id": "pg_login", "title": "Log in", "slug": "/login" }] } }),
        2000.0,
        0.0,
    );
    let frame = item("fr", "frame", None, Some("Checkout"), json!({}), 0.0, 0.0);
    let a = in_frame(
        item(
            "a",
            "shape",
            None,
            None,
            json!({ "shape": "rect", "text": "Sign in", "page_id": "pg_login" }),
            10.0,
            10.0,
        ),
        "fr",
    );
    let b = in_frame(
        item(
            "b",
            "shape",
            None,
            None,
            json!({ "shape": "diamond", "text": "OK?" }),
            10.0,
            300.0,
        ),
        "fr",
    );
    let sticky = in_frame(
        item(
            "s",
            "sticky",
            None,
            None,
            json!({ "text": "idea" }),
            300.0,
            10.0,
        ),
        "fr",
    );
    let outside = item(
        "o",
        "shape",
        None,
        None,
        json!({ "shape": "rect", "text": "Elsewhere" }),
        900.0,
        900.0,
    );
    let empty = item(
        "fr2",
        "frame",
        None,
        Some("No arrows"),
        json!({}),
        0.0,
        1200.0,
    );
    let items = vec![sitemap, frame, a, b, sticky, outside, empty];
    let conns = vec![
        connector("c2", "b", "a", Some("no"), "2026-11-06T00:00:02.000Z"),
        connector("c1", "a", "b", None, "2026-11-06T00:00:01.000Z"),
        connector("c3", "a", "o", None, "2026-11-06T00:00:03.000Z"),
    ];
    let m = model(&items, &conns, &[]);
    assert_eq!(m.flows.len(), 1, "frames without connectors aren't flows");
    let f = &m.flows[0];
    assert_eq!(f.title, "Checkout");
    let ids: Vec<&str> = f.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["a", "b"], "no stickies, nothing outside the frame");
    assert_eq!(f.nodes[0].page_id.as_deref(), Some("pg_login"));
    assert_eq!(f.nodes[1].shape.as_deref(), Some("diamond"));
    assert_eq!(f.edges.len(), 2, "only edges between members");
    assert_eq!(f.edges[0].from, "a", "edges in the order they were drawn");
    assert_eq!(f.edges[1].label.as_deref(), Some("no"));
    assert_eq!(m.pages[0].children[0].slug.as_deref(), Some("/login"));
    assert!(m.warnings.is_empty());
}

#[test]
fn two_sitemaps_are_concatenated_with_a_warning() {
    let s = |id: &str, page: &str, y: f64| {
        item(
            id,
            "card",
            Some("sitemap"),
            Some("Map"),
            json!({ "root": { "id": page, "title": page } }),
            0.0,
            y,
        )
    };
    let m = model(&[s("s2", "pg_b", 600.0), s("s1", "pg_a", 0.0)], &[], &[]);
    let roots: Vec<&str> = m.pages.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(roots, ["pg_a", "pg_b"]);
    assert_eq!(m.warnings.len(), 1);
}

#[test]
fn the_etag_is_stable_and_follows_content() {
    let t = tables();
    let items = vec![item(
        "n1",
        "card",
        Some("note"),
        Some("A"),
        json!({ "markdown": "a" }),
        0.0,
        0.0,
    )];
    let one = etag(&model(&items, &[], &t));
    assert_eq!(one, etag(&model(&items, &[], &t)));
    assert!(one.starts_with('"') && one.ends_with('"'));
    // Moving a note within its row doesn't change the model; editing it does.
    let mut moved = items.clone();
    moved[0].x = 40.0;
    assert_eq!(
        etag(&model(&moved, &[], &t)),
        one,
        "no coordinates in the model"
    );
    let mut edited = items.clone();
    edited[0].content = json!({ "markdown": "b" });
    assert_ne!(etag(&model(&edited, &[], &t)), one);
}
