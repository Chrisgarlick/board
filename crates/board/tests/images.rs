//! Phase 4: rendered images and signed image links.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use board::types::{Actor, Scope};
use chrono::TimeDelta;
use common::TestApp;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

struct Raw {
    status: StatusCode,
    content_type: String,
    bytes: Vec<u8>,
}

async fn raw(
    app: &TestApp,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Raw {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
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
    let content_type = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    Raw {
        status,
        content_type,
        bytes,
    }
}

fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "a PNG");
    (
        u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
    )
}

/// A board with a frame holding a small flow; returns (board, frame id).
async fn flow(app: &TestApp) -> (String, String) {
    let board = app.board().await;
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/flows"),
            Some(json!({ "mermaid": "flowchart TD\n a(Basket) --> b{Logged in?}\n b -->|no| c[Log in]", "frame_title": "Checkout" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    (board, res.body["frames"][0].as_str().unwrap().to_string())
}

#[tokio::test]
async fn frames_render_as_svg_and_png() {
    let app = TestApp::new().await;
    let (_, frame) = flow(&app).await;
    let f = app
        .call("GET", &format!("/api/items/{frame}"), None)
        .await
        .body;
    let (w, h) = (f["w"].as_f64().unwrap(), f["h"].as_f64().unwrap());

    let svg = raw(
        &app,
        "GET",
        &format!("/api/items/{frame}/render.svg"),
        Some(&app.admin),
        None,
    )
    .await;
    assert_eq!(svg.status, StatusCode::OK);
    assert_eq!(svg.content_type, "image/svg+xml");
    let text = String::from_utf8(svg.bytes).unwrap();
    assert!(
        !text.contains("<text"),
        "exported SVG text is paths by default"
    );
    assert!(text.contains("<path"));

    let live = raw(
        &app,
        "GET",
        &format!("/api/items/{frame}/render.svg?text=text&title=false"),
        Some(&app.admin),
        None,
    )
    .await;
    let text = String::from_utf8(live.bytes).unwrap();
    assert!(
        text.contains(">Logged in?<"),
        "text kept as text when asked"
    );
    assert!(!text.contains(">Checkout<"), "no title when title=false");

    let png = raw(
        &app,
        "GET",
        &format!("/api/items/{frame}/render.png?scale=2"),
        Some(&app.admin),
        None,
    )
    .await;
    assert_eq!(png.content_type, "image/png");
    assert_eq!(
        png_size(&png.bytes),
        ((w * 2.0) as u32, ((h + 30.0) * 2.0) as u32),
        "frame plus title space, at 2x"
    );

    let bad = raw(
        &app,
        "GET",
        &format!("/api/items/{frame}/render.png?scale=9"),
        Some(&app.admin),
        None,
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);

    let read = app.token(Scope::Read, Actor::Api).await;
    let as_reader = raw(
        &app,
        "GET",
        &format!("/api/items/{frame}/render.png"),
        Some(&read),
        None,
    )
    .await;
    assert_eq!(
        as_reader.status,
        StatusCode::OK,
        "read tokens can fetch images"
    );
}

#[tokio::test]
async fn a_selection_renders_cropped_to_its_items() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let a = app
        .item(
            &board,
            json!({ "kind": "sticky", "content": { "text": "One" }, "x": 0, "y": 0 }),
        )
        .await;
    let b = app
        .item(
            &board,
            json!({ "kind": "sticky", "content": { "text": "Two" }, "x": 300, "y": 0 }),
        )
        .await;
    let res = raw(
        &app,
        "POST",
        &format!("/api/boards/{board}/render"),
        Some(&app.admin),
        Some(json!({ "item_ids": [a["id"], b["id"]], "format": "png", "scale": 1 })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    // Two 180-wide stickies 300 apart, plus 24 padding each side.
    assert_eq!(png_size(&res.bytes), (480 + 48, 180 + 48));

    let res = raw(
        &app,
        "POST",
        &format!("/api/boards/{board}/render"),
        Some(&app.admin),
        Some(json!({ "item_ids": ["nope"] })),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn signed_links_work_without_login_until_they_expire() {
    let app = TestApp::new().await;
    let (_, frame) = flow(&app).await;
    let res = app
        .call(
            "POST",
            &format!("/api/items/{frame}/image-url"),
            Some(json!({ "format": "png", "days": 7 })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    let url = res.body["url"].as_str().unwrap();
    assert!(url.starts_with("http://localhost:7070/i/"), "{url}");
    let path = url.trim_start_matches("http://localhost:7070");

    let ok = raw(&app, "GET", path, None, None).await;
    assert_eq!(
        ok.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&ok.bytes)
    );
    assert_eq!(ok.content_type, "image/png");

    let tampered = raw(&app, "GET", &path.replace("exp=", "exp=9"), None, None).await;
    assert_eq!(tampered.status, StatusCode::FORBIDDEN);
    let unsigned = raw(&app, "GET", &format!("/i/{frame}.png"), None, None).await;
    assert_eq!(unsigned.status, StatusCode::FORBIDDEN);

    app.clock.advance(TimeDelta::days(8));
    let expired = raw(&app, "GET", path, None, None).await;
    assert_eq!(expired.status, StatusCode::GONE);
}

#[tokio::test]
async fn a_link_to_a_deleted_item_says_so() {
    let app = TestApp::new().await;
    let (_, frame) = flow(&app).await;
    let res = app
        .call(
            "POST",
            &format!("/api/items/{frame}/image-url"),
            Some(json!({})),
        )
        .await;
    let path = res.body["url"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://localhost:7070")
        .to_string();
    app.call("DELETE", &format!("/api/items/{frame}"), None)
        .await;
    let gone = raw(&app, "GET", &path, None, None).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);

    let res = app
        .call(
            "POST",
            &format!("/api/items/{frame}/image-url"),
            Some(json!({ "days": 400 })),
        )
        .await;
    assert_eq!(
        res.status,
        StatusCode::NOT_FOUND,
        "can't link a trashed item"
    );
}
