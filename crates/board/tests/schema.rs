//! Phase 5: the board schema, schema cards, sitemaps and page links.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chrono::TimeDelta;
use common::TestApp;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

async fn table(app: &TestApp, board: &str, name: &str, definition: Value) -> Value {
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/tables"),
            Some(json!({ "name": name, "definition": definition })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    res.body
}

async fn users(app: &TestApp, board: &str) -> Value {
    table(
        app,
        board,
        "users",
        json!({ "columns": [
            { "name": "id", "type": "bigint", "primary": true, "generated": "increment" },
            { "name": "email", "type": "string", "length": 191, "unique": true },
        ] }),
    )
    .await["table"]
        .clone()
}

async fn schema_card(app: &TestApp, board: &str, title: &str, tables: Value) -> Value {
    app.item(
        board,
        json!({ "kind": "card", "card_type": "schema", "title": title, "content": { "tables": tables } }),
    )
    .await
}

#[tokio::test]
async fn tables_are_created_by_name_with_references_across_cards() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let users = users(&app, &board).await;
    assert!(users["id"].as_str().unwrap().starts_with("tbl_"));
    let user_pk = users["definition"]["columns"][0]["id"].as_str().unwrap();

    // `orders` lives on a second schema card and references users by name.
    let people = schema_card(&app, &board, "People", json!(["users"])).await;
    let commerce = schema_card(&app, &board, "Commerce", json!([])).await;
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/tables"),
            Some(json!({
                "name": "orders",
                "item_id": commerce["id"],
                "definition": { "columns": [
                    { "name": "id", "type": "bigint", "primary": true },
                    { "name": "user_id", "type": "integer", "references": { "table": "users", "column": "id" } },
                    { "name": "status", "type": "enum", "values": ["pending", "paid"], "default": "pending" },
                ] },
            })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    let orders = &res.body["table"];
    let fk = &orders["definition"]["columns"][1]["references"];
    assert_eq!(fk["table_id"], users["id"]);
    assert_eq!(fk["column_id"], user_pk);
    assert_eq!(fk["table"], "users", "names come back with ids");
    assert_eq!(fk["column"], "id");
    assert_eq!(res.body["schema_version"], 2);
    assert_eq!(
        res.body["warnings"][0]["path"], "definition.columns[1].references",
        "integer referencing bigint is a warning, not an error"
    );
    assert_eq!(
        res.body["item"]["content"]["table_ids"],
        json!([orders["id"]]),
        "added to the schema card"
    );
    assert_eq!(people["content"]["table_ids"], json!([users["id"]]));

    // The board carries the schema; the schema endpoint has the version.
    let full = app.full(&board).await;
    let names: Vec<&str> = full["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["orders", "users"]);
    let schema = app
        .call("GET", &format!("/api/boards/{board}/schema"), None)
        .await;
    assert_eq!(schema.body["schema_version"], 2);

    // The commerce card's ER: users is a stub, the FK is one-to-many.
    let er = app
        .call(
            "GET",
            &format!("/api/items/{}/er", commerce["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(er.status, StatusCode::OK);
    let mermaid = er.body["mermaid"].as_str().unwrap();
    assert!(
        mermaid.contains(r#"users ||--o{ orders : "user_id""#),
        "{mermaid}"
    );
    assert_eq!(er.body["stubs"][0]["name"], "users");

    let found = app.call("GET", "/api/search?q=status", None).await;
    assert_eq!(found.body["results"][0]["entity_kind"], "table");
    assert_eq!(found.body["results"][0]["title"], "orders");
}

#[tokio::test]
async fn column_ids_survive_renames_and_edits() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let t = users(&app, &board).await;
    let id = t["id"].as_str().unwrap();
    let cols = t["definition"]["columns"].as_array().unwrap().clone();
    // Past the revision coalescing window, so the first version is kept.
    app.clock.advance(TimeDelta::minutes(6));

    // Rename `email` (carrying its id), and resend `id` by name only.
    let res = app
        .call(
            "PATCH",
            &format!("/api/tables/{id}"),
            Some(json!({ "version": 1, "name": "accounts", "definition": { "columns": [
                { "name": "id", "type": "bigint", "primary": true, "generated": "increment" },
                { "id": cols[1]["id"], "name": "email_address", "type": "string", "length": 191, "unique": true },
                { "name": "name", "type": "string" },
            ] } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    let after = &res.body["table"]["definition"]["columns"];
    assert_eq!(after[0]["id"], cols[0]["id"], "matched by name");
    assert_eq!(after[1]["id"], cols[1]["id"], "renamed with its id");
    assert_eq!(after[1]["name"], "email_address");
    assert_eq!(res.body["table"]["name"], "accounts");
    assert_eq!(res.body["table"]["version"], 2);
    assert_eq!(res.body["schema_version"], 2);

    // A stale version is a conflict carrying the current table.
    let stale = app
        .call(
            "PATCH",
            &format!("/api/tables/{id}"),
            Some(json!({ "version": 1, "name": "people" })),
        )
        .await;
    assert_eq!(stale.status, StatusCode::CONFLICT);
    assert_eq!(stale.body["current"]["name"], "accounts");

    // Saving the same thing again changes nothing.
    let same = app
        .call(
            "PATCH",
            &format!("/api/tables/{id}"),
            Some(json!({ "version": 2, "name": "accounts" })),
        )
        .await;
    assert_eq!(same.body["table"]["version"], 2);
    assert_eq!(same.body["schema_version"], 2, "no bump for a no-op");

    // Restore the first revision: an ordinary update.
    let revs = app
        .call("GET", &format!("/api/tables/{id}/revisions"), None)
        .await;
    let revs = revs.body["revisions"].as_array().unwrap();
    let first = revs.last().unwrap()["id"].as_i64().unwrap();
    let restored = app
        .call(
            "POST",
            &format!("/api/tables/{id}/restore/{first}"),
            Some(json!({ "version": 2 })),
        )
        .await;
    assert_eq!(restored.status, StatusCode::OK, "{:?}", restored.body);
    assert_eq!(restored.body["table"]["name"], "users");
    assert_eq!(
        restored.body["table"]["definition"]["columns"][1]["id"],
        cols[1]["id"]
    );
}

#[tokio::test]
async fn validation_errors_name_the_field() {
    let app = TestApp::new().await;
    let board = app.board().await;
    users(&app, &board).await;
    let try_table = |name: &'static str, definition: Value| {
        let app = &app;
        let board = board.clone();
        async move {
            app.call(
                "POST",
                &format!("/api/boards/{board}/tables"),
                Some(json!({ "name": name, "definition": definition })),
            )
            .await
        }
    };
    let res = try_table("Users", json!({})).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["path"], "name");
    let res = try_table(
        "orders",
        json!({ "columns": [{ "name": "total", "type": "decimal", "precision": 4, "scale": 6 }] }),
    )
    .await;
    assert_eq!(res.body["path"], "definition.columns[0].scale");
    let res = try_table(
        "orders",
        json!({ "columns": [{ "name": "user_id", "type": "bigint", "references": { "table": "customers" } }] }),
    )
    .await;
    assert_eq!(res.body["path"], "definition.columns[0].references.table");
    assert!(res.body["message"].as_str().unwrap().contains("users"));
    let res = try_table("orders", json!({ "colums": [] })).await;
    assert_eq!(res.body["path"], "definition.colums");
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/tables"),
            Some(json!({ "name": "x", "item_id": "nope" })),
        )
        .await;
    assert_eq!(res.body["path"], "item_id");
}

#[tokio::test]
async fn referenced_tables_need_force_to_delete() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let users = users(&app, &board).await;
    let users_id = users["id"].as_str().unwrap();
    let orders = table(
        &app,
        &board,
        "orders",
        json!({ "columns": [
            { "name": "id", "type": "bigint", "primary": true },
            { "name": "user_id", "type": "bigint", "references": { "table": "users" } },
        ] }),
    )
    .await;
    let orders_id = orders["table"]["id"].as_str().unwrap();
    let card = schema_card(&app, &board, "Data", json!(["users", "orders"])).await;
    let mut rx = app.state.hub.subscribe(&board);

    // Removing the referenced column is refused too.
    let res = app
        .call(
            "PATCH",
            &format!("/api/tables/{users_id}"),
            Some(json!({ "version": 1, "definition": { "columns": [{ "name": "email", "type": "string" }] } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        res.body["message"]
            .as_str()
            .unwrap()
            .contains("orders.user_id")
    );

    let res = app
        .call("DELETE", &format!("/api/tables/{users_id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "table_referenced");
    assert!(
        res.body["message"]
            .as_str()
            .unwrap()
            .contains("orders.user_id")
    );

    let res = app
        .call("DELETE", &format!("/api/tables/{users_id}?force=1"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    assert_eq!(res.body["updated"][0]["name"], "orders");
    assert!(
        res.body["updated"][0]["definition"]["columns"][1]["references"].is_null(),
        "the reference went with it"
    );

    // One event: the delete and the side effect, with the schema version.
    let published = rx.recv().await.unwrap();
    let kinds: Vec<&str> = published.changes.iter().map(|c| c.kind).collect();
    assert_eq!(kinds, ["table.deleted", "table.updated"]);
    let v = published.changes[0].schema_version.unwrap();
    assert!(v >= 4, "{v}");

    // The card keeps the id; the trash lists the table; undelete restores it.
    let item = app
        .call(
            "GET",
            &format!("/api/items/{}", card["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(item.body["content"]["table_ids"][0], users["id"]);
    let trash = app
        .call("GET", &format!("/api/boards/{board}/trash"), None)
        .await;
    assert_eq!(trash.body["tables"][0]["name"], "users");
    assert!(app.full(&board).await["tables"].as_array().unwrap().len() == 1);

    let res = app
        .call("POST", &format!("/api/tables/{users_id}/undelete"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    assert_eq!(res.body["table"]["name"], "users");
    let _ = orders_id;
}

#[tokio::test]
async fn undelete_checks_names_and_references() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let users = users(&app, &board).await;
    let users_id = users["id"].as_str().unwrap();
    let orders = table(
        &app,
        &board,
        "orders",
        json!({ "columns": [{ "name": "user_id", "type": "bigint", "references": { "table": "users" } }] }),
    )
    .await;
    let orders_id = orders["table"]["id"].as_str().unwrap();

    // Trash orders, then users (nothing live references users any more).
    assert_eq!(
        app.call("DELETE", &format!("/api/tables/{orders_id}"), None)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        app.call("DELETE", &format!("/api/tables/{users_id}"), None)
            .await
            .status,
        StatusCode::OK
    );

    // orders can't come back before the table it references.
    let res = app
        .call("POST", &format!("/api/tables/{orders_id}/undelete"), None)
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "references_missing");
    assert!(res.body["message"].as_str().unwrap().contains("users"));

    // A new `users` takes the name; the old one can't come back until renamed.
    let newer = table(&app, &board, "users", json!({})).await;
    let res = app
        .call("POST", &format!("/api/tables/{users_id}/undelete"), None)
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "name_taken");
    app.call(
        "PATCH",
        &format!("/api/tables/{}", newer["table"]["id"].as_str().unwrap()),
        Some(json!({ "version": 1, "name": "users_v2" })),
    )
    .await;
    let res = app
        .call("POST", &format!("/api/tables/{users_id}/undelete"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    let res = app
        .call("POST", &format!("/api/tables/{orders_id}/undelete"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "now its reference resolves");
}

#[tokio::test]
async fn schema_cards_render_as_er_diagrams() {
    let app = TestApp::new().await;
    let board = app.board().await;
    users(&app, &board).await;
    table(
        &app,
        &board,
        "orders",
        json!({ "columns": [
            { "name": "id", "type": "bigint", "primary": true },
            { "name": "user_id", "type": "bigint", "references": { "table": "users" } },
        ] }),
    )
    .await;
    let card = schema_card(&app, &board, "Data model", json!(["users", "orders"])).await;
    let id = card["id"].as_str().unwrap();

    let get = |uri: String| {
        let app = &app;
        async move {
            let req = Request::builder()
                .uri(uri)
                .header(header::AUTHORIZATION, format!("Bearer {}", app.admin))
                .body(Body::empty())
                .unwrap();
            let res = app.router.clone().oneshot(req).await.unwrap();
            let status = res.status();
            (
                status,
                res.into_body().collect().await.unwrap().to_bytes().to_vec(),
            )
        }
    };
    let (status, svg) = get(format!("/api/items/{id}/render.svg?text=text")).await;
    assert_eq!(status, StatusCode::OK);
    let svg = String::from_utf8(svg).unwrap();
    assert!(svg.contains(">Data model<"), "card title as the heading");
    assert!(svg.contains(">user_id<") && svg.contains(">FK<"));

    let (status, png) = get(format!("/api/items/{id}/render.png")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&png[..4], b"\x89PNG");

    // Inside a frame, the card shows its diagram scaled into the card.
    let frame = app
        .item(
            &board,
            json!({ "kind": "frame", "title": "Data", "content": {}, "x": -100, "y": -100, "w": 1400, "h": 1000 }),
        )
        .await;
    let (status, svg) = get(format!(
        "/api/items/{}/render.svg?text=text",
        frame["id"].as_str().unwrap()
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    let svg = String::from_utf8(svg).unwrap();
    assert!(svg.contains("preserveAspectRatio"), "embedded diagram");
    assert!(svg.contains(">orders<"));
}

#[tokio::test]
async fn sitemaps_give_pages_ids_and_shapes_link_to_them() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let sitemap = app
        .item(
            &board,
            json!({ "kind": "card", "card_type": "sitemap", "title": "Sitemap", "content": { "root": {
                "title": "Home", "slug": "/", "children": [
                    { "title": "Log in", "slug": "login", "purpose": "Existing customers" },
                    { "title": "Basket", "slug": "/basket" },
                ] } } }),
        )
        .await;
    let root = &sitemap["content"]["root"];
    let login = root["children"][0].clone();
    assert!(login["id"].as_str().unwrap().starts_with("pg_"));
    assert_eq!(login["slug"], "/login");

    // A journey step linked by slug.
    let step = app
        .item(
            &board,
            json!({ "kind": "shape", "content": { "shape": "rect", "text": "Sign in", "page": "/login" } }),
        )
        .await;
    assert_eq!(step["content"]["page_id"], login["id"]);
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/items"),
            Some(json!({ "kind": "shape", "content": { "shape": "rect", "page": "/checkout" } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["path"], "content.page");

    // Edit the sitemap without ids: pages keep theirs by slug.
    let res = app
        .call(
            "PATCH",
            &format!("/api/items/{}", sitemap["id"].as_str().unwrap()),
            Some(
                json!({ "version": sitemap["version"], "content": { "root": {
                "title": "Home", "slug": "/", "children": [
                    { "title": "Basket", "slug": "/basket" },
                    { "title": "Sign in", "slug": "/login" },
                ] } } }),
            ),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    assert_eq!(
        res.body["content"]["root"]["children"][1]["id"],
        login["id"]
    );
    assert_eq!(res.body["content"]["root"]["id"], root["id"]);

    // A second sitemap can't reuse a slug.
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/items"),
            Some(
                json!({ "kind": "card", "card_type": "sitemap", "title": "Other",
                         "content": { "root": { "title": "Login", "slug": "/login" } } }),
            ),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["path"], "content.root.slug");

    // The sitemap card can't move to another board; the step can.
    let other = app.board().await;
    let res = app
        .call(
            "POST",
            "/api/items/move",
            Some(json!({ "item_ids": [sitemap["id"]], "board_id": other })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{:?}", res.body);
    assert_eq!(res.body["error"], "cannot_move");
}

#[tokio::test]
async fn schema_cards_hold_ids_for_trashed_tables() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let users = users(&app, &board).await;
    let card = schema_card(&app, &board, "Data", json!(["users"])).await;
    let id = card["id"].as_str().unwrap();
    app.call(
        "DELETE",
        &format!("/api/tables/{}", users["id"].as_str().unwrap()),
        None,
    )
    .await;

    // Editing the card's title still works; the trashed id stays.
    let res = app
        .call(
            "PATCH",
            &format!("/api/items/{id}"),
            Some(json!({ "version": card["version"], "title": "Data model", "content": card["content"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    assert_eq!(res.body["content"]["table_ids"][0], users["id"]);

    // But a trashed table can't be added fresh, and unknown names are refused.
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/items"),
            Some(json!({ "kind": "card", "card_type": "schema", "title": "x",
                         "content": { "table_ids": [users["id"]] } })),
        )
        .await;
    assert_eq!(res.body["path"], "content.table_ids[0]");
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/items"),
            Some(json!({ "kind": "card", "card_type": "schema", "title": "x",
                         "content": { "tables": ["users"] } })),
        )
        .await;
    assert_eq!(res.body["path"], "content.tables[0]");
}
