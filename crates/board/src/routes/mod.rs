//! REST handlers (spec section 6). Each one opens a transaction, calls the
//! service layer, commits, and publishes what changed to live subscribers.
//!
//! Item and connector writes all go through the batch machinery, even for a
//! single change, so REST, the UI's batches and MCP produce identical events.

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router, middleware};
use serde::Deserialize;
use serde_json::{Value, json};

pub mod images;
pub mod passkeys;
pub mod pdf;
pub mod schema;

use crate::app::AppState;
use crate::auth;
use crate::db;
use crate::error::{ApiJson, AppError, AppResult};
use crate::events::Change;
use crate::service::batch::{self, Batch, BatchResult, Op};
use crate::service::boards::{self, BoardPatch, NewBoard};
use crate::service::connectors::{self, ConnectorPatch, NewConnector};
use crate::service::flows::{self, FlowRequest};
use crate::service::items::{self, Geometry, ItemPatch, NewItem};
use crate::service::moves::{self, MoveRequest};
use crate::service::{search, tables};
use crate::types::{Caller, Ctx};

const DEFAULT_BODY_LIMIT: usize = 1024 * 1024;
const BATCH_BODY_LIMIT: usize = 2 * 1024 * 1024;
const MAX_CLIENT_ID: usize = 64;

/// Routes under `/api`, all behind authentication.
pub fn api(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/me", get(me))
        .route("/logout", post(logout))
        .route("/boards", get(list_boards).post(create_board))
        .route(
            "/boards/{id}",
            get(get_board).patch(update_board).delete(delete_board),
        )
        .route("/boards/{id}/events", get(events))
        .route("/boards/{id}/items", post(create_item))
        .route(
            "/boards/{id}/batch",
            post(apply_batch).layer(DefaultBodyLimit::max(BATCH_BODY_LIMIT)),
        )
        .route("/boards/{id}/flows", post(create_flow))
        .route("/boards/{id}/flows/{frame_id}/mermaid", get(flow_mermaid))
        .route("/boards/{id}/trash", get(trash))
        .route(
            "/items/{id}",
            get(get_item).patch(update_item).delete(delete_item),
        )
        .route("/items/move", post(move_items))
        .route("/items/{id}/geometry", patch(set_geometry))
        .route("/items/{id}/undelete", post(undelete_item))
        .route("/items/{id}/revisions", get(revisions))
        .route("/items/{id}/restore/{rev}", post(restore_revision))
        .route("/items/{id}/render.svg", get(images::item_svg))
        .route("/items/{id}/render.png", get(images::item_png))
        .route("/items/{id}/image-url", post(images::create_link))
        .route("/boards/{id}/render", post(images::selection))
        .route("/boards/{id}/connectors", post(create_connector))
        .route(
            "/connectors/{id}",
            patch(update_connector).delete(delete_connector),
        )
        .route("/connectors/{id}/undelete", post(undelete_connector))
        .route("/boards/{id}/schema", get(schema::get_schema))
        .route("/boards/{id}/tables", post(schema::create_table))
        .route(
            "/tables/{id}",
            get(schema::get_table)
                .patch(schema::update_table)
                .delete(schema::delete_table),
        )
        .route("/tables/{id}/undelete", post(schema::undelete_table))
        .route("/tables/{id}/revisions", get(schema::table_revisions))
        .route("/tables/{id}/restore/{rev}", post(schema::restore_table))
        .route("/items/{id}/er", get(schema::item_er))
        .route("/boards/{id}/model", get(get_model))
        .route("/boards/{id}/pdf", post(pdf::render_pdf))
        .route("/boards/{id}/pdf-link", post(pdf::create_link))
        .route("/boards/{id}/typeset", get(pdf::document))
        .route("/typeset/clients", get(pdf::clients))
        .route("/passkeys", get(passkeys::list).post(passkeys::register))
        .route("/passkeys/options", post(passkeys::creation_options))
        .route("/passkeys/{id}", axum::routing::delete(passkeys::remove))
        .route("/search", get(search_items))
        .fallback(api_not_found)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(middleware::from_fn_with_state(state, auth::require_caller))
}

/// Routes outside `/api` that don't need a login.
pub fn public() -> Router<AppState> {
    Router::new()
        .route("/login/{code}", get(login))
        .route("/i/{file}", get(images::public_image))
        .route("/api/model.schema.json", get(model_schema))
        .route("/p/{file}", get(pdf::public_pdf))
        .route("/auth/passkey/options", post(passkeys::request_options))
        .route("/auth/passkey/login", post(passkeys::login))
}

