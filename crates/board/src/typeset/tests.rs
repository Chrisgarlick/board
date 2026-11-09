use serde_json::{Value, json};

use super::*;
use crate::schema;

fn board() -> Board {
    Board {
        id: "b1".into(),
        title: "Florist site".into(),
        description: Some("Online orders for a florist\nSecond line".into()),
        stack: None,
        client: Some("kritano".into()),
        schema_version: 1,
        archived: false,
        created_at: String::new(),
        updated_at: String::new(),
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

fn inside(mut i: Item, frame: &str) -> Item {
    i.frame_id = Some(frame.into());
    i
}

fn tables() -> Vec<BoardTable> {
    let mut out: Vec<BoardTable> = Vec::new();
    for (name, def) in [
        (
            "users",
            json!({ "comment": "People who order", "columns": [
            { "name": "id", "type": "bigint", "primary": true, "generated": "increment" },
            { "name": "email", "type": "string", "length": 191, "unique": true } ] }),
        ),
        (
            "orders",
            json!({ "columns": [
            { "name": "id", "type": "bigint", "primary": true },
            { "name": "user_id", "type": "bigint", "references": { "table": "users" } },
            { "name": "status", "type": "enum", "values": ["new", "sent"], "default": "new" },
            { "name": "note", "type": "text", "nullable": true, "comment": "Card *message*" } ] }),
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

fn picture(i: &Item) -> Option<Image> {
    Some(Image {
        url: format!("data:image/svg+xml;base64,{}", i.id),
    })
}

fn blocks(doc: &Value, page: usize) -> Vec<Value> {
    doc["pages"][page]["blocks"].as_array().unwrap().clone()
}

fn types(doc: &Value, page: usize) -> Vec<String> {
    blocks(doc, page)
        .iter()
        .map(|b| b["type"].as_str().unwrap().to_string())
        .collect()
}

/// A board with a flow frame holding a note, a loose feature and a schema card.
fn plan() -> (Vec<Item>, Vec<BoardTable>) {
    let t = tables();
    let orders = t[1].id.clone();
    let items = vec![
        item(
            "fr",
            "frame",
            None,
            Some("Checkout"),
            json!({ "description": "How a customer pays" }),
            0.0,
            0.0,
        ),
        inside(
            item(
                "s1",
                "shape",
                None,
                None,
                json!({ "shape": "rect", "text": "Basket" }),
                10.0,
                10.0,
            ),
            "fr",
        ),
        inside(
            item(
                "s2",
                "shape",
                None,
                None,
                json!({ "shape": "diamond", "text": "Paid?" }),
                10.0,
                120.0,
            ),
            "fr",
        ),
        inside(
            item(
                "n1",
                "card",
                Some("note"),
                Some("Edge cases"),
                json!({ "markdown": "# Refunds\nPartial refunds go to `orders`." }),
                300.0,
                10.0,
            ),
            "fr",
        ),
        item(
            "f1",
            "card",
            Some("feature"),
            Some("Online payments"),
            json!({
            "status": "planned", "summary": "Customers pay by **card**.", "stories": ["As a customer I pay online", ""],
            "acceptance": [{ "text": "Cards work", "done": true }, { "text": "Receipts sent", "done": false }],
            "table_ids": [orders, "tbl_gone"], "page_ids": [] }),
            0.0,
            900.0,
        ),
        item(
            "sc",
            "card",
            Some("schema"),
            Some("Data"),
            json!({ "table_ids": [t[0].id, t[1].id] }),
            600.0,
            900.0,
        ),
        item(
            "lone",
            "shape",
            None,
            None,
            json!({ "shape": "rect", "text": "Loose" }),
            2000.0,
            2000.0,
        ),
    ];
    (items, t)
}

#[test]
fn reading_order_frames_then_their_cards() {
    let (items, t) = plan();
    let d = document(&board(), &items, &t, &Options::default(), picture).unwrap();
    let doc = &d.json;
    assert_eq!(doc["schema"], 2);
    assert_eq!(doc["frontmatter"]["title"], "Florist site");
    assert_eq!(
        doc["frontmatter"]["subtitle"], "Online orders for a florist",
        "first line of the description"
    );
    assert_eq!(doc["frontmatter"]["eyebrow"], "Planning document");
    // Three sections (no contents page under four), loose shapes left out.
    assert_eq!(doc["pages"].as_array().unwrap().len(), 3);
    assert_eq!(types(doc, 0), ["opener", "image", "heading", "markdown"]);
    let frame = blocks(doc, 0);
    assert_eq!(frame[0]["intro"], "How a customer pays");
    assert_eq!(frame[1]["width"], "full");
    assert_eq!(frame[1]["alt"], "Flowchart, Checkout: Basket, then Paid?");
    assert_eq!(frame[2]["level"], 3);
    assert_eq!(
        frame[3]["content"], "#### Refunds\nPartial refunds go to `orders`.",
        "note headings sit under the card's"
    );
    assert_eq!(d.images, 2);
}

#[test]
fn features_and_schemas_map_to_blocks() {
    let (items, t) = plan();
    let doc = document(&board(), &items, &t, &Options::default(), picture)
        .unwrap()
        .json;
    let f = blocks(&doc, 1);
    assert_eq!(
        types(&doc, 1),
        [
            "opener",
            "facts",
            "markdown",
            "heading",
            "bullet_list",
            "heading",
            "checklist"
        ]
    );
    assert_eq!(
        f[1]["items"][0],
        json!({ "label": "Status", "value": "Planned" })
    );
    assert_eq!(
        f[1]["items"][1],
        json!({ "label": "Tables", "value": "orders" }),
        "trashed tables dropped"
    );
    assert_eq!(
        f[4]["items"],
        json!(["As a customer I pay online"]),
        "blank stories dropped"
    );
    assert_eq!(
        f[6]["items"][0],
        json!({ "text": "Cards work", "done": true })
    );

    let s = blocks(&doc, 2);
    assert_eq!(
        types(&doc, 2),
        [
            "opener",
            "image",
            "heading",
            "paragraph",
            "table",
            "heading",
            "table"
        ]
    );
    assert_eq!(s[1]["alt"], "Entity-relationship diagram of users, orders");
    assert_eq!(s[2]["text"], "users");
    let orders = &s[6];
    assert_eq!(
        orders["headers"],
        json!(["Column", "Type", "Required", "Key", "Notes"])
    );
    assert_eq!(
        orders["rows"][1],
        json!([
            "`user_id`",
            "`bigint`",
            "Yes",
            "FK",
            "References `users.id`"
        ])
    );
    assert_eq!(orders["rows"][2][4], "One of `new`, `sent`. Default `new`");
    assert_eq!(orders["rows"][3][2], "No", "nullable is not required");
    assert_eq!(
        orders["rows"][3][4], "Card \\*message\\*",
        "free text is escaped"
    );
    assert_eq!(s[4]["rows"][0][4], "Auto-increments");
}

#[test]
fn explicit_selections_keep_their_order() {
    let (items, t) = plan();
    let opts = Options {
        item_ids: vec!["sc".into(), "fr".into(), "sc".into()],
        ..Options::default()
    };
    let doc = document(&board(), &items, &t, &opts, picture).unwrap().json;
    assert_eq!(
        doc["pages"].as_array().unwrap().len(),
        2,
        "duplicates dropped"
    );
    assert_eq!(blocks(&doc, 0)[0]["title"], "Data");

    let bad = Options {
        item_ids: vec!["s1".into()],
        ..Options::default()
    };
    let err = document(&board(), &items, &t, &bad, picture).unwrap_err();
    assert!(
        matches!(err, AppError::Invalid { ref path, .. } if path == "item_ids[0]"),
        "{err:?}"
    );
}

#[test]
fn a_part_divider_for_combined_documents() {
    let (items, t) = plan();
    let opts = Options {
        part: Some("The plan".into()),
        title: Some("Acme proposal".into()),
        ..Options::default()
    };
    let doc = document(&board(), &items, &t, &opts, picture).unwrap().json;
    assert_eq!(
        blocks(&doc, 0),
        vec![json!({ "type": "part", "title": "The plan" })]
    );
    assert_eq!(doc["frontmatter"]["title"], "Acme proposal");
}

#[test]
fn image_limits_become_warnings() {
    let mut items = Vec::new();
    for k in 0..30 {
        let f = format!("f{k}");
        items.push(item(
            &f,
            "frame",
            None,
            Some(&format!("Flow {k}")),
            json!({}),
            0.0,
            k as f64 * 1000.0,
        ));
        items.push(inside(
            item(
                &format!("s{k}"),
                "shape",
                None,
                None,
                json!({ "shape": "rect", "text": "x" }),
                0.0,
                k as f64 * 1000.0,
            ),
            &f,
        ));
    }
    let d = document(&board(), &items, &[], &Options::default(), picture).unwrap();
    assert_eq!(d.images, MAX_IMAGES);
    assert_eq!(d.warnings.len(), 6);
    assert!(d.warnings[0].contains("24 images"));
    assert_eq!(
        types(&d.json, 0),
        ["contents"],
        "a long document gets a contents page"
    );

    let failing = document(&board(), &items[..2], &[], &Options::default(), |_| None).unwrap();
    assert!(failing.warnings[0].contains("couldn't be drawn"));
    let empty = document(&board(), &[], &[], &Options::default(), picture).unwrap();
    assert!(empty.warnings[0].contains("Nothing to export"));
}

#[test]
fn sitemaps_and_checklists() {
    let items = vec![
        item(
            "sm",
            "card",
            Some("sitemap"),
            Some("Pages"),
            json!({ "root": { "id": "pg_1", "title": "Home", "slug": "/", "children": [
            { "id": "pg_2", "title": "Shop", "slug": "/shop", "purpose": "Browse *flowers*" } ] } }),
            0.0,
            0.0,
        ),
        item(
            "cl",
            "card",
            Some("checklist"),
            Some("Launch"),
            json!({ "items": [
            { "text": "DNS", "done": true }, { "text": "Fonts", "done": false, "group": "Design" } ] }),
            500.0,
            0.0,
        ),
    ];
    let doc = document(&board(), &items, &[], &Options::default(), picture)
        .unwrap()
        .json;
    assert_eq!(
        blocks(&doc, 0)[1],
        json!({ "type": "tree", "items": [{ "text": "**Home** `/`", "children": [
            { "text": "**Shop** `/shop`: Browse \\*flowers\\*" } ] }] })
    );
    assert_eq!(
        types(&doc, 1),
        ["opener", "checklist", "heading", "checklist"]
    );
    assert_eq!(blocks(&doc, 1)[2]["text"], "Design");
}

#[test]
fn markdown_helpers() {
    assert_eq!(esc("user_id [x] *a*"), "user\\_id \\[x\\] \\*a\\*");
    assert_eq!(
        demote("# A\n## B\n```\n# not a heading\n```\n#hashtag", 3),
        "### A\n#### B\n```\n# not a heading\n```\n#hashtag"
    );
    assert_eq!(demote("###### Deep", 3), "###### Deep", "capped at six");
}
