//! The axum router and shared state.

use std::sync::Arc;

use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::assets;
use crate::clock::Clock;
use crate::config::Config;
use crate::events::Hub;
use crate::render;
use crate::routes;

/// State shared by every handler.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pool: SqlitePool,
    pub clock: Arc<dyn Clock>,
    pub hub: Hub,
    pub renders: render::Cache,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .merge(routes::public())
        .nest("/api", routes::api(state.clone()))
        .nest_service("/mcp", crate::mcp::service(state.clone()))
        .fallback(assets::serve)
        .with_state(state)
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}