pub fn ctx(state: &AppState, caller: Caller) -> Ctx {
    Ctx {
        actor: caller.actor,
        now: state.clock.now(),
    }
}

/// The caller's `X-Client-Id`, used to tag live updates with who caused them.
pub fn origin(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-client-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= MAX_CLIENT_ID)
        .map(str::to_string)
}

async fn api_not_found() -> AppError {
    AppError::NotFound("route")
}

/// Run operations on a board in one transaction and publish their changes.
pub async fn run_batch(
    state: &AppState,
    caller: Caller,
    origin: Option<String>,
    board_id: &str,
    ops: Vec<Op>,
) -> AppResult<BatchResult> {
    let mut tx = db::write_tx(&state.pool).await?;
    let mut result = batch::apply(&mut tx, ctx(state, caller), board_id, Batch { ops }).await?;
    tx.commit().await?;
    state
        .hub
        .publish(board_id, origin, std::mem::take(&mut result.changes));
    Ok(result)
}

/// The single result of a one-op batch.
fn only(mut result: BatchResult) -> Value {
    result.results.pop().unwrap_or(Value::Null)
}

async fn board_of_item(state: &AppState, id: &str) -> AppResult<String> {
    let mut conn = state.pool.acquire().await?;
    items::board_of(&mut conn, id).await
}

async fn board_of_connector(state: &AppState, id: &str) -> AppResult<String> {
    let mut conn = state.pool.acquire().await?;
    connectors::board_of(&mut conn, id).await
}

// ---------- Auth ----------

async fn me(Extension(caller): Extension<Caller>) -> Json<Value> {
    Json(json!({ "actor": caller.actor, "scope": caller.scope }))
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    if let Some(session) = auth::session_from_headers(&headers) {
        auth::end_session(&state.pool, session).await?;
    }
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, auth::clear_session_cookie())],
    )
        .into_response())
}

/// Redeem a one-time login link. Responds with a small page that moves on to
/// the app, rather than a redirect, so the `SameSite=Strict` cookie is sent
/// even when the link was opened from another site (chat, email).
async fn login(State(state): State<AppState>, Path(code): Path<String>) -> AppResult<Response> {
    let now = state.clock.now();
    match auth::redeem_login_code(&state.pool, &code, now).await? {
        Some(session) => Ok((
            [
                (
                    header::SET_COOKIE,
                    auth::session_cookie(&session, &state.config.public_origin),
                ),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ],
            Html(
                "<!doctype html><meta charset=utf-8><meta http-equiv=refresh content=\"0;url=/\">\
                 <title>Board</title><p>Logged in. <a href=\"/\">Continue to Board</a>.",
            ),
        )
            .into_response()),
        None => Ok((
            StatusCode::UNAUTHORIZED,
            Html(
                "<!doctype html><meta charset=utf-8><title>Board</title>\
                 <p>This login link has expired or has already been used. \
                 Run <code>board login</code> for a new one.",
            ),
        )
            .into_response()),
    }
}

// ---------- Boards ----------

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    archived: bool,
}

async fn list_boards(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    let boards = boards::list(&mut conn, query.q.as_deref(), query.archived).await?;
    Ok(Json(json!({ "boards": boards })))
}

async fn create_board(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    ApiJson(body): ApiJson<NewBoard>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let mut tx = db::write_tx(&state.pool).await?;
    let board = boards::create(&mut tx, ctx(&state, caller), body).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!(board))))
}

async fn get_board(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    Ok(Json(json!(boards::get_full(&mut conn, &id).await?)))
}

async fn update_board(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<BoardPatch>,
) -> AppResult<Json<Value>> {
    let mut tx = db::write_tx(&state.pool).await?;
    let board = boards::update(&mut tx, ctx(&state, caller), &id, body).await?;
    tx.commit().await?;
    let value = json!(board);
    state.hub.publish(
        &id,
        origin(&headers),
        vec![Change::board("board.updated", value.clone())],
    );
    Ok(Json(value))
}

