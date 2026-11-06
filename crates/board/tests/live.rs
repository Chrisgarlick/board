//! Phase 3: live updates, flows and the MCP server.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use board::types::{Actor, Scope};
use chrono::TimeDelta;
use common::{Res, TestApp};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

// ---------- Live updates ----------

#[tokio::test]
async fn writes_publish_changes_with_their_origin() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let mut rx = app.state.hub.subscribe(&board);

    // A batch from a browser tab, tagged with its client id.
    let request = Request::post(format!("/api/boards/{board}/batch"))
        .header(header::AUTHORIZATION, format!("Bearer {}", app.admin))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-client-id", "tab-1")
        .body(Body::from(
            json!({ "ops": [
                { "op": "create_item", "ref": "a", "kind": "shape", "content": { "shape": "rect", "text": "A" }, "x": 0, "y": 0 },
                { "op": "create_item", "ref": "b", "kind": "shape", "content": { "shape": "rect", "text": "B" }, "x": 0, "y": 200 },
                { "op": "create_connector", "from": "a", "to": "b" }
            ]})
            .to_string(),
        ))
        .unwrap();
    let res = app.send(request).await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);

    let published = rx.recv().await.unwrap();
    assert_eq!(published.origin.as_deref(), Some("tab-1"));
    let kinds: Vec<&str> = published.changes.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        vec!["item.created", "item.created", "connector.created"]
    );

    // A single REST change from a script has no origin.
    let a = res.body["refs"]["a"].as_str().unwrap().to_string();
    app.call("DELETE", &format!("/api/items/{a}"), None).await;
    let published = rx.recv().await.unwrap();
    assert_eq!(published.origin, None);
    assert_eq!(published.changes[0].kind, "item.deleted");
    assert_eq!(published.changes[0].id.as_deref(), Some(a.as_str()));
}

