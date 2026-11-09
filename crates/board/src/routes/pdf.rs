//! Planning PDFs through Typeset (spec section 8): Board builds a Typeset
//! document from a board ([`crate::typeset`]) and renders it with its own
//! Clients caller key, the same way Design does. Typeset never calls Board:
//! drawings travel inside the document as data URIs.

use std::collections::BTreeMap;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, http::HeaderMap};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chrono::TimeDelta;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::{ApiJson, AppError, AppResult};
use crate::render::{self, Format, Request, Scene, Theme};
use crate::schema::BoardTable;
use crate::service::{boards, connectors, items, tables};
use crate::signing;
use crate::typeset::{self, Document, Image, MAX_IMAGE_BYTES, Options};

const RENDER_TIMEOUT_SECS: u64 = 90;
const DEFAULT_LINK_DAYS: i64 = 7;
const MAX_LINK_DAYS: i64 = 90;

/// How drawings go into the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Images {
    /// Inside it, as data URIs: for PDFs (Typeset needn't reach Board).
    Embedded,
    /// Signed HTTPS links to Board's images: a small document for Claude or
    /// Design to pass on to Typeset themselves.
    Links,
}

/// Build a board's Typeset document.
pub async fn build(
    state: &AppState,
    board_id: &str,
    opts: Options,
    images: Images,
) -> AppResult<Document> {
    let mut conn = state.pool.acquire().await?;
    let board = boards::get(&mut conn, board_id).await?;
    let all = items::list_live(&mut conn, board_id).await?;
    let conns = connectors::list_live(&mut conn, board_id).await?;
    let schema = tables::board_tables(&mut conn, board_id).await?;
    drop(conn);
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        typeset::document(&board, &all, &schema, &opts, |item| match images {
            Images::Embedded => embedded(&state, item, &all, &conns, &schema),
            Images::Links => link(&state, item),
        })
    })
    .await
    .map_err(|e| AppError::Internal(e.into()))?
}

/// A frame or schema card drawn as a data URI: SVG with text as paths
/// (vector, needs no fonts), or PNG if the SVG is over Typeset's limit.
fn embedded(
    state: &AppState,
    item: &items::Item,
    all: &[items::Item],
    conns: &[connectors::Connector],
    schema: &[BoardTable],
) -> Option<Image> {
    let scene = if item.kind == "frame" {
        Scene {
            frame: Some(item.clone()),
            items: all
                .iter()
                .filter(|i| i.frame_id.as_deref() == Some(item.id.as_str()))
                .cloned()
                .collect(),
            connectors: conns.to_vec(),
            tables: schema.to_vec(),
        }
    } else {
        Scene {
            frame: None,
            items: vec![item.clone()],
            connectors: Vec::new(),
            tables: schema.to_vec(),
        }
    };
    // Print theme, no title (the section opener carries it).
    let svg_req = Request {
        format: Format::Svg,
        title: false,
        theme: Theme::Light,
        text_as_paths: true,
        scale: 1.0,
    };
    let svg = render::draw(&state.renders, &scene, svg_req).ok()?;
    if svg.len() <= MAX_IMAGE_BYTES {
        return Some(Image {
            url: format!(
                "data:image/svg+xml;base64,{}",
                STANDARD.encode(svg.as_slice())
            ),
        });
    }
    let png = render::draw(
        &state.renders,
        &scene,
        Request {
            format: Format::Png,
            scale: 2.0,
            ..svg_req
        },
    )
    .ok()?;
    Some(Image {
        url: format!("data:image/png;base64,{}", STANDARD.encode(png.as_slice())),
    })
}

/// A signed SVG link to a frame or schema card (30 days).
fn link(state: &AppState, item: &items::Item) -> Option<Image> {
    let body = super::images::LinkBody {
        format: "svg".into(),
        scale: None,
        theme: None,
        title: Some(false),
        days: Some(30),
    };
    let v = super::images::image_link(state, &item.id, &body).ok()?;
    Some(Image {
        url: v["url"].as_str()?.to_string(),
    })
}

// ---------- Talking to Typeset ----------

fn typeset_config(state: &AppState) -> AppResult<(String, String)> {
    let not_set = || {
        AppError::State {
        code: "typeset_not_configured",
        message: "Typeset isn't set up on this Board: add typeset_url and typeset_key (a Clients caller key) to board.toml and restart".into(),
    }
    };
    let url = state.config.typeset_url.clone().ok_or_else(not_set)?;
    let key = state
        .config
        .typeset_key
        .clone()
        .filter(|k| !k.is_empty())
        .ok_or_else(not_set)?;
    Ok((url.trim_end_matches('/').to_string(), key))
}

