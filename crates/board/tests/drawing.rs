//! Phase 2: shapes, stickies, text, frames and connectors over the REST API.

mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::{Value, json};

fn id(v: &Value) -> String {
    v["id"].as_str().expect("id").to_string()
}

async fn shape(app: &TestApp, board: &str, shape: &str, x: f64, y: f64) -> Value {
    app.item(
        board,
        json!({ "kind": "shape", "content": { "shape": shape, "text": shape }, "x": x, "y": y }),
    )
    .await
}

async fn frame(app: &TestApp, board: &str, title: &str, x: f64, y: f64, w: f64, h: f64) -> Value {
    app.item(
        board,
        json!({ "kind": "frame", "title": title, "content": {}, "x": x, "y": y, "w": w, "h": h }),
    )
    .await
}

fn find<'a>(full: &'a Value, item: &str) -> &'a Value {
    full["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == item)
        .unwrap_or_else(|| panic!("item {item} not on board"))
}

// ---------- Kinds ----------

#[tokio::test]
async fn every_kind_can_be_created_with_its_default_size() {
    let app = TestApp::new().await;
    let board = app.board().await;

    let rect = shape(&app, &board, "rect", 0.0, 0.0).await;
    assert_eq!(
        (rect["w"].as_f64(), rect["h"].as_f64()),
        (Some(160.0), Some(72.0))
    );
    assert_eq!(rect["title"], Value::Null);
    let diamond = shape(&app, &board, "diamond", 300.0, 0.0).await;
    assert_eq!(
        (diamond["w"].as_f64(), diamond["h"].as_f64()),
        (Some(150.0), Some(110.0))
    );

    let sticky = app
        .item(&board, json!({ "kind": "sticky", "content": { "text": "Guest checkout?" }, "style": { "color": "yellow" } }))
        .await;
    assert_eq!(sticky["w"], 180.0);
    assert_eq!(sticky["style"]["color"], "yellow");

    let text = app
        .item(&board, json!({ "kind": "text", "content": { "text": "Phase 1", "size": "xl" }, "style": { "align": "center" } }))
        .await;
    assert_eq!(text["content"]["size"], "xl");

    let f = frame(&app, &board, "Checkout flow", 0.0, 500.0, 800.0, 600.0).await;
    assert_eq!(f["title"], "Checkout flow");

    // Shapes don't take a title; frames need one.
    let res = app
        .call(
            "POST",
            &format!("/api/boards/{board}/items"),
            Some(json!({ "kind": "shape", "title": "x", "content": { "shape": "rect" } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["path"], "title");

    // Switching shape type keeps the item (and its connectors).
    let res = app
        .call(
            "PATCH",
            &format!("/api/items/{}", id(&rect)),
            Some(json!({ "version": 1, "content": { "shape": "diamond", "text": "Logged in?" } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["content"]["shape"], "diamond");
}

// ---------- Frames ----------

#[tokio::test]
async fn membership_follows_the_items_centre() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let f = id(&frame(&app, &board, "Flow", 0.0, 0.0, 800.0, 600.0).await);

    let inside = shape(&app, &board, "rect", 100.0, 100.0).await;
    assert_eq!(inside["frame_id"], f.as_str());
    let outside = shape(&app, &board, "rect", 1000.0, 100.0).await;
    assert_eq!(outside["frame_id"], Value::Null);

    // Half in, half out: the centre (760 + 80, …) is outside, so no frame.
    let res = app
        .batch(
            &board,
            json!([{ "op": "set_geometry", "item": id(&inside), "x": 760, "y": 100 }]),
        )
        .await;
    assert_eq!(res.body["results"][0]["frame_id"], Value::Null);

    // Dragging the outside shape in joins it.
    let res = app
        .batch(
            &board,
            json!([{ "op": "set_geometry", "item": id(&outside), "x": 300, "y": 300 }]),
        )
        .await;
    assert_eq!(res.body["results"][0]["frame_id"], f.as_str());
}

#[tokio::test]
async fn overlapping_frames_the_topmost_wins() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let lower = id(&frame(&app, &board, "Lower", 0.0, 0.0, 600.0, 600.0).await);
    let upper = id(&frame(&app, &board, "Upper", 300.0, 300.0, 600.0, 600.0).await);
    let s = shape(&app, &board, "rect", 400.0, 400.0).await;
    assert_eq!(s["frame_id"], upper.as_str());

    // Send the upper frame below the lower one: the shape changes frame.
    let res = app
        .batch(
            &board,
            json!([{ "op": "set_geometry", "item": upper, "z": -10 }]),
        )
        .await;
    let affected: Vec<&Value> = res.body["affected"].as_array().unwrap().iter().collect();
    assert_eq!(affected.len(), 1);
    assert_eq!(affected[0]["frame_id"], lower.as_str());
}

#[tokio::test]
async fn a_new_frame_captures_what_is_inside_it() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let a = id(&shape(&app, &board, "rect", 100.0, 100.0).await);
    let b = id(&shape(&app, &board, "rect", 2000.0, 100.0).await);

    let res = app
        .batch(
            &board,
            json!([{ "op": "create_item", "kind": "frame", "title": "Flow", "content": {},
                     "x": 0, "y": 0, "w": 800, "h": 600 }]),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    let f = id(&res.body["results"][0]);
    let affected = res.body["affected"].as_array().unwrap();
    assert_eq!(affected.len(), 1);
    assert_eq!(affected[0]["id"], a.as_str());
    assert_eq!(affected[0]["frame_id"], f.as_str());
    assert_eq!(find(&app.full(&board).await, &b)["frame_id"], Value::Null);
}

#[tokio::test]
async fn moving_a_frame_moves_its_members_even_locked_ones() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let f = id(&frame(&app, &board, "Flow", 0.0, 0.0, 800.0, 600.0).await);
    let a = id(&shape(&app, &board, "rect", 100.0, 100.0).await);
    let b = id(&shape(&app, &board, "ellipse", 300.0, 300.0).await);
    let outside = id(&shape(&app, &board, "rect", 2000.0, 0.0).await);
    app.batch(
        &board,
        json!([{ "op": "set_geometry", "item": b, "locked": true }]),
    )
    .await;

    let res = app
        .batch(
            &board,
            json!([{ "op": "set_geometry", "item": f, "x": 1000, "y": 500 }]),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    let affected = res.body["affected"].as_array().unwrap();
    assert_eq!(affected.len(), 2, "both members, nothing else");

    let full = app.full(&board).await;
    assert_eq!(
        (find(&full, &a)["x"].as_f64(), find(&full, &a)["y"].as_f64()),
        (Some(1100.0), Some(600.0))
    );
    assert_eq!(
        (find(&full, &b)["x"].as_f64(), find(&full, &b)["y"].as_f64()),
        (Some(1300.0), Some(800.0))
    );
    assert_eq!(find(&full, &b)["frame_id"], f.as_str());
    assert_eq!(find(&full, &outside)["x"], 2000.0);
}

#[tokio::test]
async fn deleting_a_frame_releases_or_trashes_its_members() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let f = id(&frame(&app, &board, "Flow", 0.0, 0.0, 800.0, 600.0).await);
    let a = id(&shape(&app, &board, "rect", 100.0, 100.0).await);

    // Without contents: the member stays, released.
    let res = app.call("DELETE", &format!("/api/items/{f}"), None).await;
    assert_eq!(res.body["deleted"], json!([f]));
    assert_eq!(res.body["affected"], json!([a]));
    assert_eq!(find(&app.full(&board).await, &a)["frame_id"], Value::Null);

    // Restoring the frame captures the member again.
    let res = app
        .batch(&board, json!([{ "op": "undelete_item", "item": f }]))
        .await;
    assert_eq!(res.body["affected"][0]["frame_id"], f.as_str());

    // With contents: both go to the trash.
    let res = app
        .call(
            "DELETE",
            &format!("/api/items/{f}?with_contents=true"),
            None,
        )
        .await;
    let deleted = res.body["deleted"].as_array().unwrap();
    assert_eq!(deleted.len(), 2);
    assert!(
        app.full(&board).await["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

// ---------- Connectors ----------

#[tokio::test]
async fn connectors_validate_update_and_follow_their_items() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let other = app.board().await;
    let a = id(&shape(&app, &board, "rect", 0.0, 0.0).await);
    let b = id(&shape(&app, &board, "diamond", 0.0, 200.0).await);
    let elsewhere = id(&shape(&app, &other, "rect", 0.0, 0.0).await);

    let create = |body: Value| {
        let board = board.clone();
        let app = &app;
        async move {
            app.call(
                "POST",
                &format!("/api/boards/{board}/connectors"),
                Some(body),
            )
            .await
        }
    };

    let res =
        create(json!({ "from": a, "to": b, "label": "next", "style": { "line": "dashed" } })).await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    let c = id(&res.body);
    assert_eq!(res.body["from_anchor"], "auto");
    assert_eq!(res.body["style"]["line"], "dashed");

    // Two connectors between the same pair are allowed (yes/no branches).
    assert_eq!(
        create(json!({ "from": b, "to": a, "label": "no" }))
            .await
            .status,
        StatusCode::CREATED
    );
    // Self-links, other boards and bad styles are refused.
    assert_eq!(
        create(json!({ "from": a, "to": a })).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let res = create(json!({ "from": a, "to": elsewhere })).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["path"], "to");
    let res = create(json!({ "from": a, "to": b, "style": { "end": "triangle" } })).await;
    assert_eq!(res.body["path"], "style.end");

    let res = app
        .call(
            "PATCH",
            &format!("/api/connectors/{c}"),
            Some(json!({ "from_anchor": "bottom", "to_anchor": "top", "label": null, "style": { "routing": "straight" } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    assert_eq!(res.body["from_anchor"], "bottom");
    assert_eq!(res.body["label"], Value::Null);
    assert_eq!(res.body["style"], json!({ "routing": "straight" }));

    // Deleting an item hides its connectors; restoring it brings them back.
    app.call("DELETE", &format!("/api/items/{b}"), None).await;
    assert!(
        app.full(&board).await["connectors"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    app.call("POST", &format!("/api/items/{b}/undelete"), None)
        .await;
    assert_eq!(
        app.full(&board).await["connectors"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // Deleting a connector puts it in the trash, where it can be restored.
    assert_eq!(
        app.call("DELETE", &format!("/api/connectors/{c}"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    let trash = app
        .call("GET", &format!("/api/boards/{board}/trash"), None)
        .await;
    assert_eq!(trash.body["connectors"][0]["id"], c.as_str());
    let res = app
        .call("POST", &format!("/api/connectors/{c}/undelete"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK);
}

#[tokio::test]
async fn a_whole_flow_in_one_batch() {
    let app = TestApp::new().await;
    let board = app.board().await;
    let res = app
        .batch(
            &board,
            json!([
                { "op": "create_item", "ref": "frame", "kind": "frame", "title": "Checkout",
                  "content": {}, "x": 0, "y": 0, "w": 600, "h": 500 },
                { "op": "create_item", "ref": "basket", "kind": "shape",
                  "content": { "shape": "rounded", "text": "Basket" }, "x": 40, "y": 40 },
                { "op": "create_item", "ref": "auth", "kind": "shape",
                  "content": { "shape": "diamond", "text": "Logged in?" }, "x": 45, "y": 200 },
                { "op": "create_connector", "ref": "c1", "from": "basket", "to": "auth" },
                { "op": "update_connector", "connector": "c1", "label": "next" }
            ]),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    let refs = &res.body["refs"];
    assert_eq!(res.body["results"][1]["frame_id"], refs["frame"]);
    assert_eq!(res.body["results"][4]["label"], "next");
    assert_eq!(res.body["results"][4]["from_item"], refs["basket"]);

    // A connector to a ref that doesn't exist fails the whole batch.
    let res = app
        .batch(
            &board,
            json!([
                { "op": "create_item", "ref": "x", "kind": "sticky", "content": {} },
                { "op": "create_connector", "from": "x", "to": "nope" }
            ]),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["op_index"], 1);
    assert_eq!(app.full(&board).await["items"].as_array().unwrap().len(), 3);
}

// ---------- Moving between boards ----------

#[tokio::test]
async fn moving_a_frame_to_another_board_takes_its_flow() {
    let app = TestApp::new().await;
    let source = app.board().await;
    let target = app.board().await;
    let f = id(&frame(&app, &source, "Flow", 0.0, 0.0, 800.0, 600.0).await);
    let a = id(&shape(&app, &source, "rect", 100.0, 100.0).await);
    let b = id(&shape(&app, &source, "rect", 100.0, 300.0).await);
    let outside = id(&shape(&app, &source, "rect", 2000.0, 0.0).await);
    let res = app
        .batch(
            &source,
            json!([
                { "op": "create_connector", "from": a, "to": b },
                { "op": "create_connector", "from": b, "to": outside }
            ]),
        )
        .await;
    let inner = id(&res.body["results"][0]);
    let crossing = id(&res.body["results"][1]);

    let res = app
        .call(
            "POST",
            "/api/items/move",
            Some(json!({ "item_ids": [f], "board_id": target, "dx": 50, "dy": 0 })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    assert_eq!(
        res.body["items"].as_array().unwrap().len(),
        3,
        "frame and both members"
    );
    assert_eq!(res.body["connectors"], json!([inner]));
    assert_eq!(res.body["removed_connectors"], json!([crossing]));

    let moved = app.full(&target).await;
    assert_eq!(find(&moved, &a)["frame_id"], f.as_str());
    assert_eq!(find(&moved, &a)["x"], 150.0);
    assert_eq!(moved["connectors"][0]["id"], inner.as_str());
    let left = app.full(&source).await;
    assert_eq!(left["items"].as_array().unwrap().len(), 1);
    assert!(left["connectors"].as_array().unwrap().is_empty());

    let res = app
        .call(
            "POST",
            "/api/items/move",
            Some(json!({ "item_ids": [a], "board_id": target })),
        )
        .await;
    assert_eq!(
        res.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "already on that board"
    );
}
