//! Frames, selections and single items as SVG (spec section 8.1).
//!
//! Geometry and colours match the canvas: shapes use the same outlines as
//! `web/src/canvas/shapes.tsx`, connectors the shared routing, colours the
//! palette in `web/src/styles.css`. Text is wrapped here (see `text.rs`) and
//! written as `<text>` in Inter; `png.rs` can turn it into paths.

use std::collections::HashSet;
use std::fmt::Write as _;

use serde_json::Value;

use super::routing::{self, Point, Rect, Route, Routing, Side, n};
use super::text::{self, Weight};
use crate::service::connectors::Connector;
use crate::service::items::Item;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub theme: Theme,
    /// Draw a frame's title above it (off when a heading carries it, e.g. in a PDF).
    pub title: bool,
}

struct Palette {
    background: &'static str,
    ink: &'static str,
    ink_soft: &'static str,
    card: &'static str,
    card_edge: &'static str,
    surface: &'static str,
    frame_fill: &'static str,
    frame_edge: &'static str,
    connector: &'static str,
    sticky: &'static str,
}

const LIGHT: Palette = Palette {
    background: "#ffffff",
    ink: "#1e2923",
    ink_soft: "#55635b",
    card: "#ffffff",
    card_edge: "#b9c5b6",
    surface: "#f3f6f2",
    frame_fill: "#f4f7f3",
    frame_edge: "#aebcab",
    connector: "#5d6a63",
    sticky: "#fbefb8",
};

const DARK: Palette = Palette {
    background: "#16201a",
    ink: "#e4ebe5",
    ink_soft: "#9aa99f",
    card: "#222c26",
    card_edge: "#34443a",
    surface: "#1b251f",
    frame_fill: "#1b251f",
    frame_edge: "#2b3a31",
    connector: "#a9b7ae",
    sticky: "#45391a",
};

/// (fill, line) for a palette colour, per theme. Mirrors styles.css.
fn color(name: &str, theme: Theme) -> Option<(&'static str, &'static str)> {
    let light = match name {
        "grey" => ("#eef1ee", "#5d6a63"),
        "blue" => ("#dde9f6", "#2f5f8f"),
        "green" => ("#dcefdc", "#2f6d3c"),
        "yellow" => ("#fbefb8", "#7a5d0e"),
        "orange" => ("#fbe2c8", "#9a5317"),
        "red" => ("#f8d9d6", "#a3322a"),
        "purple" => ("#e7def4", "#5f3d8e"),
        "pink" => ("#f7dbe8", "#9a3567"),
        _ => return None,
    };
    if theme == Theme::Light {
        return Some(light);
    }
    Some(match name {
        "grey" => ("#27312b", "#a9b7ae"),
        "blue" => ("#1f3346", "#8fb8e3"),
        "green" => ("#1f3a26", "#8ccf98"),
        "yellow" => ("#45391a", "#e8cf72"),
        "orange" => ("#44301d", "#f0b27a"),
        "red" => ("#45231f", "#f0948b"),
        "purple" => ("#33284a", "#c3a8ec"),
        _ => ("#432536", "#ee9cc3"),
    })
}

const TITLE_SPACE: f64 = 30.0;
const PADDING: f64 = 24.0;
const HEAD: f64 = 10.0;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn style_color(i: &Item) -> Option<&str> {
    i.style
        .as_ref()
        .and_then(|s| s.get("color"))
        .and_then(Value::as_str)
}

