//! Shared test harness: a real app on a temporary database, with a clock
//! the tests can move.

#![allow(dead_code)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use board::app::{AppState, router};
use board::auth;
use board::clock::{Clock, ManualClock};
use board::config::Config;
use board::db;
use board::types::{Actor, Scope};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

pub const ORIGIN: &str = "http://localhost:7070";

pub struct TestApp {
    pub router: Router,
    pub state: AppState,
    pub clock: Arc<ManualClock>,
    pub admin: String,
    _dir: tempfile::TempDir,
}

pub struct Res {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: Value,
}

impl TestApp {
    pub async fn new() -> Self {
        Self::with_config("").await
    }

    /// A test app with extra lines in board.toml (e.g. Typeset settings).
    #[allow(dead_code)]
    pub async fn with_config(extra: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("board.toml");
        std::fs::write(
            &config_path,
            format!(
                "database = \"{}\"\n{extra}\n",
                dir.path().join("board.db").display()
            ),
        )
        .unwrap();
        let config = Config::load_or_init(&config_path).unwrap();
        let pool = db::connect(&config.database).await.unwrap();
        let clock = Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 11, 6, 9, 0, 0).unwrap(),
        ));
        let state = AppState {
            config: Arc::new(config),
            pool,
            clock: clock.clone(),
            hub: Default::default(),
            renders: Default::default(),
        };
        let admin = auth::create_token(
            &state.pool,
            "test-admin",
            Scope::Admin,
            Actor::Api,
            clock.now(),
        )
        .await
        .unwrap()
        .token;
        Self {
            router: router(state.clone()),
            state,
            clock,
            admin,
            _dir: dir,
        }
    }

    pub async fn token(&self, scope: Scope, actor: Actor) -> String {
        auth::create_token(&self.state.pool, "t", scope, actor, self.clock.now())
            .await
            .unwrap()
            .token
    }

    pub async fn send(&self, request: Request<Body>) -> Res {
        let res = self.router.clone().oneshot(request).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
        };
        Res {
            status,
            headers,
            body,
        }
    }

    /// A request with the admin token.
    pub async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> Res {
        self.call_as(&self.admin.clone(), method, uri, body).await
    }

    pub async fn call_as(&self, token: &str, method: &str, uri: &str, body: Option<Value>) -> Res {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        let request = match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap();
        self.send(request).await
    }

    pub async fn board(&self) -> String {
        let res = self
            .call(
                "POST",
                "/api/boards",
                Some(json!({ "title": "Acme rebuild" })),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        res.body["id"].as_str().unwrap().to_string()
    }

    pub async fn note(&self, board: &str, title: &str, markdown: &str) -> Value {
        let res = self
            .call(
                "POST",
                &format!("/api/boards/{board}/items"),
                Some(json!({
                    "kind": "card", "card_type": "note", "title": title,
                    "content": { "markdown": markdown }
                })),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        res.body
    }

    pub async fn revision_count(&self, item: &str) -> usize {
        self.call("GET", &format!("/api/items/{item}/revisions"), None)
            .await
            .body["revisions"]
            .as_array()
            .unwrap()
            .len()
    }
}

impl TestApp {
    /// Create an item of any kind on a board; panics unless it's created.
    pub async fn item(&self, board: &str, body: Value) -> Value {
        let res = self
            .call("POST", &format!("/api/boards/{board}/items"), Some(body))
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        res.body
    }

    pub async fn batch(&self, board: &str, ops: Value) -> Res {
        self.call(
            "POST",
            &format!("/api/boards/{board}/batch"),
            Some(json!({ "ops": ops })),
        )
        .await
    }

    pub async fn full(&self, board: &str) -> Value {
        self.call("GET", &format!("/api/boards/{board}"), None)
            .await
            .body
    }
}
