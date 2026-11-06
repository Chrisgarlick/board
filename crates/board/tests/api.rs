//! End-to-end tests of the REST API against a real (temporary) database.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use board::auth;
use board::clock::Clock;
use board::service::housekeeping;
use board::types::{Actor, Scope};
use chrono::TimeDelta;
use common::{ORIGIN, TestApp};
use serde_json::json;

// ---------- Basics ----------

#[tokio::test]
async fn healthz_and_unknown_routes() {
    let app = TestApp::new().await;
    let res = app
        .send(Request::get("/healthz").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["status"], "ok");

    let res = app.call("GET", "/api/nope", None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["error"], "not_found");

    let res = app
        .send(Request::get("/missing.js").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

// ---------- Auth ----------

#[tokio::test]
async fn api_needs_credentials() {
    let app = TestApp::new().await;
    let res = app
        .send(Request::get("/api/boards").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body["error"], "unauthorized");

    let res = app.call_as("brd_wrong", "GET", "/api/boards", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn read_tokens_can_only_read() {
    let app = TestApp::new().await;
    let read = app.token(Scope::Read, Actor::Api).await;
    assert_eq!(
        app.call_as(&read, "GET", "/api/boards", None).await.status,
        StatusCode::OK
    );
    let res = app
        .call_as(&read, "POST", "/api/boards", Some(json!({ "title": "x" })))
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn login_link_sets_a_session_that_needs_a_matching_origin_to_write() {
    let app = TestApp::new().await;
    let code = auth::create_login_code(&app.state.pool, app.clock.now())
        .await
        .unwrap();

    let res = app
        .send(
            Request::get(format!("/login/{code}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    let set_cookie = res.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Strict"));
    let cookie = set_cookie.split(';').next().unwrap().to_string();

    // The link only works once.
    let again = app
        .send(
            Request::get(format!("/login/{code}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(again.status, StatusCode::UNAUTHORIZED);

    let with_cookie = |method: &str, origin: Option<&str>| {
        let mut b = Request::builder()
            .method(method)
            .uri(if method == "GET" {
                "/api/me"
            } else {
                "/api/boards"
            })
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(o) = origin {
            b = b.header(header::ORIGIN, o);
        }
        b.body(Body::from(json!({ "title": "From the UI" }).to_string()))
            .unwrap()
    };

    let me = app.send(with_cookie("GET", None)).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["actor"], "user");

    assert_eq!(
        app.send(with_cookie("POST", None)).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(with_cookie("POST", Some("https://evil.example")))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(with_cookie("POST", Some(ORIGIN))).await.status,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn expired_login_links_are_refused() {
    let app = TestApp::new().await;
    let code = auth::create_login_code(&app.state.pool, app.clock.now())
        .await
        .unwrap();
    app.clock.advance(TimeDelta::minutes(11));
    let res = app
        .send(
            Request::get(format!("/login/{code}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

// ---------- Boards ----------

#[tokio::test]
async fn boards_crud_search_and_archive() {
    let app = TestApp::new().await;
    let id = app.board().await;
    app.call(
        "POST",
        "/api/boards",
        Some(json!({ "title": "Bakery site" })),
    )
    .await;

    let res = app.call("GET", "/api/boards?q=acme", None).await;
    let boards = res.body["boards"].as_array().unwrap();
    assert_eq!(boards.len(), 1);
    assert_eq!(boards[0]["title"], "Acme rebuild");
    assert_eq!(boards[0]["item_count"], 0);

    let res = app
        .call(
            "PATCH",
            &format!("/api/boards/{id}"),
            Some(json!({ "archived": true, "stack": "Laravel + MySQL" })),
        )
        .await;
    assert_eq!(res.body["archived"], true);
    assert_eq!(res.body["stack"], "Laravel + MySQL");
    assert_eq!(
        app.call("GET", "/api/boards", None).await.body["boards"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "archived boards are hidden from the default list"
    );
    assert_eq!(
        app.call("GET", "/api/boards?archived=true", None)
            .await
            .body["boards"][0]["id"],
        id.as_str()
    );

    let res = app
        .call("POST", "/api/boards", Some(json!({ "title": "   " })))
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);

    app.note(&id, "Kickoff", "hello").await;
    assert_eq!(
        app.call("DELETE", &format!("/api/boards/{id}"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.call("GET", &format!("/api/boards/{id}"), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert!(
        app.call("GET", "/api/search?q=kickoff", None).await.body["results"]
            .as_array()
            .unwrap()
            .is_empty(),
        "a deleted board's items leave the search index"
    );
}

// ---------- Items ----------

#[tokio::test]
async fn create_validates_and_auto_places() {
    let app = TestApp::new().await;
    let board = app.board().await;

    let a = app.note(&board, "A", "first").await;
    let b = app.note(&board, "B", "second").await;
    assert_eq!((a["x"].as_f64(), a["y"].as_f64()), (Some(0.0), Some(0.0)));
    assert_eq!((b["x"].as_f64(), b["y"].as_f64()), (Some(360.0), Some(0.0)));
    assert_eq!(a["version"], 1);
    assert_eq!(a["created_by"], "api");
    assert!(b["z"].as_i64() > a["z"].as_i64());

    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/items"),
            Some(json!({
                "kind": "card", "card_type": "checklist", "title": "Launch",
                "content": { "items": [{ "text": "DNS", "dnoe": true }] }
            })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["error"], "invalid_content");
    assert_eq!(res.body["path"], "content.items[0].dnoe");

    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/items"),
            Some(json!({ "kind": "card", "card_type": "note", "title": "x", "content": { "markdown": "" }, "x": 5 })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["path"], "y");

    let full = app.call("GET", &format!("/api/boards/{board}"), None).await;
    assert_eq!(full.body["items"].as_array().unwrap().len(), 2);
    assert_eq!(full.body["connectors"], json!([]));
}

#[tokio::test]
async fn stale_version_conflicts_with_the_current_item() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let item = app.note(&board, "Notes", "v1").await;
    let id = item["id"].as_str().unwrap();

    let res = app
        .call(
            "PATCH",
            &format!("/api/items/{id}"),
            Some(json!({ "version": 1, "content": { "markdown": "v2" } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["version"], 2);

    let res = app
        .call(
            "PATCH",
            &format!("/api/items/{id}"),
            Some(json!({ "version": 1, "content": { "markdown": "stale" } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "conflict");
    assert_eq!(res.body["current"]["version"], 2);
    assert_eq!(res.body["current"]["content"]["markdown"], "v2");

    // A no-op update doesn't bump the version.
    let res = app
        .call(
            "PATCH",
            &format!("/api/items/{id}"),
            Some(json!({ "version": 2, "content": { "markdown": "v2" } })),
        )
        .await;
    assert_eq!(res.body["version"], 2);
}

#[tokio::test]
async fn geometry_is_last_write_wins_and_respects_locks() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let id = app.note(&board, "N", "").await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let res = app
        .call(
            "PATCH",
            &format!("/api/items/{id}/geometry"),
            Some(json!({ "x": 100, "y": 50, "w": 400 })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["x"], 100.0);
    assert_eq!(res.body["w"], 400.0);
    assert_eq!(res.body["version"], 1, "geometry doesn't bump the version");

    app.call(
        "PATCH",
        &format!("/api/items/{id}/geometry"),
        Some(json!({ "locked": true })),
    )
    .await;
    let res = app
        .call(
            "PATCH",
            &format!("/api/items/{id}/geometry"),
            Some(json!({ "x": 0 })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "locked");

    let res = app
        .call(
            "PATCH",
            &format!("/api/items/{id}/geometry"),
            Some(json!({ "x": 0, "locked": false })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "unlock and move in one request");
}

#[tokio::test]
async fn soft_delete_hides_and_undelete_restores() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let id = app.note(&board, "Doomed", "pricing table").await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let res = app.call("DELETE", &format!("/api/items/{id}"), None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["deleted"], json!([id]));
    assert_eq!(
        app.call("GET", &format!("/api/items/{id}"), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let full = app.call("GET", &format!("/api/boards/{board}"), None).await;
    assert!(full.body["items"].as_array().unwrap().is_empty());
    let trash = app
        .call("GET", &format!("/api/boards/{board}/trash"), None)
        .await;
    assert_eq!(trash.body["items"][0]["id"], id.as_str());
    assert_eq!(trash.body["items"][0]["deleted_by"], "api");
    assert!(
        app.call("GET", "/api/search?q=pricing", None).await.body["results"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let res = app
        .call("POST", &format!("/api/items/{id}/undelete"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.body.get("deleted_at").is_none());
    assert_eq!(
        app.call("GET", "/api/search?q=pricing", None).await.body["results"][0]["entity_id"],
        id.as_str()
    );
}

#[tokio::test]
async fn trash_is_emptied_after_thirty_days() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let id = app.note(&board, "Old", "").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    app.call("DELETE", &format!("/api/items/{id}"), None).await;

    app.clock.advance(TimeDelta::days(29));
    housekeeping::run(&app.state.pool, app.clock.as_ref())
        .await
        .unwrap();
    let trash = app
        .call("GET", &format!("/api/boards/{board}/trash"), None)
        .await;
    assert_eq!(trash.body["items"].as_array().unwrap().len(), 1);

    app.clock.advance(TimeDelta::days(2));
    housekeeping::run(&app.state.pool, app.clock.as_ref())
        .await
        .unwrap();
    let trash = app
        .call("GET", &format!("/api/boards/{board}/trash"), None)
        .await;
    assert!(trash.body["items"].as_array().unwrap().is_empty());
}

// ---------- Revisions ----------

#[tokio::test]
async fn revisions_coalesce_per_actor_within_five_minutes() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let id = app.note(&board, "N", "a").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let edit =
        |version: i64, text: &str| json!({ "version": version, "content": { "markdown": text } });

    // Creation plus two quick edits by the same actor: one revision.
    app.call("PATCH", &format!("/api/items/{id}"), Some(edit(1, "ab")))
        .await;
    app.clock.advance(TimeDelta::minutes(2));
    app.call("PATCH", &format!("/api/items/{id}"), Some(edit(2, "abc")))
        .await;
    assert_eq!(app.revision_count(&id).await, 1);

    // Same actor after a 6-minute pause: a new revision.
    app.clock.advance(TimeDelta::minutes(6));
    app.call("PATCH", &format!("/api/items/{id}"), Some(edit(3, "abcd")))
        .await;
    assert_eq!(app.revision_count(&id).await, 2);

    // A different actor always starts a new revision, even straight away.
    let claude = app.token(Scope::Admin, Actor::Claude).await;
    let res = app
        .call_as(
            &claude,
            "PATCH",
            &format!("/api/items/{id}"),
            Some(edit(4, "claude was here")),
        )
        .await;
    assert_eq!(res.body["updated_by"], "claude");
    let revs = app
        .call("GET", &format!("/api/items/{id}/revisions"), None)
        .await;
    assert_eq!(revs.body["revisions"].as_array().unwrap().len(), 3);
    assert_eq!(revs.body["revisions"][0]["changed_by"], "claude");

    // Undo Claude's edit by restoring the revision before it.
    let before_claude = revs.body["revisions"][1]["id"].as_i64().unwrap();
    let res = app
        .call(
            "POST",
            &format!("/api/items/{id}/restore/{before_claude}"),
            Some(json!({ "version": 5 })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["content"]["markdown"], "abcd");
    assert_eq!(res.body["version"], 6);
}

#[tokio::test]
async fn revisions_are_pruned_to_twenty() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let id = app.note(&board, "N", "0").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    for v in 1..=25 {
        app.clock.advance(TimeDelta::minutes(6));
        app.call(
            "PATCH",
            &format!("/api/items/{id}"),
            Some(json!({ "version": v, "content": { "markdown": v.to_string() } })),
        )
        .await;
    }
    assert_eq!(app.revision_count(&id).await, 20);
}

// ---------- Batch ----------

#[tokio::test]
async fn batch_uses_refs_and_is_all_or_nothing() {
    let app = TestApp::new().await;
    let board = app.board().await;

    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/batch"),
            Some(json!({ "ops": [
                { "op": "create_item", "ref": "a", "kind": "card", "card_type": "note",
                  "title": "A", "content": { "markdown": "" } },
                { "op": "create_item", "ref": "b", "kind": "card", "card_type": "checklist",
                  "title": "B", "content": { "items": [] } },
                { "op": "set_geometry", "item": "a", "x": 1000, "y": 1000 },
                { "op": "update_item", "item": "b", "version": 1,
                  "content": { "items": [{ "text": "one" }] } }
            ]})),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    assert_eq!(res.body["results"][2]["x"], 1000.0);
    assert_eq!(res.body["results"][3]["version"], 2);
    let a = res.body["refs"]["a"].as_str().unwrap().to_string();

    // The third op fails, so the first two must not apply either.
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/batch"),
            Some(json!({ "ops": [
                { "op": "create_item", "kind": "card", "card_type": "note",
                  "title": "C", "content": { "markdown": "" } },
                { "op": "delete_item", "item": a },
                { "op": "update_item", "item": a, "version": 1, "title": "too late" }
            ]})),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["op_index"], 2);

    let full = app.call("GET", &format!("/api/boards/{board}"), None).await;
    let items = full.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "nothing from the failed batch was applied");
    assert!(items.iter().any(|i| i["id"] == a.as_str()));
}

#[tokio::test]
async fn batch_rejects_items_from_other_boards() {
    let app = TestApp::new().await;
    let one = app.board().await;
    let two = app.board().await;
    let id = app.note(&one, "N", "").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{two}/batch"),
            Some(json!({ "ops": [{ "op": "delete_item", "item": id }] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

// ---------- Search and limits ----------

#[tokio::test]
async fn search_matches_prefixes_and_scopes_to_a_board() {
    let app = TestApp::new().await;
    let one = app.board().await;
    let two = app.board().await;
    app.note(&one, "Payments", "Stripe checkout and refunds")
        .await;
    app.note(&two, "Hosting", "checkout page on a CDN").await;

    let all = app.call("GET", "/api/search?q=check", None).await;
    assert_eq!(all.body["results"].as_array().unwrap().len(), 2);

    let scoped = app
        .call(
            "GET",
            &format!("/api/search?q=checkout&board_id={one}"),
            None,
        )
        .await;
    let results = scoped.body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["title"], "Payments");
    assert_eq!(results[0]["board_title"], "Acme rebuild");

    // FTS syntax in the query is taken literally, not as operators.
    let res = app.call("GET", "/api/search?q=%22OR(%20NEAR", None).await;
    assert_eq!(res.status, StatusCode::OK);
}

#[tokio::test]
async fn oversized_bodies_are_rejected() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let big = "x".repeat(1_100_000);
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/items"),
            Some(json!({ "kind": "card", "card_type": "note", "title": "big", "content": { "markdown": big } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(res.body["error"], "payload_too_large");
}