#[tokio::test]
async fn the_event_stream_needs_a_real_board_and_login() {
    let app = TestApp::new().await;
    let res = app.call("GET", "/api/boards/nope/events", None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = app
        .send(
            Request::get("/api/boards/nope/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

// ---------- Flows ----------

const CHECKOUT: &str = "flowchart TD
  basket(Basket) --> auth{Logged in?}
  auth -->|yes| pay[Payment]
  auth -->|no| login[Log in]
  login --> pay
  pay -.-> done((Paid))";

#[tokio::test]
async fn mermaid_becomes_a_laid_out_flow_in_a_frame() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/flows"),
            Some(json!({ "mermaid": CHECKOUT, "frame_title": "Checkout" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    let frame = res.body["frames"][0].as_str().unwrap().to_string();
    let ids = &res.body["items"];

    let full = app.full(&board).await;
    let items = full["items"].as_array().unwrap();
    assert_eq!(items.len(), 6, "a frame and five steps");
    assert_eq!(full["connectors"].as_array().unwrap().len(), 5);
    let get = |r: &str| items.iter().find(|i| i["id"] == ids[r]).unwrap();
    assert_eq!(get("auth")["content"]["shape"], "diamond");
    assert_eq!(get("done")["content"]["shape"], "ellipse");
    assert!(
        get("auth")["y"].as_f64() > get("basket")["y"].as_f64(),
        "top to bottom"
    );
    for r in ["basket", "auth", "pay", "login", "done"] {
        assert_eq!(get(r)["frame_id"], frame.as_str(), "{r} is in the frame");
        assert_eq!(get(r)["created_by"], "api");
    }
    let dashed = full["connectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["from_item"] == ids["pay"])
        .unwrap();
    assert_eq!(dashed["style"]["line"], "dashed");

    // Read it back as Mermaid.
    let res = app
        .call(
            "GET",
            &format!("/api/boards/{board}/flows/{frame}/mermaid"),
            None,
        )
        .await;
    let text = res.body["mermaid"].as_str().unwrap();
    assert!(text.starts_with("flowchart TB"), "{text}");
    assert!(text.contains("{\"Logged in?\"}"), "{text}");
    assert!(text.contains("-->|yes|"), "{text}");
    assert!(text.contains("-.->"), "{text}");
}

#[tokio::test]
async fn structured_flows_are_placed_beside_a_given_item() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let anchor = app
        .item(
            &board,
            json!({ "kind": "sticky", "content": { "text": "Start here" }, "x": 0, "y": 0 }),
        )
        .await;
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/flows"),
            Some(json!({
                "direction": "LR",
                "near": anchor["id"],
                "nodes": [
                    { "ref": "a", "text": "Browse" },
                    { "ref": "b", "shape": "diamond", "text": "In stock?", "color": "orange" }
                ],
                "edges": [{ "from": "a", "to": "b", "label": "check" }]
            })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    let full = app.full(&board).await;
    let items = full["items"].as_array().unwrap();
    let a = items
        .iter()
        .find(|i| i["id"] == res.body["items"]["a"])
        .unwrap();
    let b = items
        .iter()
        .find(|i| i["id"] == res.body["items"]["b"])
        .unwrap();
    assert_eq!(a["x"], 300.0, "120 to the right of the 180-wide sticky");
    assert!(b["x"].as_f64() > a["x"].as_f64(), "left to right");
    assert_eq!(b["style"]["color"], "orange");
    assert_eq!(full["connectors"][0]["label"], "check");
}

#[tokio::test]
async fn bad_flows_say_what_is_wrong() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/flows"),
            Some(json!({ "mermaid": "flowchart TD\nA --> B\nclassDef x fill:#f00" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["path"], "mermaid");
    assert!(
        res.body["message"].as_str().unwrap().starts_with("line 3:"),
        "{:?}",
        res.body
    );

    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/flows"),
            Some(json!({ "nodes": [{ "ref": "a", "text": "A" }], "edges": [{ "from": "a", "to": "z" }] })),
        )
        .await;
    assert_eq!(res.body["path"], "edges[0].to");
    assert!(
        app.full(&board).await["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

// ---------- MCP ----------

struct Mcp<'a> {
    app: &'a TestApp,
    token: String,
    session: Option<String>,
    next_id: u64,
}

impl<'a> Mcp<'a> {
    async fn connect(app: &'a TestApp, token: String) -> Mcp<'a> {
        let mut mcp = Mcp {
            app,
            token,
            session: None,
            next_id: 1,
        };
        let (status, body, session) = mcp
            .raw(json!({
                "jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18", "capabilities": {},
                            "clientInfo": { "name": "test", "version": "1" } }
            }))
            .await;
        assert_eq!(status, StatusCode::OK, "{body:?}");
        assert_eq!(body["result"]["serverInfo"]["name"], "board");
        mcp.session = session;
        mcp.raw(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await;
        mcp
    }

    /// Send one JSON-RPC message; returns status, the JSON-RPC response and any session id.
    async fn raw(&self, message: Value) -> (StatusCode, Value, Option<String>) {
        let mut req = Request::post("/mcp")
            .header(header::HOST, "localhost")
            .header(header::AUTHORIZATION, format!("Bearer {}", self.token))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18");
        if let Some(s) = &self.session {
            req = req.header("mcp-session-id", s);
        }
        let res = self
            .app
            .router
            .clone()
            .oneshot(req.body(Body::from(message.to_string())).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let session = res
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&bytes);
        // Responses arrive as server-sent events; take the JSON-RPC message.
        let body = text
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter_map(|d| serde_json::from_str::<Value>(d).ok())
            .find(|v| v.get("jsonrpc").is_some())
            .or_else(|| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        (status, body, session)
    }

    /// Call a tool. Returns (is_error, text).
    async fn call(&mut self, name: &str, arguments: Value) -> (bool, String) {
        self.next_id += 1;
        let (_, body, _) = self
            .raw(
                json!({ "jsonrpc": "2.0", "id": self.next_id, "method": "tools/call",
                         "params": { "name": name, "arguments": arguments } }),
            )
            .await;
        let result = &body["result"];
        assert!(result.is_object(), "no result for {name}: {body}");
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();
        (result["isError"] == true, text)
    }

    async fn ok(&mut self, name: &str, arguments: Value) -> Value {
        let (is_error, text) = self.call(name, arguments).await;
        assert!(!is_error, "{name} failed: {text}");
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    }
}

#[tokio::test]
async fn claude_draws_a_flow_and_it_is_recorded_as_claude() {
    let app = TestApp::new().await;
    let claude = app.token(Scope::Admin, Actor::Claude).await;
    let mut mcp = Mcp::connect(&app, claude).await;

    let board = mcp
        .ok(
            "create_board",
            json!({ "title": "Bakery site", "stack": "Laravel" }),
        )
        .await;
    let board_id = board["id"].as_str().unwrap().to_string();
    let mut rx = app.state.hub.subscribe(&board_id);

    let flow = mcp
        .ok(
            "draw_flow",
            json!({ "board_id": board_id, "mermaid": CHECKOUT, "frame_title": "Checkout" }),
        )
        .await;
    assert_eq!(flow["items"].as_object().unwrap().len(), 5);

    // The open board hears about it as one batch, with no origin (not a tab).
    let published = rx.recv().await.unwrap();
    assert_eq!(published.origin, None);
    assert_eq!(
        published.changes.len(),
        11,
        "1 frame + 5 shapes + 5 connectors"
    );

    let listed = mcp
        .ok(
            "get_board",
            json!({ "board_id": board_id, "kinds": ["shape"] }),
        )
        .await;
    let shapes = listed["items"].as_array().unwrap();
    assert_eq!(shapes.len(), 5);
    assert!(shapes.iter().all(|s| s["updated_by"] == "claude"));

    let frame = flow["frames"][0].as_str().unwrap();
    let text = mcp.ok("get_flow", json!({ "frame_id": frame })).await;
    assert!(text.as_str().unwrap().contains("Logged in?"));

    let found = mcp.ok("search", json!({ "query": "payment" })).await;
    assert_eq!(found[0]["board_title"], "Bakery site");
}

#[tokio::test]
async fn tool_errors_carry_the_api_error_so_claude_can_fix_them() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let claude = app.token(Scope::Admin, Actor::Claude).await;
    let mut mcp = Mcp::connect(&app, claude).await;

    let (is_error, text) = mcp
        .call(
            "add_items",
            json!({ "board_id": board, "items": [{ "kind": "shape", "title": "Oops", "content": { "shape": "rect" } }] }),
        )
        .await;
    assert!(is_error);
    let err: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(err["path"], "title");
    assert_eq!(err["op_index"], 0);

    let (is_error, text) = mcp
        .call(
            "draw_flow",
            json!({ "board_id": board, "mermaid": "A --> B\nstyle A fill:red" }),
        )
        .await;
    assert!(is_error);
    assert!(text.contains("line 2"), "{text}");
}

#[tokio::test]
async fn claude_can_add_items_connect_and_update_them() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let claude = app.token(Scope::Admin, Actor::Claude).await;
    let mut mcp = Mcp::connect(&app, claude).await;

    let made = mcp
        .ok(
            "add_items",
            json!({
                "board_id": board,
                "items": [
                    { "ref": "note", "kind": "card", "card_type": "note", "title": "Goals", "content": { "markdown": "- fast" } },
                    { "ref": "idea", "kind": "sticky", "content": { "text": "Loyalty scheme?" }, "style": { "color": "pink" } }
                ],
                "connectors": [{ "from": "idea", "to": "note", "label": "feeds" }]
            }),
        )
        .await;
    let note = made["refs"]["note"].as_str().unwrap().to_string();
    assert_eq!(made["connectors"].as_array().unwrap().len(), 1);

    let updated = mcp
        .ok("update_item", json!({ "item_id": note, "version": 1, "content": { "markdown": "- fast\n- accessible" } }))
        .await;
    assert_eq!(updated["version"], 2);
    assert_eq!(updated["updated_by"], "claude");

    // A stale version is a conflict that includes the current item.
    let (is_error, text) = mcp
        .call(
            "update_item",
            json!({ "item_id": note, "version": 1, "title": "Stale" }),
        )
        .await;
    assert!(is_error);
    let err: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(err["error"], "conflict");
    assert_eq!(err["current"]["version"], 2);

    let revisions = mcp.ok("list_revisions", json!({ "item_id": note })).await;
    assert_eq!(revisions[0]["changed_by"], "claude");
}

#[tokio::test]
async fn claude_cannot_delete_what_the_user_just_edited_without_force() {
    let app = TestApp::new().await;
    let board = app.board().await;
    // A note "the user" edited (via a session-less user actor: use the batch API as a user is
    // awkward here, so mark it directly).
    let note = app
        .item(&board, json!({ "kind": "card", "card_type": "note", "title": "Mine", "content": { "markdown": "x" } }))
        .await;
    let id = note["id"].as_str().unwrap().to_string();
    sqlx::query("UPDATE items SET updated_by = 'user' WHERE id = ?")
        .bind(&id)
        .execute(&app.state.pool)
        .await
        .unwrap();

    let claude = app.token(Scope::Admin, Actor::Claude).await;
    let mut mcp = Mcp::connect(&app, claude).await;
    let (is_error, text) = mcp.call("delete_items", json!({ "item_ids": [id] })).await;
    assert!(is_error);
    assert!(text.contains("recently_edited"), "{text}");

    // After five minutes, or with force, it's allowed.
    app.clock.advance(TimeDelta::minutes(6));
    let (is_error, _) = mcp.call("delete_items", json!({ "item_ids": [id] })).await;
    assert!(!is_error);
}

#[tokio::test]
async fn mcp_needs_an_admin_token() {
    let app = TestApp::new().await;
    let read = app.token(Scope::Read, Actor::Api).await;
    let mcp = Mcp {
        app: &app,
        token: read,
        session: None,
        next_id: 1,
    };
    let (status, _, _) = mcp
        .raw(json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize",
                     "params": { "protocolVersion": "2025-06-18", "capabilities": {},
                                 "clientInfo": { "name": "t", "version": "1" } } }))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let res: Res = app
        .send(
            Request::post("/mcp")
                .header(header::HOST, "localhost")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}
