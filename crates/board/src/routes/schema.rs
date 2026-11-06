//! The board schema over REST (spec section 6, "Schema"). Every write
//! publishes `table.*` events carrying the new `schema_version`.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ctx, origin};
use crate::app::AppState;
use crate::content::CardType;
use crate::db;
use crate::error::{ApiJson, AppError, AppResult};
use crate::events::Change;
use crate::schema::er;
use crate::service::tables::{self, NewTable, TablePatch, Written};
use crate::service::{boards, items};
use crate::types::Caller;

/// Publish a schema write: the table itself, tables changed as a side
/// effect, and the schema card a new table was added to.
pub fn publish(state: &AppState, origin: Option<String>, kind: &'static str, w: &Written) {
    let mut changes = vec![if kind == "table.deleted" {
        Change::table_deleted(w.table.id.clone(), w.schema_version)
    } else {
        Change::table(kind, w.table.clone(), w.schema_version)
    }];
    changes.extend(
        w.updated
            .iter()
            .map(|t| Change::table("table.updated", t.clone(), w.schema_version)),
    );
    if let Some(item) = &w.item {
        changes.push(Change::item("item.updated", item.clone()));
    }
    state.hub.publish(&w.table.board_id, origin, changes);
}

pub async fn get_schema(
    State(state): State<AppState>,
    Path(board_id): Path<String>,
) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    let board = boards::get(&mut conn, &board_id).await?;
    let tables = tables::list_live(&mut conn, &board_id).await?;
    Ok(Json(
        json!({ "schema_version": board.schema_version, "tables": tables }),
    ))
}

pub async fn create_table(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(board_id): Path<String>,
    ApiJson(body): ApiJson<NewTable>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let mut tx = db::write_tx(&state.pool).await?;
    let written = tables::create(&mut tx, ctx(&state, caller), &board_id, body).await?;
    tx.commit().await?;
    publish(&state, origin(&headers), "table.created", &written);
    Ok((StatusCode::CREATED, Json(json!(written))))
}

pub async fn get_table(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    Ok(Json(json!(tables::get(&mut conn, &id).await?)))
}

pub async fn update_table(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<TablePatch>,
) -> AppResult<Json<Value>> {
    let mut tx = db::write_tx(&state.pool).await?;
    let written = tables::update(&mut tx, ctx(&state, caller), &id, body).await?;
    tx.commit().await?;
    publish(&state, origin(&headers), "table.updated", &written);
    Ok(Json(json!(written)))
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    /// `1` or `true`: also remove references from other tables.
    #[serde(default)]
    force: Option<String>,
}

pub async fn delete_table(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<DeleteQuery>,
) -> AppResult<Json<Value>> {
    let force = matches!(q.force.as_deref(), Some("1" | "true"));
    let mut tx = db::write_tx(&state.pool).await?;
    let written = tables::delete(&mut tx, ctx(&state, caller), &id, force).await?;
    tx.commit().await?;
    publish(&state, origin(&headers), "table.deleted", &written);
    Ok(Json(json!(written)))
}

pub async fn undelete_table(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let mut tx = db::write_tx(&state.pool).await?;
    let written = tables::undelete(&mut tx, ctx(&state, caller), &id).await?;
    tx.commit().await?;
    publish(&state, origin(&headers), "table.undeleted", &written);
    Ok(Json(json!(written)))
}

pub async fn table_revisions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    let revisions = tables::revisions(&mut conn, &id).await?;
    Ok(Json(json!({ "revisions": revisions })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreBody {
    version: i64,
}

pub async fn restore_table(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Path((id, rev)): Path<(String, i64)>,
    ApiJson(body): ApiJson<RestoreBody>,
) -> AppResult<Json<Value>> {
    let mut tx = db::write_tx(&state.pool).await?;
    let written = tables::restore(&mut tx, ctx(&state, caller), &id, rev, body.version).await?;
    tx.commit().await?;
    publish(&state, origin(&headers), "table.updated", &written);
    Ok(Json(json!(written)))
}

/// `GET /api/items/{id}/er`: a schema card's tables as Mermaid `erDiagram`.
pub async fn item_er(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    let item = items::get(&mut conn, &id).await?;
    let ids = schema_card_tables(&item)?;
    let board = tables::board_tables(&mut conn, &item.board_id).await?;
    let view = er::view(&ids, &board);
    Ok(Json(json!({
        "mermaid": er::mermaid(&view),
        "tables": view.tables[..view.shown].iter().map(|t| json!({ "id": t.id, "name": t.name })).collect::<Vec<_>>(),
        "stubs": view.tables[view.shown..].iter().map(|t| json!({ "id": t.id, "name": t.name })).collect::<Vec<_>>(),
    })))
}

/// The table ids a schema card shows.
pub fn schema_card_tables(item: &items::Item) -> AppResult<Vec<String>> {
    if item.card_type.as_deref() != Some(CardType::Schema.as_str()) {
        return Err(AppError::invalid("item_id", "not a schema card"));
    }
    Ok(item
        .content
        .get("table_ids")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
}