fn style_align(i: &Item, default: &'static str) -> String {
    i.style
        .as_ref()
        .and_then(|s| s.get("align"))
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

/// Render `items` (and the connectors among them). With `frame`, the image is
/// the frame's rectangle; otherwise the items' bounding box plus padding.
pub fn render(
    frame: Option<&Item>,
    items: &[Item],
    connectors: &[Connector],
    opts: Options,
) -> String {
    let p = if opts.theme == Theme::Light {
        &LIGHT
    } else {
        &DARK
    };

    // The image's box in canvas coordinates.
    let (bx, by, bw, bh) = match frame {
        Some(f) => {
            let title = if opts.title { TITLE_SPACE } else { 0.0 };
            (f.x, f.y - title, f.w, f.h + title)
        }
        None => {
            let min_x = items.iter().map(|i| i.x).fold(f64::MAX, f64::min);
            let min_y = items.iter().map(|i| i.y).fold(f64::MAX, f64::min);
            let max_x = items.iter().map(|i| i.x + i.w).fold(f64::MIN, f64::max);
            let max_y = items.iter().map(|i| i.y + i.h).fold(f64::MIN, f64::max);
            if items.is_empty() {
                (0.0, 0.0, 1.0, 1.0)
            } else {
                (
                    min_x - PADDING,
                    min_y - PADDING,
                    max_x - min_x + 2.0 * PADDING,
                    max_y - min_y + 2.0 * PADDING,
                )
            }
        }
    };

    let mut out = String::new();
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="Inter">"#,
        w = n(bw),
        h = n(bh)
    );
    let _ = write!(
        out,
        r#"<rect width="100%" height="100%" fill="{}"/>"#,
        p.background
    );
    let _ = write!(out, r#"<g transform="translate({} {})">"#, n(-bx), n(-by));

    if let Some(f) = frame {
        draw_frame(&mut out, f, p, opts);
    }
    // Frames inside a selection draw first, then connectors, then everything else.
    let mut ordered: Vec<&Item> = items.iter().collect();
    ordered.sort_by(|a, b| {
        (a.kind != "frame")
            .cmp(&(b.kind != "frame"))
            .then(a.z.cmp(&b.z))
            .then(a.id.cmp(&b.id))
    });
    for i in ordered.iter().filter(|i| i.kind == "frame") {
        draw_frame(&mut out, i, p, opts);
    }
    let ids: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    for c in connectors {
        if ids.contains(c.from_item.as_str()) && ids.contains(c.to_item.as_str()) {
            let a = items.iter().find(|i| i.id == c.from_item).expect("checked");
            let b = items.iter().find(|i| i.id == c.to_item).expect("checked");
            draw_connector(&mut out, c, a, b, p, opts.theme);
        }
    }
    for i in ordered.iter().filter(|i| i.kind != "frame") {
        match i.kind.as_str() {
            "shape" => draw_shape(&mut out, i, p, opts.theme),
            "sticky" => draw_sticky(&mut out, i, p, opts.theme),
            "text" => draw_text_item(&mut out, i, p, opts.theme),
            "card" => draw_card(&mut out, i, p, opts.theme),
            _ => {}
        }
    }
    out.push_str("</g></svg>");
    out
}

/// Wrapped lines as `<text>`, vertically centred on `cy` (or from `top`).
#[allow(clippy::too_many_arguments)]
fn text_block(
    out: &mut String,
    lines: &[String],
    size: f64,
    line_height: f64,
    weight: Weight,
    fill: &str,
    anchor_x: f64,
    align: &str,
    top: f64,
) {
    let anchor = match align {
        "left" => "start",
        "right" => "end",
        _ => "middle",
    };
    let _ = write!(
        out,
        r#"<text font-size="{}" font-weight="{}" fill="{}" text-anchor="{}">"#,
        n(size),
        weight.css(),
        fill,
        anchor
    );
    let lead = size * line_height;
    // Centre the ascent within each line box.
    let baseline = top + (lead - size) / 2.0 + text::ascent(size) * 0.92;
    for (k, line) in lines.iter().enumerate() {
        let _ = write!(
            out,
            r#"<tspan x="{}" y="{}">{}</tspan>"#,
            n(anchor_x),
            n(baseline + k as f64 * lead),
            esc(line)
        );
    }
    out.push_str("</text>");
}

fn shape_path(shape: &str, x: f64, y: f64, w: f64, h: f64) -> String {
    let i = 1.0;
    let (l, t, r, b) = (x + i, y + i, x + w - i, y + h - i);
    match shape {
        "rounded" => {
            let rx = (h / 2.0).min(36.0);
            format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}"/>"#,
                n(l),
                n(t),
                n(r - l),
                n(b - t),
                n(rx)
            )
        }
        "ellipse" => format!(
            r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}"/>"#,
            n(x + w / 2.0),
            n(y + h / 2.0),
            n(w / 2.0 - i),
            n(h / 2.0 - i)
        ),
        "diamond" => format!(
            r#"<polygon points="{},{} {},{} {},{} {},{}"/>"#,
            n(x + w / 2.0),
            n(t),
            n(r),
            n(y + h / 2.0),
            n(x + w / 2.0),
            n(b),
            n(l),
            n(y + h / 2.0)
        ),
        "parallelogram" => {
            let s = (w * 0.15).min(24.0);
            format!(
                r#"<polygon points="{},{} {},{} {},{} {},{}"/>"#,
                n(x + s),
                n(t),
                n(r),
                n(t),
                n(r - s),
                n(b),
                n(l),
                n(b)
            )
        }
        "cylinder" => {
            let ry = (h * 0.15).min(14.0);
            let rx = w / 2.0 - i;
            format!(
                r#"<path d="M{l} {a} A{rx} {ry} 0 0 1 {r} {a} V{bb} A{rx} {ry} 0 0 1 {l} {bb} Z"/><path fill="none" d="M{l} {a} A{rx} {ry} 0 0 0 {r} {a}"/>"#,
                l = n(l),
                r = n(r),
                a = n(y + ry),
                bb = n(b - ry),
                rx = n(rx),
                ry = n(ry)
            )
        }
        "document" => {
            let wave = (h * 0.12).min(10.0);
            format!(
                r#"<path d="M{l} {t} H{r} V{w1} Q{qx} {qy} {mx} {w1} T{l} {w1} Z"/>"#,
                l = n(l),
                t = n(t),
                r = n(r),
                w1 = n(b - wave),
                qx = n(x + w * 0.75),
                qy = n(b - wave * 2.2),
                mx = n(x + w / 2.0)
            )
        }
        _ => format!(
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="2"/>"#,
            n(l),
            n(t),
            n(r - l),
            n(b - t)
        ),
    }
}

