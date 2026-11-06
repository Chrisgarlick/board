//! API errors and their JSON shape.
//!
//! Every error body is `{ "error": <code>, "message": <text>, ... }`, with
//! `path` for validation errors and `current` for version conflicts (spec
//! sections 5.5 and 6), so REST and MCP clients can handle them uniformly.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0} not found")]
    NotFound(&'static str),

    /// A stale `version`: carries the current record so the caller can merge.
    #[error("changed since you loaded it")]
    Conflict { current: Value },

    /// Any other 409, e.g. moving a locked item.
    #[error("{message}")]
    State { code: &'static str, message: String },

    #[error("{message}")]
    Invalid { path: String, message: String },

    #[error("{0}")]
    BadRequest(String),

    #[error("not logged in")]
    Unauthorized,

    #[error("{0}")]
    Forbidden(&'static str),

    #[error("request body too large")]
    PayloadTooLarge,

    /// An error in one operation of a batch: which op, and what went wrong.
    #[error("operation {index}: {source}")]
    InBatch {
        index: usize,
        #[source]
        source: Box<AppError>,
    },

    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn invalid(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Invalid {
            path: path.into(),
            message: message.into(),
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict { .. } | Self::State { .. } => StatusCode::CONFLICT,
            Self::Invalid { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::InBatch { source, .. } => source.status(),
            Self::Database(_) | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn body(&self) -> Value {
        match self {
            Self::NotFound(_) => json!({ "error": "not_found", "message": self.to_string() }),
            Self::Conflict { current } => json!({
                "error": "conflict",
                "message": self.to_string(),
                "current": current,
            }),
            Self::State { code, message } => json!({ "error": code, "message": message }),
            Self::Invalid { path, message } => json!({
                "error": "invalid_content",
                "path": path,
                "message": message,
            }),
            Self::BadRequest(m) => json!({ "error": "bad_request", "message": m }),
            Self::Unauthorized => json!({ "error": "unauthorized", "message": self.to_string() }),
            Self::Forbidden(m) => json!({ "error": "forbidden", "message": m }),
            Self::PayloadTooLarge => {
                json!({ "error": "payload_too_large", "message": self.to_string() })
            }
            Self::InBatch { index, source } => {
                let mut body = source.body();
                body["op_index"] = json!(index);
                body
            }
            // Don't leak internals; the details go to the log.
            Self::Database(_) | Self::Internal(_) => {
                json!({ "error": "internal", "message": "internal error" })
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        if status.is_server_error() {
            tracing::error!(error = ?self, "request failed");
        }
        (status, Json(self.body())).into_response()
    }
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        match rejection.status() {
            StatusCode::PAYLOAD_TOO_LARGE => Self::PayloadTooLarge,
            StatusCode::UNPROCESSABLE_ENTITY => Self::Invalid {
                path: String::new(),
                message: rejection.body_text(),
            },
            _ => Self::BadRequest(rejection.body_text()),
        }
    }
}

/// `Json<T>` whose rejections use our error shape instead of plain text.
#[derive(axum::extract::FromRequest)]
#[from_request(via(axum::Json), rejection(AppError))]
pub struct ApiJson<T>(pub T);
