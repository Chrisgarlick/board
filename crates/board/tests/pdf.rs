//! Phase 7: planning PDFs through Typeset, against a fake Typeset that
//! records what Board sends and answers like the real one.

mod common;

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use common::TestApp;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const KEY: &str = "ck_test_key";

/// One request to the fake: path, Authorization header, JSON body.
type Recorded = (String, Option<String>, Value);

#[derive(Clone, Default)]
struct Seen {
    requests: Arc<Mutex<Vec<Recorded>>>,
}

/// A fake Typeset on a free port. Returns its URL and what it was sent.
async fn fake_typeset() -> (String, Seen) {
    let seen = Seen::default();
    async fn record(seen: &Seen, path: &str, headers: &HeaderMap, body: Value) -> bool {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let ok = auth.as_deref() == Some(&format!("Bearer {KEY}"));
        seen.requests
            .lock()
            .unwrap()
            .push((path.to_string(), auth, body));
        ok
    }
    let app = Router::new()
        .route(
            "/api/render",
            post(
                |State(seen): State<Seen>, headers: HeaderMap, Json(body): Json<Value>| async move {
                    if !record(&seen, "/api/render", &headers, body.clone()).await {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({ "error": "bad key", "status": 401 })),
                        )
                            .into_response();
                    }
                    if body["client"] == "nobody" {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(json!({ "error": "No client `nobody`", "status": 404 })),
                        )
                            .into_response();
                    }
                    (
                        [(header::CONTENT_TYPE, "application/pdf")],
                        b"%PDF-1.7 fake".to_vec(),
                    )
                        .into_response()
                },
            ),
        )
        .route(
            "/api/proof",
            post(
                |State(seen): State<Seen>, headers: HeaderMap, Json(body): Json<Value>| async move {
                    record(&seen, "/api/proof", &headers, body).await;
                    Json(json!({ "pages": 3, "ppi": 40, "images": ["aGk=", "aGk=", "aGk="] }))
                },
            ),
        )
        .route(
            "/api/clients",
            get(|State(seen): State<Seen>, headers: HeaderMap| async move {
                record(&seen, "/api/clients", &headers, Value::Null).await;
                Json(json!([{ "slug": "kritano", "name": "Kritano", "brand_version": 2 }]))
            }),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, seen)
}

async fn raw(
    app: &TestApp,
    method: &str,
    uri: &str,
    body: Option<Value>,
    auth: bool,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut b = Request::builder().method(method).uri(uri);
    if auth {
        b = b.header(header::AUTHORIZATION, format!("Bearer {}", app.admin));
    }
    let req = match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string())),
        None => b.body(Body::empty()),
    }
    .unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    (
        status,
        headers,
        res.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}

/// A board with a flow, a schema card and a feature.
async fn plan(app: &TestApp) -> String {
    let board = app.board().await;
    app.call(
        "PATCH",
        &format!("/api/boards/{board}"),
        Some(json!({ "client": "kritano" })),
    )
    .await;
    app.call(
        "POST",
        &format!("/api/boards/{board}/tables"),
        Some(json!({ "name": "orders", "definition": { "columns": [{ "name": "id", "type": "bigint", "primary": true }] } })),
    )
    .await;
    app.call(
        "POST",
        &format!("/api/boards/{board}/flows"),
        Some(json!({ "mermaid": "flowchart TD\n a[Basket] --> b{Paid?}", "frame_title": "Checkout" })),
    )
    .await;
    app.item(&board, json!({ "kind": "card", "card_type": "schema", "title": "Data", "content": { "tables": ["orders"] }, "x": 0, "y": 1200 })).await;
    app.item(
        &board,
        json!({ "kind": "card", "card_type": "feature", "title": "Payments", "x": 600, "y": 1200,
        "content": { "status": "planned", "acceptance": [{ "text": "Cards work" }] } }),
    )
    .await;
    board
}