fn draw_shape(out: &mut String, i: &Item, p: &Palette, theme: Theme) {
    let shape = str_of(&i.content, "shape");
    let (fill, line) = style_color(i)
        .and_then(|c| color(c, theme))
        .unwrap_or((p.card, p.ink));
    let _ = write!(
        out,
        r#"<g fill="{fill}" stroke="{line}" stroke-width="1.5">{}</g>"#,
        shape_path(shape, i.x, i.y, i.w, i.h)
    );

    let words = str_of(&i.content, "text");
    if words.is_empty() {
        return;
    }
    // Inner padding mirrors the canvas (diamonds and parallelograms need more room).
    let (px, py) = match shape {
        "diamond" => (24.0, 18.0),
        "parallelogram" => (22.0, 6.0),
        "cylinder" => (11.0, 16.0),
        _ => (11.0, 6.0),
    };
    let (bw, bh) = (i.w - 2.0 * px, i.h - 2.0 * py);
    let fitted = text::fit(words, bw, bh, 15.0, 9.0, 1.25, Weight::SemiBold);
    let align = style_align(i, "center");
    let x = match align.as_str() {
        "left" => i.x + px,
        "right" => i.x + i.w - px,
        _ => i.x + i.w / 2.0,
    };
    let block = fitted.lines.len() as f64 * fitted.size * 1.25;
    let top = i.y + (i.h - block) / 2.0;
    text_block(
        out,
        &fitted.lines,
        fitted.size,
        1.25,
        Weight::SemiBold,
        p.ink,
        x,
        &align,
        top,
    );
}

fn draw_sticky(out: &mut String, i: &Item, p: &Palette, theme: Theme) {
    let fill = style_color(i)
        .and_then(|c| color(c, theme))
        .map(|c| c.0)
        .unwrap_or(p.sticky);
    let _ = write!(
        out,
        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="2" fill="{fill}"/>"#,
        n(i.x),
        n(i.y),
        n(i.w),
        n(i.h)
    );
    let words = str_of(&i.content, "text");
    if words.is_empty() {
        return;
    }
    let fitted = text::fit(
        words,
        i.w - 26.0,
        i.h - 22.0,
        15.0,
        10.0,
        1.35,
        Weight::Regular,
    );
    text_block(
        out,
        &fitted.lines,
        fitted.size,
        1.35,
        Weight::Regular,
        p.ink,
        i.x + 13.0,
        "left",
        i.y + 11.0,
    );
}