async fn delete_board(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let mut tx = db::write_tx(&state.pool).await?;
    boards::delete(&mut tx, &id).await?;
    tx.commit().await?;
    state.hub.publish(
        &id,
        origin(&headers),
        vec![Change::removed("board.deleted", id.clone())],
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn events(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Response> {
    let mut conn = state.pool.acquire().await?;
    boards::require_exists(&mut conn, &id).await?;
    Ok(state.hub.stream(&id).into_response())
}

async fn trash(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    boards::require_exists(&mut conn, &id).await?;
    let items = items::list_trash(&mut conn, &id).await?;
    let connectors = connectors::list_trash(&mut conn, &id).await?;
    let tables = tables::list_trash(&mut conn, &id).await?;
    Ok(Json(
        json!({ "items": items, "connectors": connectors, "tables": tables }),
    ))
}

async fn apply_batch(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Batch>,
) -> AppResult<Json<Value>> {
    let result = run_batch(&state, caller, origin(&headers), &id, body.ops).await?;
    Ok(Json(json!(result)))
}

// ---------- Flows ----------

async fn create_flow(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<FlowRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let mut tx = db::write_tx(&state.pool).await?;
    let (created, changes) = flows::create(&mut tx, ctx(&state, caller), &id, body)
        .await
        .map_err(|e| match e {
            AppError::InBatch { source, .. } => *source,
            other => other,
        })?;
    tx.commit().await?;
    state.hub.publish(&id, origin(&headers), changes);
    Ok((StatusCode::CREATED, Json(json!(created))))
}

async fn flow_mermaid(
    State(state): State<AppState>,
    Path((id, frame_id)): Path<(String, String)>,
) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    let mermaid = flows::frame_to_mermaid(&mut conn, &id, &frame_id).await?;
    Ok(Json(json!({ "mermaid": mermaid })))
}

// ---------- Items ----------

async fn create_item(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(board_id): Path<String>,
    ApiJson(body): ApiJson<NewItem>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let op = Op::CreateItem {
        reference: None,
        kind: body.kind,
        card_type: body.card_type,
        title: body.title,
        content: body.content,
        style: body.style,
        x: body.x,
        y: body.y,
        w: body.w,
        h: body.h,
        z: body.z,
    };
    let result = run_batch(&state, caller, origin(&headers), &board_id, vec![op]).await?;
    Ok((StatusCode::CREATED, Json(only(result))))
}

async fn get_item(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    Ok(Json(json!(items::get(&mut conn, &id).await?)))
}

async fn update_item(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ItemPatch>,
) -> AppResult<Json<Value>> {
    let board = board_of_item(&state, &id).await?;
    let op = Op::UpdateItem {
        item: id,
        version: body.version,
        title: body.title,
        content: body.content,
        style: body.style,
    };
    let result = run_batch(&state, caller, origin(&headers), &board, vec![op]).await;
    Ok(Json(only(unwrap_single(result)?)))
}

/// A one-op batch reports errors with `op_index`; for single-item routes
/// that's noise, so unwrap it.
fn unwrap_single(result: AppResult<BatchResult>) -> AppResult<BatchResult> {
    result.map_err(|e| match e {
        AppError::InBatch { source, .. } => *source,
        other => other,
    })
}

async fn set_geometry(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Geometry>,
) -> AppResult<Json<Value>> {
    let board = board_of_item(&state, &id).await?;
    let op = Op::SetGeometry {
        item: id,
        x: body.x,
        y: body.y,
        w: body.w,
        h: body.h,
        z: body.z,
        locked: body.locked,
    };
    let result = run_batch(&state, caller, origin(&headers), &board, vec![op]).await;
    Ok(Json(only(unwrap_single(result)?)))
}

#[derive(Deserialize)]
struct DeleteQuery {
    #[serde(default)]
    with_contents: bool,
}

async fn delete_item(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<DeleteQuery>,
) -> AppResult<Json<Value>> {
    let board = board_of_item(&state, &id).await?;
    let op = Op::DeleteItem {
        item: id,
        with_contents: query.with_contents,
    };
    let result = run_batch(&state, caller, origin(&headers), &board, vec![op]).await;
    Ok(Json(only(unwrap_single(result)?)))
}

async fn undelete_item(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let board = board_of_item(&state, &id).await?;
    let op = Op::UndeleteItem { item: id };
    let result = run_batch(&state, caller, origin(&headers), &board, vec![op]).await;
    Ok(Json(only(unwrap_single(result)?)))
}

async fn move_items(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<MoveRequest>,
) -> AppResult<Json<Value>> {
    let mut tx = db::write_tx(&state.pool).await?;
    let moved = moves::move_items(&mut tx, ctx(&state, caller), body).await?;
    let mut left_behind = Vec::new();
    for id in &moved.affected {
        if let Ok(item) = items::get(&mut tx, id).await {
            left_behind.push(item);
        }
    }
    tx.commit().await?;

    // Both boards hear about the move: the old one loses the items, the new
    // one gains them.
    let origin = origin(&headers);
    let mut from_changes: Vec<Change> = moved
        .items
        .iter()
        .map(|i| Change::item("item.moved", i.clone()))
        .collect();
    from_changes.extend(
        moved
            .removed_connectors
            .iter()
            .map(|c| Change::removed("connector.deleted", c.clone())),
    );
    from_changes.extend(
        left_behind
            .into_iter()
            .map(|i| Change::item("item.geometry", i)),
    );
    state
        .hub
        .publish(&moved.from_board, origin.clone(), from_changes);
    let mut to_changes: Vec<Change> = moved
        .items
        .iter()
        .map(|i| Change::item("item.moved", i.clone()))
        .collect();
    let mut conn = state.pool.acquire().await?;
    for cid in &moved.connectors {
        if let Ok(c) = connectors::get(&mut conn, cid).await {
            to_changes.push(Change::connector("connector.created", c));
        }
    }
    state.hub.publish(&moved.to_board, origin, to_changes);
    Ok(Json(json!(moved)))
}

async fn revisions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    let revisions = items::revisions(&mut conn, &id).await?;
    Ok(Json(json!({ "revisions": revisions })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreBody {
    version: i64,
}

async fn restore_revision(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path((id, rev)): Path<(String, i64)>,
    ApiJson(body): ApiJson<RestoreBody>,
) -> AppResult<Json<Value>> {
    let mut tx = db::write_tx(&state.pool).await?;
    let item = items::restore(&mut tx, ctx(&state, caller), &id, rev, body.version).await?;
    tx.commit().await?;
    state.hub.publish(
        &item.board_id,
        origin(&headers),
        vec![Change::item("item.updated", item.clone())],
    );
    Ok(Json(json!(item)))
}

// ---------- Connectors ----------

async fn create_connector(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(board_id): Path<String>,
    ApiJson(body): ApiJson<NewConnector>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let op = Op::CreateConnector {
        reference: None,
        from: body.from,
        to: body.to,
        from_anchor: body.from_anchor,
        to_anchor: body.to_anchor,
        label: body.label,
        style: body.style,
    };
    let result = run_batch(&state, caller, origin(&headers), &board_id, vec![op]).await;
    Ok((StatusCode::CREATED, Json(only(unwrap_single(result)?))))
}

async fn update_connector(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ConnectorPatch>,
) -> AppResult<Json<Value>> {
    let board = board_of_connector(&state, &id).await?;
    let op = Op::UpdateConnector {
        connector: id,
        from: body.from,
        to: body.to,
        from_anchor: body.from_anchor,
        to_anchor: body.to_anchor,
        label: body.label,
        style: body.style,
    };
    let result = run_batch(&state, caller, origin(&headers), &board, vec![op]).await;
    Ok(Json(only(unwrap_single(result)?)))
}

async fn delete_connector(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let board = board_of_connector(&state, &id).await?;
    let op = Op::DeleteConnector { connector: id };
    unwrap_single(run_batch(&state, caller, origin(&headers), &board, vec![op]).await)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn undelete_connector(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let board = board_of_connector(&state, &id).await?;
    let op = Op::UndeleteConnector { connector: id };
    let result = run_batch(&state, caller, origin(&headers), &board, vec![op]).await;
    Ok(Json(only(unwrap_single(result)?)))
}

// ---------- The board model (spec 9.2) ----------

/// `GET /api/boards/{id}/model`: the board model, with an `ETag`; a request
/// carrying the same `If-None-Match` gets 304 and no body.
async fn get_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let mut conn = state.pool.acquire().await?;
    let model = crate::model::load(&mut conn, &id).await?;
    let etag = crate::model::etag(&model);
    let cache = [
        (header::ETAG, etag.clone()),
        (header::CACHE_CONTROL, "private, no-cache".to_string()),
    ];
    let matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"));
    if matches {
        return Ok((StatusCode::NOT_MODIFIED, cache).into_response());
    }
    Ok((cache, Json(model)).into_response())
}

/// `GET /api/model.schema.json`: JSON Schema for the board model. Public:
/// it describes the shape, not any board's contents.
async fn model_schema() -> Json<Value> {
    Json(json!(schemars::schema_for!(board_model::BoardModel)))
}

// ---------- Search ----------

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    board_id: Option<String>,
}

async fn search_items(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> AppResult<Json<Value>> {
    let hits = search::search(&state.pool, &query.q, query.board_id.as_deref()).await?;
    Ok(Json(json!({ "results": hits })))
}
