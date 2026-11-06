//! Rendered images (spec section 8.2): an item or a selection as SVG or PNG,
//! and signed links that work without a login.

use std::collections::BTreeMap;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::TimeDelta;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::{ApiJson, AppError, AppResult};
use crate::render::{self, Format, Request, Theme};
use crate::signing;

const DEFAULT_LINK_DAYS: i64 = 7;
const MAX_LINK_DAYS: i64 = 90;

#[derive(Debug, Default, Deserialize)]
pub struct RenderQuery {
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub title: Option<bool>,
    /// `paths` (default) or `text`, for SVG.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub scale: Option<f32>,
}

impl RenderQuery {
    fn request(&self, format: Format) -> AppResult<Request> {
        let mut r = Request {
            format,
            ..Request::default()
        };
        if let Some(t) = &self.theme {
            r.theme = Theme::parse(t).ok_or_else(|| AppError::invalid("theme", "light or dark"))?;
        }
        if let Some(t) = self.title {
            r.title = t;
        }
        match self.text.as_deref() {
            None | Some("paths") => r.text_as_paths = true,
            Some("text") => r.text_as_paths = false,
            Some(_) => return Err(AppError::invalid("text", "paths or text")),
        }
        if let Some(s) = self.scale {
            if !(1.0..=4.0).contains(&s) {
                return Err(AppError::invalid("scale", "between 1 and 4"));
            }
            r.scale = s;
        }
        Ok(r)
    }

    /// The query as signing parameters (only what was set).
    fn params(&self) -> BTreeMap<String, String> {
        let mut p = BTreeMap::new();
        if let Some(t) = &self.theme {
            p.insert("theme".into(), t.clone());
        }
        if let Some(t) = self.title {
            p.insert("title".into(), t.to_string());
        }
        if let Some(t) = &self.text {
            p.insert("text".into(), t.clone());
        }
        if let Some(s) = self.scale {
            p.insert("scale".into(), s.to_string());
        }
        p
    }
}

fn image(bytes: std::sync::Arc<Vec<u8>>, format: Format, filename: &str) -> Response {
    let mut res = Response::new(Body::from(bytes.as_ref().clone()));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(format.content_type()),
    );
    // Images show the item as it is now; a short cache is fine.
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=60"),
    );
    let safe: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if let Ok(v) = HeaderValue::from_str(&format!("inline; filename=\"{safe}.{}\"", format.ext())) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    res
}

/// Render one item (a frame with its contents, or anything else alone).
async fn render_item(state: &AppState, id: &str, req: Request) -> AppResult<Response> {
    let mut conn = state.pool.acquire().await?;
    let (frame, items, connectors) = render::scene(&mut conn, id).await?;
    drop(conn);
    let name = frame
        .as_ref()
        .and_then(|f| f.title.clone())
        .or_else(|| items.first().and_then(|i| i.title.clone()))
        .unwrap_or_else(|| "board".into());
    let cache = state.renders.clone();
    let bytes = tokio::task::spawn_blocking(move || {
        render::draw(&cache, frame.as_ref(), &items, &connectors, req)
    })
    .await
    .map_err(|e| AppError::Internal(e.into()))??;
    Ok(image(bytes, req.format, &name))
}

pub async fn item_svg(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<RenderQuery>,
) -> AppResult<Response> {
    render_item(&state, &id, q.request(Format::Svg)?).await
}