fn draw_text_item(out: &mut String, i: &Item, p: &Palette, theme: Theme) {
    let words = str_of(&i.content, "text");
    if words.is_empty() {
        return;
    }
    let (size, weight) = match str_of(&i.content, "size") {
        "s" => (13.0, Weight::Regular),
        "l" => (22.0, Weight::SemiBold),
        "xl" => (32.0, Weight::Bold),
        _ => (16.0, Weight::Regular),
    };
    let fill = style_color(i)
        .and_then(|c| color(c, theme))
        .map(|c| c.1)
        .unwrap_or(p.ink);
    let align = style_align(i, "left");
    let lines = text::wrap(words, i.w - 6.0, size, weight);
    let x = match align.as_str() {
        "center" => i.x + i.w / 2.0,
        "right" => i.x + i.w - 3.0,
        _ => i.x + 3.0,
    };
    text_block(out, &lines, size, 1.2, weight, fill, x, &align, i.y + 2.0);
}

fn draw_frame(out: &mut String, f: &Item, p: &Palette, opts: Options) {
    let (fill, edge) = style_color(f)
        .and_then(|c| color(c, opts.theme))
        .unwrap_or((p.frame_fill, p.frame_edge));
    let _ = write!(
        out,
        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="4" fill="{fill}" stroke="{edge}" stroke-width="1"/>"#,
        n(f.x + 0.5),
        n(f.y + 0.5),
        n(f.w - 1.0),
        n(f.h - 1.0)
    );
    if opts.title
        && let Some(title) = f.title.as_deref().filter(|t| !t.is_empty())
    {
        let fitted = text::fit(title, f.w, 22.0, 15.0, 15.0, 1.2, Weight::SemiBold);
        text_block(
            out,
            &fitted.lines[..1.min(fitted.lines.len())],
            15.0,
            1.2,
            Weight::SemiBold,
            p.ink,
            f.x + 8.0,
            "left",
            f.y - 26.0,
        );
    }
}

fn draw_card(out: &mut String, i: &Item, p: &Palette, theme: Theme) {
    let accent = style_color(i).and_then(|c| color(c, theme)).map(|c| c.1);
    let _ = write!(
        out,
        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="3" fill="{}" stroke="{}"/>"#,
        n(i.x + 0.5),
        n(i.y + 0.5),
        n(i.w - 1.0),
        n(i.h - 1.0),
        p.card,
        p.card_edge
    );
    let header = 36.0;
    let _ = write!(
        out,
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}"/>"#,
        n(i.x),
        n(i.y + header),
        n(i.x + i.w),
        n(i.y + header),
        p.card_edge
    );
    if let Some(a) = accent {
        let _ = write!(
            out,
            r#"<rect x="{}" y="{}" width="3" height="{}" fill="{a}"/>"#,
            n(i.x + 0.5),
            n(i.y + 0.5),
            n(header)
        );
    }
    let title = i.title.as_deref().unwrap_or("");
    let fitted = text::fit(title, i.w - 24.0, 20.0, 15.0, 15.0, 1.2, Weight::SemiBold);
    text_block(
        out,
        &fitted.lines[..1.min(fitted.lines.len())],
        15.0,
        1.2,
        Weight::SemiBold,
        p.ink,
        i.x + 12.0,
        "left",
        i.y + 9.0,
    );

    // A preview of the body.
    let body_top = i.y + header + 10.0;
    let body_h = (i.h - header - 18.0).max(0.0);
    match i.card_type.as_deref() {
        Some("note") => {
            let plain: String = str_of(&i.content, "markdown")
                .lines()
                .map(|l| {
                    l.trim_start_matches(['#', '>', '-', '*', ' '])
                        .replace("**", "")
                        .replace('`', "")
                })
                .collect::<Vec<_>>()
                .join("\n");
            let fitted = text::fit(
                plain.trim(),
                i.w - 26.0,
                body_h,
                13.0,
                13.0,
                1.45,
                Weight::Regular,
            );
            text_block(
                out,
                &fitted.lines,
                13.0,
                1.45,
                Weight::Regular,
                p.ink_soft,
                i.x + 13.0,
                "left",
                body_top,
            );
        }
        Some("checklist") => {
            let row = 22.0;
            let items = i
                .content
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let rows = ((body_h / row).floor() as usize).max(1);
            for (k, entry) in items.iter().take(rows).enumerate() {
                let y = body_top + k as f64 * row;
                let done = entry.get("done").and_then(Value::as_bool).unwrap_or(false);
                let _ = write!(
                    out,
                    r#"<rect x="{}" y="{}" width="11" height="11" rx="2" fill="{}" stroke="{}"/>"#,
                    n(i.x + 13.0),
                    n(y + 4.0),
                    if done { p.ink } else { "none" },
                    p.ink_soft
                );
                let label = text::fit(
                    str_of(entry, "text"),
                    i.w - 50.0,
                    18.0,
                    13.0,
                    13.0,
                    1.4,
                    Weight::Regular,
                );
                let color = if done { p.ink_soft } else { p.ink };
                text_block(
                    out,
                    &label.lines[..1.min(label.lines.len())],
                    13.0,
                    1.4,
                    Weight::Regular,
                    color,
                    i.x + 32.0,
                    "left",
                    y,
                );
            }
        }
        _ => {}
    }
}

