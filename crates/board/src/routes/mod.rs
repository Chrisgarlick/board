//! REST handlers (spec section 6). Each one opens a transaction, calls the
//! service layer and commits; the services hold the rules.

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router, middleware};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::auth;
use crate::db;
use crate::error::{ApiJson, AppError, AppResult};
use crate::service::batch::{self, Batch};
use crate::service::boards::{self, BoardPatch, NewBoard};
use crate::service::items::{self, Geometry, ItemPatch, NewItem};
use crate::service::search;
use crate::types::{Caller, Ctx};

const DEFAULT_BODY_LIMIT: usize = 1024 * 1024;
const BATCH_BODY_LIMIT: usize = 2 * 1024 * 1024;

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
        .route("/boards/{id}/items", post(create_item))
        .route(
            "/boards/{id}/batch",
            post(apply_batch).layer(DefaultBodyLimit::max(BATCH_BODY_LIMIT)),
        )
        .route("/boards/{id}/trash", get(trash))
        .route(
            "/items/{id}",
            get(get_item).patch(update_item).delete(delete_item),
        )
        .route("/items/{id}/geometry", patch(set_geometry))
        .route("/items/{id}/undelete", post(undelete_item))
        .route("/items/{id}/revisions", get(revisions))
        .route("/items/{id}/restore/{rev}", post(restore_revision))
        .route("/search", get(search_items))
        .fallback(api_not_found)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(middleware::from_fn_with_state(state, auth::require_caller))
}

/// Routes outside `/api` that don't need a login.
pub fn public() -> Router<AppState> {
    Router::new().route("/login/{code}", get(login))
}

fn ctx(state: &AppState, caller: Caller) -> Ctx {
    Ctx {
        actor: caller.actor,
        now: state.clock.now(),
    }
}

async fn api_not_found() -> AppError {
    AppError::NotFound("route")
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
    Path(id): Path<String>,
    ApiJson(body): ApiJson<BoardPatch>,
) -> AppResult<Json<Value>> {
    let mut tx = db::write_tx(&state.pool).await?;
    let board = boards::update(&mut tx, ctx(&state, caller), &id, body).await?;
    tx.commit().await?;
    Ok(Json(json!(board)))
}

async fn delete_board(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let mut tx = db::write_tx(&state.pool).await?;
    boards::delete(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn trash(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    boards::require_exists(&mut conn, &id).await?;
    let items = items::list_trash(&mut conn, &id).await?;
    Ok(Json(
        json!({ "items": items, "connectors": [], "tables": [] }),
    ))
}

async fn apply_batch(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Batch>,
) -> AppResult<Json<Value>> {
    let mut tx = db::write_tx(&state.pool).await?;
    let result = batch::apply(&mut tx, ctx(&state, caller), &id, body).await?;
    tx.commit().await?;
    Ok(Json(json!(result)))
}

// ---------- Items ----------

async fn create_item(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(board_id): Path<String>,
    ApiJson(body): ApiJson<NewItem>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let mut tx = db::write_tx(&state.pool).await?;
    let item = items::create(&mut tx, ctx(&state, caller), &board_id, body).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!(item))))
}

async fn get_item(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    Ok(Json(json!(items::get(&mut conn, &id).await?)))
}

async fn update_item(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ItemPatch>,
) -> AppResult<Json<Value>> {
    let mut tx = db::write_tx(&state.pool).await?;
    let item = items::update(&mut tx, ctx(&state, caller), &id, body).await?;
    tx.commit().await?;
    Ok(Json(json!(item)))
}

async fn set_geometry(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Geometry>,
) -> AppResult<Json<Value>> {
    let mut tx = db::write_tx(&state.pool).await?;
    let item = items::set_geometry(&mut tx, ctx(&state, caller), &id, body).await?;
    tx.commit().await?;
    Ok(Json(json!(item)))
}

async fn delete_item(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let mut tx = db::write_tx(&state.pool).await?;
    items::delete(&mut tx, ctx(&state, caller), &id).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn undelete_item(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let mut tx = db::write_tx(&state.pool).await?;
    let item = items::undelete(&mut tx, ctx(&state, caller), &id).await?;
    tx.commit().await?;
    Ok(Json(json!(item)))
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
    Path((id, rev)): Path<(String, i64)>,
    ApiJson(body): ApiJson<RestoreBody>,
) -> AppResult<Json<Value>> {
    let mut tx = db::write_tx(&state.pool).await?;
    let item = items::restore(&mut tx, ctx(&state, caller), &id, rev, body.version).await?;
    tx.commit().await?;
    Ok(Json(json!(item)))
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
