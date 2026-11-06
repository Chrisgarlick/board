//! Phase 6: feature cards and the board model.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use board::types::{Actor, Scope};
use common::TestApp;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

async fn get(
    app: &TestApp,
    uri: &str,
    token: Option<&str>,
    etag: Option<&str>,
) -> (StatusCode, Option<String>, Value) {
    let mut b = Request::builder().uri(uri);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    if let Some(e) = etag {
        b = b.header(header::IF_NONE_MATCH, e);
    }
    let res = app
        .router
        .clone()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let tag = res
        .headers()
        .get(header::ETAG)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        tag,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A small real plan: a table, a sitemap, a feature using both, a flow.
async fn plan(app: &TestApp) -> String {
    let board = app.board().await;
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/tables"),
            Some(json!({ "name": "orders", "definition": { "columns": [
                { "name": "id", "type": "bigint", "primary": true },
                { "name": "status", "type": "enum", "values": ["pending", "paid"] },
            ] } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED);
    app.item(
        &board,
        json!({ "kind": "card", "card_type": "sitemap", "title": "Sitemap", "x": 0, "y": 0,
                "content": { "root": { "title": "Home", "slug": "/", "children": [{ "title": "Checkout", "slug": "/checkout" }] } } }),
    )
    .await;
    app.item(
        &board,
        json!({ "kind": "card", "card_type": "feature", "title": "Online payments", "x": 400, "y": 0,
                "content": { "status": "planned", "summary": "Pay by card", "stories": ["As a customer I want to pay online"],
                             "acceptance": [{ "text": "Card payments succeed" }], "tables": ["orders"], "pages": ["/checkout"] } }),
    )
    .await;
    let flow = app
        .call(
            "POST",
            &format!("/api/boards/{board}/flows"),
            Some(json!({ "mermaid": "flowchart TD\n a[Basket] --> b{Paid?}", "frame_title": "Checkout flow" })),
        )
        .await;
    assert_eq!(flow.status, StatusCode::CREATED);
    board
}

#[tokio::test]
async fn the_model_has_the_plan_and_an_etag() {
    let app = TestApp::new().await;
    let board = plan(&app).await;
    let design = app.token(Scope::Read, Actor::Api).await;

    let (status, etag, m) = get(
        &app,
        &format!("/api/boards/{board}/model"),
        Some(&design),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{m}");
    assert_eq!(m["contract"], "board-model");
    assert_eq!(m["contract_version"], 1);
    assert_eq!(m["tables"][0]["name"], "orders");
    let status_col = &m["tables"][0]["columns"][1];
    assert_eq!(status_col["label"], "Status");
    assert_eq!(status_col["hash"].as_str().unwrap().len(), 8);
    assert_eq!(m["pages"][0]["children"][0]["slug"], "/checkout");
    let f = &m["features"][0];
    assert_eq!(f["status"], "planned");
    assert_eq!(f["table_ids"][0], m["tables"][0]["id"]);
    assert_eq!(f["page_ids"][0], m["pages"][0]["children"][0]["id"]);
    assert_eq!(m["flows"][0]["title"], "Checkout flow");
    assert_eq!(m["flows"][0]["edges"].as_array().unwrap().len(), 1);

    // Unchanged: 304. Changed: a new ETag.
    let etag = etag.unwrap();
    let (status, _, _) = get(
        &app,
        &format!("/api/boards/{board}/model"),
        Some(&design),
        Some(&etag),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    let table = m["tables"][0]["id"].as_str().unwrap();
    let res = app
        .call(
            "PATCH",
            &format!("/api/tables/{table}"),
            Some(json!({ "version": 1, "definition": { "columns": [
                { "name": "id", "type": "bigint", "primary": true },
                { "name": "status", "type": "enum", "values": ["pending", "paid", "refunded"] },
            ] } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    let (status, etag2, m2) = get(
        &app,
        &format!("/api/boards/{board}/model"),
        Some(&design),
        Some(&etag),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(etag2.unwrap(), etag);
    assert_ne!(
        m2["tables"][0]["columns"][1]["hash"], status_col["hash"],
        "the changed column's hash"
    );
    assert_eq!(
        m2["tables"][0]["columns"][0]["hash"], m["tables"][0]["columns"][0]["hash"],
        "the others stay"
    );
    assert_eq!(m2["tables"][0]["columns"][1]["id"], status_col["id"]);

    // Moving a card changes nothing Design reads.
    let feature = app.full(&board).await["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["card_type"] == "feature")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, before, _) = get(
        &app,
        &format!("/api/boards/{board}/model"),
        Some(&design),
        None,
    )
    .await;
    app.call(
        "PATCH",
        &format!("/api/items/{feature}/geometry"),
        Some(json!({ "x": 420 })),
    )
    .await;
    let (status, _, _) = get(
        &app,
        &format!("/api/boards/{board}/model"),
        Some(&design),
        before.as_deref(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);

    // Without a token: refused.
    let (status, _, _) = get(&app, &format!("/api/boards/{board}/model"), None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_json_schema_is_public() {
    let app = TestApp::new().await;
    let (status, _, schema) = get(&app, "/api/model.schema.json", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let text = schema.to_string();
    assert!(
        text.contains("BoardModel") && text.contains("contract_version"),
        "{text:.200}"
    );
}

#[tokio::test]
async fn feature_cards_validate_their_references() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/items"),
            Some(json!({ "kind": "card", "card_type": "feature", "title": "x", "content": { "tables": ["nope"] } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["path"], "content.tables[0]");
    let card = app
        .item(
            &board,
            json!({ "kind": "card", "card_type": "feature", "title": "Search", "content": {} }),
        )
        .await;
    assert_eq!(card["content"]["status"], "idea");
    assert_eq!(card["w"], 380.0);
    let found = app.call("GET", "/api/search?q=search", None).await;
    assert_eq!(found.body["results"][0]["title"], "Search");
}