fn rect(i: &Item) -> Rect {
    Rect {
        x: i.x,
        y: i.y,
        w: i.w,
        h: i.h,
    }
}

fn draw_connector(out: &mut String, c: &Connector, a: &Item, b: &Item, p: &Palette, theme: Theme) {
    let style = c.style.as_ref();
    let get = |k: &str| style.and_then(|s| s.get(k)).and_then(Value::as_str);
    let routing = Routing::parse(get("routing").unwrap_or("elbow"));
    let r = routing::route(
        rect(a),
        rect(b),
        Side::parse(&c.from_anchor),
        Side::parse(&c.to_anchor),
        routing,
    );
    let line = get("color")
        .and_then(|n| color(n, theme))
        .map(|c| c.1)
        .unwrap_or(p.connector);
    let dash = match get("line") {
        Some("dashed") => r#" stroke-dasharray="7 5""#,
        Some("dotted") => r#" stroke-dasharray="1.5 4" stroke-linecap="round""#,
        _ => "",
    };
    let _ = write!(
        out,
        r#"<path d="{}" fill="none" stroke="{line}" stroke-width="1.75" stroke-linejoin="round"{dash}/>"#,
        routing::path_data(&r)
    );
    head(out, &r, true, get("start").unwrap_or("none"), line);
    head(out, &r, false, get("end").unwrap_or("arrow"), line);

    if let Some(label) = c.label.as_deref().filter(|l| !l.is_empty()) {
        let size = 13.0;
        let w = text::width(label, size, Weight::Regular) + 14.0;
        let h = 20.0;
        let _ = write!(
            out,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="3" fill="{}" stroke="{}"/>"#,
            n(r.label.x - w / 2.0),
            n(r.label.y - h / 2.0),
            n(w),
            n(h),
            p.surface,
            p.frame_edge
        );
        text_block(
            out,
            &[label.to_string()],
            size,
            1.0,
            Weight::Regular,
            p.ink,
            r.label.x,
            "center",
            r.label.y - size / 2.0,
        );
    }
}