/// Call Typeset. Returns the response body.
async fn call(
    state: &AppState,
    method: &'static str,
    path: &str,
    body: Option<Value>,
) -> AppResult<Vec<u8>> {
    let (base, key) = typeset_config(state)?;
    let url = format!("{base}{path}");
    tokio::task::spawn_blocking(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(RENDER_TIMEOUT_SECS)))
            .http_status_as_error(false)
            .build()
            .into();
        let auth = format!("Bearer {key}");
        let result = match body {
            Some(b) => agent
                .post(&url)
                .header("Authorization", &auth)
                .header("Content-Type", "application/json")
                .send(b.to_string()),
            None if method == "GET" => agent.get(&url).header("Authorization", &auth).call(),
            None => agent.post(&url).header("Authorization", &auth).send_empty(),
        };
        let mut res = result.map_err(|e| AppError::State {
            code: "typeset_unreachable",
            message: format!("Typeset can't be reached: {e}"),
        })?;
        let status = res.status().as_u16();
        let bytes = res
            .body_mut()
            .with_config()
            .limit(200 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| AppError::State {
                code: "typeset_unreachable",
                message: format!("Typeset's reply was cut off: {e}"),
            })?;
        if !(200..300).contains(&status) {
            let detail = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|v| v["error"].as_str().map(str::to_string))
                .unwrap_or_default();
            tracing::warn!(status, %detail, "Typeset refused a request");
            return Err(AppError::State {
                code: "typeset_error",
                message: if detail.is_empty() {
                    format!("Typeset said {status}")
                } else {
                    format!("Typeset said {status}: {detail}")
                },
            });
        }
        Ok(bytes)
    })
    .await
    .map_err(|e| AppError::Internal(e.into()))?
}

fn render_body(doc: &Document, client: Option<&str>, print: bool) -> Value {
    let mut body = json!({
        "content": doc.json.to_string(),
        "input_format": "json",
        "format": "pdf",
        "document_type": "proposal",
        "print": print,
    });
    if let Some(c) = client {
        body["client"] = json!(c);
    }
    body
}

/// The client whose brand to use: the request's, else the board's.
async fn client_for(state: &AppState, board_id: &str, opts: &Options) -> AppResult<Option<String>> {
    if opts.client.is_some() {
        return Ok(opts.client.clone());
    }
    let mut conn = state.pool.acquire().await?;
    Ok(boards::get(&mut conn, board_id).await?.client)
}

/// Render a board as a PDF. Returns the file name and bytes.
pub async fn pdf(
    state: &AppState,
    board_id: &str,
    opts: Options,
    print: bool,
) -> AppResult<(String, Vec<u8>)> {
    let client = client_for(state, board_id, &opts).await?;
    let doc = build(state, board_id, opts, Images::Embedded).await?;
    let title = doc.json["frontmatter"]["title"]
        .as_str()
        .unwrap_or("board")
        .to_string();
    let bytes = call(
        state,
        "POST",
        "/api/render",
        Some(render_body(&doc, client.as_deref(), print)),
    )
    .await?;
    Ok((filename(&title), bytes))
}

/// Page thumbnails (Typeset's proof), with the document's warnings.
pub async fn proof(state: &AppState, board_id: &str, opts: Options) -> AppResult<Value> {
    let client = client_for(state, board_id, &opts).await?;
    let doc = build(state, board_id, opts, Images::Embedded).await?;
    let bytes = call(
        state,
        "POST",
        "/api/proof",
        Some(render_body(&doc, client.as_deref(), false)),
    )
    .await?;
    let mut proof: Value =
        serde_json::from_slice(&bytes).map_err(|e| AppError::Internal(e.into()))?;
    proof["warnings"] = json!(doc.warnings);
    Ok(proof)
}

fn filename(title: &str) -> String {
    let safe: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("{}.pdf", if safe.is_empty() { "board" } else { &safe })
}

fn pdf_response(name: &str, bytes: Vec<u8>, inline: bool) -> Response {
    let mut res = Response::new(Body::from(bytes));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/pdf"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    let disposition = format!(
        "{}; filename=\"{name}\"",
        if inline { "inline" } else { "attachment" }
    );
    if let Ok(v) = HeaderValue::from_str(&disposition) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    res
}

// ---------- Routes ----------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PdfBody {
    #[serde(flatten)]
    pub options: Options,
    /// Page thumbnails instead of the file.
    #[serde(default)]
    pub proof: bool,
    /// For professional printing: CMYK and bleed.
    #[serde(default)]
    pub print: bool,
}

/// `POST /api/boards/{id}/pdf`: the PDF, or `proof: true` for thumbnails.
pub async fn render_pdf(
    State(state): State<AppState>,
    Path(board_id): Path<String>,
    ApiJson(body): ApiJson<PdfBody>,
) -> AppResult<Response> {
    if body.proof {
        return Ok(Json(proof(&state, &board_id, body.options).await?).into_response());
    }
    let (name, bytes) = pdf(&state, &board_id, body.options, body.print).await?;
    Ok(pdf_response(&name, bytes, false))
}

#[derive(Deserialize)]
pub struct DocQuery {
    /// Comma-separated frame and card ids.
    #[serde(default)]
    items: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    recipient: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    part: Option<String>,
    /// `links` (default) or `embedded`.
    #[serde(default)]
    images: Option<String>,
}

