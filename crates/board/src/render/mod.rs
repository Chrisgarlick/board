//! Board draws its own images (spec section 8.1): frames, selections and
//! single items as SVG, and PNG from that SVG. No browser involved.

pub mod er;
pub mod png;
pub mod routing;
pub mod svg;
pub mod text;

use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

use sqlx::SqliteConnection;

use crate::error::{AppError, AppResult};
use crate::schema::{self, BoardTable};
use crate::service::connectors::{self, Connector};
use crate::service::items::{self, Item};
use crate::service::tables;
pub use svg::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Svg,
    Png,
}

impl Format {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "svg" => Some(Self::Svg),
            "png" => Some(Self::Png),
            _ => None,
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Svg => "image/svg+xml",
            Self::Png => "image/png",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Request {
    pub format: Format,
    /// PNG only: pixels per canvas unit, 1–4.
    pub scale: f32,
    pub theme: Theme,
    /// Draw a frame's title above it.
    pub title: bool,
    /// SVG only: write text as paths (the default for exports) or as `<text>`.
    pub text_as_paths: bool,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            format: Format::Png,
            scale: 2.0,
            theme: Theme::Light,
            title: true,
            text_as_paths: true,
        }
    }
}

/// Everything a render needs.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    /// The frame being rendered, if it's a frame.
    pub frame: Option<Item>,
    pub items: Vec<Item>,
    pub connectors: Vec<Connector>,
    /// The board's tables, for schema cards.
    pub tables: Vec<BoardTable>,
}

/// What to draw for an item: a frame brings its members and the connectors
/// between them; anything else is drawn alone (a schema card as its ER diagram).
pub async fn scene(conn: &mut SqliteConnection, item_id: &str) -> AppResult<Scene> {
    let target = items::get(conn, item_id).await?;
    let tables = tables::board_tables(conn, &target.board_id).await?;
    if target.kind != "frame" {
        return Ok(Scene {
            frame: None,
            items: vec![target],
            connectors: Vec::new(),
            tables,
        });
    }
    let members: Vec<Item> = items::list_live(conn, &target.board_id)
        .await?
        .into_iter()
        .filter(|i| i.frame_id.as_deref() == Some(item_id))
        .collect();
    let connectors = connectors::list_live(conn, &target.board_id).await?;
    Ok(Scene {
        frame: Some(target),
        items: members,
        connectors,
        tables,
    })
}

/// A selection: the given items (a frame brings its members) and the
/// connectors between them. All must be on one board.
pub async fn selection(
    conn: &mut SqliteConnection,
    board_id: &str,
    ids: &[String],
) -> AppResult<Scene> {
    if ids.is_empty() {
        return Err(AppError::invalid("item_ids", "nothing to render"));
    }
    let all = items::list_live(conn, board_id).await?;
    let wanted: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
    let picked: Vec<Item> = all
        .iter()
        .filter(|i| {
            wanted.contains(i.id.as_str())
                || i.frame_id.as_deref().is_some_and(|f| wanted.contains(f))
        })
        .cloned()
        .collect();
    if picked.len() < wanted.len() {
        return Err(AppError::invalid(
            "item_ids",
            "some of those items aren't on this board",
        ));
    }
    Ok(Scene {
        frame: None,
        items: picked,
        connectors: connectors::list_live(conn, board_id).await?,
        tables: tables::board_tables(conn, board_id).await?,
    })
}

/// The table ids a schema card shows.
pub fn card_tables(item: &Item) -> Vec<String> {
    item.content
        .get("table_ids")
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Draw and encode. Cached by the exact SVG and settings.
pub fn draw(cache: &Cache, scene: &Scene, req: Request) -> AppResult<Arc<Vec<u8>>> {
    let svg = match (&scene.frame, scene.items.as_slice()) {
        // A schema card on its own is its ER diagram.
        (None, [card]) if card.card_type.as_deref() == Some("schema") => {
            let view = schema::er::view(&card_tables(card), &scene.tables);
            let title = card.title.as_deref().filter(|_| req.title);
            er::render(&view, title, req.theme)
        }
        _ => svg::render(
            scene.frame.as_ref(),
            &scene.items,
            &scene.connectors,
            svg::Options {
                theme: req.theme,
                title: req.title,
                tables: &scene.tables,
            },
        ),
    };
    let mut h = DefaultHasher::new();
    svg.hash(&mut h);
    req.format.ext().hash(&mut h);
    req.scale.to_bits().hash(&mut h);
    req.text_as_paths.hash(&mut h);
    let key = h.finish();
    if let Some(hit) = cache.get(key) {
        return Ok(hit);
    }
    let bytes = match req.format {
        Format::Png => png::to_png(&svg, req.scale)?,
        Format::Svg if req.text_as_paths => png::text_to_paths(&svg)?.into_bytes(),
        Format::Svg => svg.into_bytes(),
    };
    Ok(cache.put(key, bytes))
}

/// Rendered images, most recently used kept, up to a byte budget.
#[derive(Clone)]
pub struct Cache {
    inner: Arc<Mutex<CacheInner>>,
    budget: usize,
}

struct CacheInner {
    map: HashMap<u64, Arc<Vec<u8>>>,
    order: VecDeque<u64>,
    bytes: usize,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(64 * 1024 * 1024)
    }
}

impl Cache {
    pub fn new(budget: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(CacheInner {
                map: HashMap::new(),
                order: VecDeque::new(),
                bytes: 0,
            })),
            budget,
        }
    }

    fn get(&self, key: u64) -> Option<Arc<Vec<u8>>> {
        let mut c = self.inner.lock().expect("cache lock");
        let hit = c.map.get(&key).cloned()?;
        c.order.retain(|k| *k != key);
        c.order.push_back(key);
        Some(hit)
    }

    fn put(&self, key: u64, bytes: Vec<u8>) -> Arc<Vec<u8>> {
        let value = Arc::new(bytes);
        let mut c = self.inner.lock().expect("cache lock");
        if value.len() > self.budget {
            return value;
        }
        c.bytes += value.len();
        if let Some(old) = c.map.insert(key, value.clone()) {
            c.bytes -= old.len();
            c.order.retain(|k| *k != key);
        }
        c.order.push_back(key);
        while c.bytes > self.budget {
            let Some(oldest) = c.order.pop_front() else {
                break;
            };
            if let Some(gone) = c.map.remove(&oldest) {
                c.bytes -= gone.len();
            }
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_keeps_recent_entries_within_budget() {
        let cache = Cache::new(10);
        cache.put(1, vec![0; 4]);
        cache.put(2, vec![0; 4]);
        assert!(cache.get(1).is_some(), "touch 1 so 2 is oldest");
        cache.put(3, vec![0; 4]);
        assert!(cache.get(2).is_none(), "least recently used went");
        assert!(cache.get(1).is_some() && cache.get(3).is_some());
        cache.put(4, vec![0; 50]);
        assert!(cache.get(4).is_none(), "too big to cache at all");
    }
}