#[tokio::test]
async fn a_pdf_is_rendered_through_typeset_in_the_boards_brand() {
    let (url, seen) = fake_typeset().await;
    let app =
        TestApp::with_config(&format!("typeset_url = \"{url}\"\ntypeset_key = \"{KEY}\"")).await;
    let board = plan(&app).await;

    let (status, headers, bytes) = raw(
        &app,
        "POST",
        &format!("/api/boards/{board}/pdf"),
        Some(json!({ "title": "Acme plan" })),
        true,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(headers[header::CONTENT_TYPE], "application/pdf");
    assert!(
        headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("Acme-plan.pdf")
    );
    assert_eq!(bytes, b"%PDF-1.7 fake");

    let requests = seen.requests.lock().unwrap().clone();
    let (path, auth, body) = &requests[0];
    assert_eq!(path, "/api/render");
    assert_eq!(
        auth.as_deref(),
        Some("Bearer ck_test_key"),
        "Board's own caller key"
    );
    assert_eq!(body["input_format"], "json");
    assert_eq!(body["client"], "kritano", "the board's client");
    let doc: Value = serde_json::from_str(body["content"].as_str().unwrap()).unwrap();
    assert_eq!(doc["schema"], 2);
    assert_eq!(doc["frontmatter"]["title"], "Acme plan");
    let images: Vec<&Value> = doc["pages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["blocks"].as_array().unwrap())
        .filter(|b| b["type"] == "image")
        .collect();
    assert_eq!(images.len(), 2, "the flow and the ER diagram");
    assert!(
        images.iter().all(|i| i["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/svg+xml;base64,")),
        "embedded vector images"
    );
    let svg = base64_decode(
        images[0]["url"]
            .as_str()
            .unwrap()
            .split(',')
            .nth(1)
            .unwrap(),
    );
    assert!(
        svg.starts_with("<svg") && !svg.contains("<text"),
        "text as paths, so Typeset needs no fonts"
    );
}

#[tokio::test]
async fn proofs_links_and_errors() {
    let (url, seen) = fake_typeset().await;
    let app =
        TestApp::with_config(&format!("typeset_url = \"{url}\"\ntypeset_key = \"{KEY}\"")).await;
    let board = plan(&app).await;

    let proof = app
        .call(
            "POST",
            &format!("/api/boards/{board}/pdf"),
            Some(json!({ "proof": true })),
        )
        .await;
    assert_eq!(proof.status, StatusCode::OK);
    assert_eq!(proof.body["pages"], 3);
    assert_eq!(proof.body["warnings"], json!([]));

    // A client Typeset doesn't know comes back as Typeset's message.
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/pdf"),
            Some(json!({ "client": "nobody" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "typeset_error");
    assert!(
        res.body["message"]
            .as_str()
            .unwrap()
            .contains("No client `nobody`")
    );

    // A signed link renders without a login, and only with its signature.
    let link = app
        .call(
            "POST",
            &format!("/api/boards/{board}/pdf-link"),
            Some(json!({ "title": "Shared" })),
        )
        .await;
    assert_eq!(link.status, StatusCode::OK, "{}", link.body);
    let path = link.body["url"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://localhost:7070")
        .to_string();
    assert!(path.starts_with(&format!("/p/{board}.pdf?")), "{path}");
    let (status, headers, bytes) = raw(&app, "GET", &path, None, false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("inline")
    );
    assert_eq!(bytes, b"%PDF-1.7 fake");
    let last = seen.requests.lock().unwrap().last().unwrap().2.clone();
    let doc: Value = serde_json::from_str(last["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        doc["frontmatter"]["title"], "Shared",
        "the link carries its options"
    );
    let (status, _, _) = raw(&app, "GET", &path.replace("o=", "o=x"), None, false).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The document for joining: images as signed links, small.
    let doc = app
        .call(
            "GET",
            &format!("/api/boards/{board}/typeset?part=The%20plan"),
            None,
        )
        .await;
    assert_eq!(doc.status, StatusCode::OK);
    assert_eq!(
        doc.body["document"]["pages"][0]["blocks"][0]["type"],
        "part"
    );
    let text = doc.body["document"].to_string();
    assert!(
        text.contains("http://localhost:7070/i/") && !text.contains("data:image"),
        "linked, not embedded"
    );

    let clients = app.call("GET", "/api/typeset/clients", None).await;
    assert_eq!(clients.body[0]["slug"], "kritano");

    let bad = app
        .call(
            "PATCH",
            &format!("/api/boards/{board}"),
            Some(json!({ "client": "Not A Slug" })),
        )
        .await;
    assert_eq!(bad.body["path"], "client");
    let model = app
        .call("GET", &format!("/api/boards/{board}/model"), None)
        .await;
    assert_eq!(
        model.body["board"]["client"], "kritano",
        "Design sees the client too"
    );
}

#[tokio::test]
async fn without_typeset_it_says_how_to_set_it_up() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let res = app
        .call("POST", &format!("/api/boards/{board}/pdf"), Some(json!({})))
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "typeset_not_configured");
    assert!(
        res.body["message"]
            .as_str()
            .unwrap()
            .contains("typeset_key")
    );
    // The document itself needs no Typeset.
    let doc = app
        .call("GET", &format!("/api/boards/{board}/typeset"), None)
        .await;
    assert_eq!(doc.status, StatusCode::OK);
    assert!(
        doc.body["warnings"][0]
            .as_str()
            .unwrap()
            .contains("Nothing to export")
    );
}

fn base64_decode(s: &str) -> String {
    use base64::Engine as _;
    String::from_utf8(base64::engine::general_purpose::STANDARD.decode(s).unwrap()).unwrap()
}