pub async fn item_png(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<RenderQuery>,
) -> AppResult<Response> {
    render_item(&state, &id, q.request(Format::Png)?).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionBody {
    pub item_ids: Vec<String>,
    #[serde(default = "png")]
    pub format: String,
    #[serde(default)]
    pub scale: Option<f32>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

fn png() -> String {
    "png".into()
}

pub async fn selection(
    State(state): State<AppState>,
    Path(board_id): Path<String>,
    ApiJson(body): ApiJson<SelectionBody>,
) -> AppResult<Response> {
    let format =
        Format::parse(&body.format).ok_or_else(|| AppError::invalid("format", "png or svg"))?;
    let req = RenderQuery {
        theme: body.theme,
        title: None,
        text: body.text,
        scale: body.scale,
    }
    .request(format)?;
    let mut conn = state.pool.acquire().await?;
    let (items, connectors) = render::selection(&mut conn, &board_id, &body.item_ids).await?;
    drop(conn);
    let cache = state.renders.clone();
    let bytes =
        tokio::task::spawn_blocking(move || render::draw(&cache, None, &items, &connectors, req))
            .await
            .map_err(|e| AppError::Internal(e.into()))??;
    Ok(image(bytes, format, "selection"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkBody {
    #[serde(default = "png")]
    pub format: String,
    #[serde(default)]
    pub scale: Option<f32>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub title: Option<bool>,
    /// How long the link works, 1–90 days (default 7).
    #[serde(default)]
    pub days: Option<i64>,
}

/// A signed image link for an item: `{public_origin}/i/{id}.{ext}?…&exp&sig`.
pub fn image_link(state: &AppState, id: &str, body: &LinkBody) -> AppResult<Value> {
    let format =
        Format::parse(&body.format).ok_or_else(|| AppError::invalid("format", "png or svg"))?;
    let days = body.days.unwrap_or(DEFAULT_LINK_DAYS);
    if !(1..=MAX_LINK_DAYS).contains(&days) {
        return Err(AppError::invalid(
            "days",
            format!("between 1 and {MAX_LINK_DAYS}"),
        ));
    }
    let q = RenderQuery {
        theme: body.theme.clone(),
        title: body.title,
        text: None,
        scale: body.scale,
    };
    q.request(format)?; // validate
    let expires = state.clock.now().plus(TimeDelta::days(days));
    let exp = expires.unix();
    let path = format!("/i/{id}.{}", format.ext());
    let secret = state.config.secret_bytes()?;
    let signed = signing::signed_path(&secret, &path, q.params(), exp);
    Ok(
        json!({ "url": format!("{}{signed}", state.config.public_origin), "expires_at": expires.to_db() }),
    )
}

pub async fn create_link(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<LinkBody>,
) -> AppResult<axum::Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    crate::service::items::get(&mut conn, &id).await?;
    Ok(axum::Json(image_link(&state, &id, &body)?))
}

/// `GET /i/{id}.{ext}?…&exp&sig`: a signed image, no login needed.
pub async fn public_image(
    State(state): State<AppState>,
    Path(file): Path<String>,
    Query(params): Query<BTreeMap<String, String>>,
) -> Response {
    let refused = |status: StatusCode, why: &str| (status, why.to_string()).into_response();
    let Some((id, ext)) = file.rsplit_once('.') else {
        return refused(StatusCode::NOT_FOUND, "not found");
    };
    let Some(format) = Format::parse(ext) else {
        return refused(StatusCode::NOT_FOUND, "not found");
    };
    let Ok(secret) = state.config.secret_bytes() else {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "server misconfigured");
    };
    let now = state.clock.now().unix();
    match signing::verify(&secret, &format!("/i/{file}"), &params, now) {
        Ok(()) => {}
        Err(signing::Invalid::Expired) => {
            return refused(StatusCode::GONE, "this link has expired");
        }
        Err(_) => return refused(StatusCode::FORBIDDEN, "this link isn't valid"),
    }
    let q = RenderQuery {
        theme: params.get("theme").cloned(),
        title: params.get("title").and_then(|t| t.parse().ok()),
        text: params.get("text").cloned(),
        scale: params.get("scale").and_then(|s| s.parse().ok()),
    };
    let req = match q.request(format) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match render_item(&state, id, req).await {
        Ok(res) => res,
        Err(AppError::NotFound(_)) => refused(StatusCode::NOT_FOUND, "that item no longer exists"),
        Err(e) => e.into_response(),
    }
}