/// Arrowheads as plain paths, matching ConnectorLayer.tsx.
fn head(out: &mut String, r: &Route, start: bool, kind: &str, line: &str) {
    if kind == "none" {
        return;
    }
    let pts = &r.points;
    let (tip, prev): (Point, Point) = if start {
        (pts[0], r.curve.as_ref().map(|c| c.c1).unwrap_or(pts[1]))
    } else {
        (
            pts[pts.len() - 1],
            r.curve.as_ref().map(|c| c.c2).unwrap_or(pts[pts.len() - 2]),
        )
    };
    let len = ((tip.x - prev.x).powi(2) + (tip.y - prev.y).powi(2))
        .sqrt()
        .max(1e-9);
    let (ux, uy) = ((tip.x - prev.x) / len, (tip.y - prev.y) / len);
    if kind == "dot" {
        let _ = write!(
            out,
            r#"<circle cx="{}" cy="{}" r="4" fill="{line}"/>"#,
            n(tip.x),
            n(tip.y)
        );
        return;
    }
    let (bx, by) = (tip.x - ux * HEAD, tip.y - uy * HEAD);
    let (px, py) = (-uy * HEAD / 2.0, ux * HEAD / 2.0);
    let d = format!(
        "M{} {}L{} {}L{} {}",
        n(bx + px),
        n(by + py),
        n(tip.x),
        n(tip.y),
        n(bx - px),
        n(by - py)
    );
    if kind == "open_arrow" {
        let _ = write!(
            out,
            r#"<path d="{d}" fill="none" stroke="{line}" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>"#
        );
    } else {
        let _ = write!(
            out,
            r#"<path d="{d}Z" fill="{line}" stroke="{line}" stroke-linejoin="round"/>"#
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(id: &str, kind: &str, content: Value, x: f64, y: f64, w: f64, h: f64) -> Item {
        serde_json::from_value(json!({
            "id": id, "board_id": "b", "kind": kind, "card_type": null, "frame_id": null,
            "title": null, "content": content, "style": null, "x": x, "y": y, "w": w, "h": h, "z": 1,
            "locked": false, "version": 1, "created_by": "user", "updated_by": "user",
            "content_updated_at": "t", "created_at": "t", "updated_at": "t"
        }))
        .unwrap()
    }

    fn connector(id: &str, from: &str, to: &str, label: Option<&str>, style: Value) -> Connector {
        serde_json::from_value(json!({
            "id": id, "board_id": "b", "from_item": from, "to_item": to,
            "from_anchor": "auto", "to_anchor": "auto", "label": label,
            "style": if style.is_null() { Value::Null } else { style },
            "created_by": "user", "created_at": "t", "updated_at": "t"
        }))
        .unwrap()
    }

    fn sample() -> (Item, Vec<Item>, Vec<Connector>) {
        let mut frame = item("f", "frame", json!({}), 0.0, 0.0, 600.0, 400.0);
        frame.title = Some("Checkout".into());
        let mut basket = item(
            "a",
            "shape",
            json!({ "shape": "rounded", "text": "Basket" }),
            40.0,
            40.0,
            160.0,
            72.0,
        );
        basket.style = Some(json!({ "color": "blue" }));
        let auth = item(
            "b",
            "shape",
            json!({ "shape": "diamond", "text": "Logged in & ready?" }),
            45.0,
            200.0,
            150.0,
            110.0,
        );
        let sticky = item(
            "s",
            "sticky",
            json!({ "text": "Guest checkout <later>" }),
            380.0,
            40.0,
            180.0,
            180.0,
        );
        let conns = vec![
            connector("c1", "a", "b", Some("next"), Value::Null),
            connector(
                "c2",
                "b",
                "s",
                None,
                json!({ "line": "dashed", "end": "open_arrow", "routing": "straight" }),
            ),
        ];
        (frame, vec![basket, auth, sticky], conns)
    }

    #[test]
    fn a_frame_renders_deterministically() {
        let (frame, items, conns) = sample();
        let svg = render(
            Some(&frame),
            &items,
            &conns,
            Options {
                theme: Theme::Light,
                title: true,
            },
        );
        insta::assert_snapshot!(svg);
    }

    #[test]
    fn frame_size_includes_the_title_only_when_asked() {
        let (frame, items, conns) = sample();
        let with = render(
            Some(&frame),
            &items,
            &conns,
            Options {
                theme: Theme::Light,
                title: true,
            },
        );
        let without = render(
            Some(&frame),
            &items,
            &conns,
            Options {
                theme: Theme::Light,
                title: false,
            },
        );
        assert!(with.contains(r#"height="430""#));
        assert!(without.contains(r#"height="400""#));
        assert!(with.contains(">Checkout<") && !without.contains(">Checkout<"));
    }

    #[test]
    fn text_is_escaped_and_colours_follow_the_theme() {
        let (frame, items, conns) = sample();
        let dark = render(
            Some(&frame),
            &items,
            &conns,
            Options {
                theme: Theme::Dark,
                title: true,
            },
        );
        assert!(dark.contains("Logged in &amp;"));
        assert!(dark.contains("&lt;later&gt;"));
        assert!(dark.contains("#1f3346"), "dark blue fill");
        assert!(!dark.contains("#dde9f6"), "no light blue");
    }

    #[test]
    fn a_selection_is_cropped_to_its_items() {
        let a = item(
            "a",
            "shape",
            json!({ "shape": "rect", "text": "A" }),
            100.0,
            100.0,
            160.0,
            72.0,
        );
        let svg = render(
            None,
            &[a],
            &[],
            Options {
                theme: Theme::Light,
                title: true,
            },
        );
        assert!(svg.contains(r#"width="208" height="120""#), "{svg}");
    }
}
