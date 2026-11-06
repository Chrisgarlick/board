//! The axum router and shared state.

use std::sync::Arc;

use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::assets;
use crate::config::Config;

/// State shared by every handler.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .fallback(assets::serve)
        .with_state(state)
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    fn test_router() -> Router {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load_or_init(&dir.path().join("board.toml")).unwrap();
        router(AppState {
            config: Arc::new(config),
        })
    }

    #[tokio::test]
    async fn healthz_reports_ok_and_version() {
        let res = test_router()
            .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn missing_asset_with_extension_is_404() {
        let res = test_router()
            .oneshot(
                Request::get("/no-such-file.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_api_route_is_404_not_the_app() {
        let res = test_router()
            .oneshot(Request::get("/api/nope").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