/// `GET /api/boards/{id}/typeset`: the Typeset document, for joining with
/// others (e.g. Design's screens) before rendering.
pub async fn document(
    State(state): State<AppState>,
    Path(board_id): Path<String>,
    Query(q): Query<DocQuery>,
) -> AppResult<Json<Value>> {
    let images = match q.images.as_deref() {
        None | Some("links") => Images::Links,
        Some("embedded") => Images::Embedded,
        Some(_) => return Err(AppError::invalid("images", "links or embedded")),
    };
    let opts = Options {
        item_ids: q
            .items
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        title: q.title,
        subtitle: q.subtitle,
        recipient: q.recipient,
        author: q.author,
        date: q.date,
        part: q.part,
        ..Options::default()
    };
    let doc = build(&state, &board_id, opts, images).await?;
    Ok(Json(
        json!({ "document": doc.json, "warnings": doc.warnings }),
    ))
}

/// `GET /api/typeset/clients`: the clients Board's key may render for.
pub async fn clients(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let bytes = call(&state, "GET", "/api/clients", None).await?;
    Ok(Json(
        serde_json::from_slice(&bytes).map_err(|e| AppError::Internal(e.into()))?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkBody {
    #[serde(flatten)]
    pub options: Options,
    /// How long the link works, 1–90 days (default 7).
    #[serde(default)]
    pub days: Option<i64>,
}

/// A signed link to the board's PDF: `{public_origin}/p/{board}.pdf?o&exp&sig`.
/// It renders when opened, so it always shows the board as it is then.
pub fn pdf_link(
    state: &AppState,
    board_id: &str,
    opts: &Options,
    days: Option<i64>,
) -> AppResult<Value> {
    let days = days.unwrap_or(DEFAULT_LINK_DAYS);
    if !(1..=MAX_LINK_DAYS).contains(&days) {
        return Err(AppError::invalid(
            "days",
            format!("between 1 and {MAX_LINK_DAYS}"),
        ));
    }
    let expires = state.clock.now().plus(TimeDelta::days(days));
    let path = format!("/p/{board_id}.pdf");
    let mut params = BTreeMap::new();
    params.insert(
        "o".to_string(),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&opts_json(opts)).unwrap_or_default()),
    );
    let secret = state.config.secret_bytes()?;
    let signed = signing::signed_path(&secret, &path, params, expires.unix());
    Ok(
        json!({ "url": format!("{}{signed}", state.config.public_origin), "expires_at": expires.to_db() }),
    )
}

fn opts_json(o: &Options) -> Value {
    let mut v = json!({});
    if !o.item_ids.is_empty() {
        v["item_ids"] = json!(o.item_ids);
    }
    for (k, val) in [
        ("title", &o.title),
        ("subtitle", &o.subtitle),
        ("recipient", &o.recipient),
        ("author", &o.author),
        ("date", &o.date),
        ("eyebrow", &o.eyebrow),
        ("client", &o.client),
        ("part", &o.part),
    ] {
        if let Some(s) = val {
            v[k] = json!(s);
        }
    }
    v
}

/// `POST /api/boards/{id}/pdf-link`.
pub async fn create_link(
    State(state): State<AppState>,
    Path(board_id): Path<String>,
    ApiJson(body): ApiJson<LinkBody>,
) -> AppResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    boards::require_exists(&mut conn, &board_id).await?;
    typeset_config(&state)?;
    Ok(Json(pdf_link(&state, &board_id, &body.options, body.days)?))
}

/// `GET /p/{board}.pdf?o&exp&sig`: a signed PDF, no login needed.
pub async fn public_pdf(
    State(state): State<AppState>,
    Path(file): Path<String>,
    Query(params): Query<BTreeMap<String, String>>,
    _headers: HeaderMap,
) -> Response {
    let refused = |status: StatusCode, why: &str| (status, why.to_string()).into_response();
    let Some(board_id) = file.strip_suffix(".pdf") else {
        return refused(StatusCode::NOT_FOUND, "not found");
    };
    let Ok(secret) = state.config.secret_bytes() else {
        return refused(StatusCode::INTERNAL_SERVER_ERROR, "server misconfigured");
    };
    match signing::verify(
        &secret,
        &format!("/p/{file}"),
        &params,
        state.clock.now().unix(),
    ) {
        Ok(()) => {}
        Err(signing::Invalid::Expired) => {
            return refused(StatusCode::GONE, "this link has expired");
        }
        Err(_) => return refused(StatusCode::FORBIDDEN, "this link isn't valid"),
    }
    let opts: Options = params
        .get("o")
        .and_then(|o| URL_SAFE_NO_PAD.decode(o).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    match pdf(&state, board_id, opts, false).await {
        Ok((name, bytes)) => pdf_response(&name, bytes, true),
        Err(AppError::NotFound(_)) => refused(StatusCode::NOT_FOUND, "that board no longer exists"),
        Err(e) => e.into_response(),
    }
}
