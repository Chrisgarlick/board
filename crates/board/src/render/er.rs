//! ER diagrams for schema cards (spec section 8.1): each table a box of
//! columns, laid out with the flow layout (parents above children, by
//! foreign key), joined by crow's-foot connectors. Tables outside the card
//! that its tables reference are drawn as stubs (name only, dashed).

use std::fmt::Write as _;

use super::routing::{self, Point, Rect, Routing, n};
use super::svg::{Palette, Theme, palette, text_block};
use super::text::{self, Weight};
use crate::flow::layout::{self, Size};
use crate::flow::mermaid::Direction;
use crate::schema::er::View;
use crate::schema::{BoardTable, type_label};

const HEADER: f64 = 30.0;
const ROW: f64 = 22.0;
const PAD_X: f64 = 12.0;
const KEY_W: f64 = 26.0;
const MIN_W: f64 = 150.0;
const MARGIN: f64 = 24.0;
const NAME_SIZE: f64 = 14.0;
const COL_SIZE: f64 = 12.5;
const TYPE_SIZE: f64 = 11.5;
const TITLE_SPACE: f64 = 34.0;

/// A whole ER diagram as an SVG document, with an optional title above.
pub fn render(view: &View, title: Option<&str>, theme: Theme) -> String {
    let p = palette(theme);
    let (w, h, body) = body(view, theme);
    let top = if title.is_some() { TITLE_SPACE } else { 0.0 };
    let (tw, th) = (w + 2.0 * MARGIN, h + 2.0 * MARGIN + top);
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="Inter">"#,
        w = n(tw),
        h = n(th)
    );
    let _ = write!(
        out,
        r#"<rect width="100%" height="100%" fill="{}"/>"#,
        p.background
    );
    if let Some(title) = title {
        let fitted = text::fit(
            title,
            tw - 2.0 * MARGIN,
            22.0,
            16.0,
            16.0,
            1.2,
            Weight::SemiBold,
        );
        text_block(
            &mut out,
            &fitted.lines[..1.min(fitted.lines.len())],
            16.0,
            1.2,
            Weight::SemiBold,
            p.ink,
            MARGIN,
            "left",
            MARGIN - 4.0,
        );
    }
    let _ = write!(
        out,
        r#"<g transform="translate({} {})">{body}</g></svg>"#,
        n(MARGIN),
        n(MARGIN + top)
    );
    out
}

/// The diagram's size and its drawing, with its top-left at (0, 0), for
/// embedding (a schema card inside a frame export).
pub fn body(view: &View, theme: Theme) -> (f64, f64, String) {
    let p = palette(theme);
    if view.tables.is_empty() {
        return (MIN_W, ROW, String::new());
    }
    let sizes: Vec<Size> = (0..view.tables.len())
        .map(|i| box_size(view.tables[i], view.is_stub(i)))
        .collect();
    let edges: Vec<(usize, usize)> = view
        .relations
        .iter()
        .filter(|r| r.parent != r.child)
        .map(|r| (r.parent, r.child))
        .collect();
    let positions = layout::layout(&sizes, &edges, Direction::TopBottom);
    let rects: Vec<Rect> = positions
        .iter()
        .zip(&sizes)
        .map(|(&(x, y), s)| Rect {
            x,
            y,
            w: s.w,
            h: s.h,
        })
        .collect();
    // Self-references loop out to the right and over the top.
    let loops = view.relations.iter().any(|r| r.parent == r.child);
    let w = rects.iter().map(|r| r.x + r.w).fold(0.0, f64::max) + if loops { 34.0 } else { 0.0 };
    let h = rects.iter().map(|r| r.y + r.h).fold(0.0, f64::max);
    let shift = if loops { 28.0 } else { 0.0 };

    let mut out = String::new();
    if shift > 0.0 {
        let _ = write!(out, r#"<g transform="translate(0 {})">"#, n(shift));
    }
    let slots = slots(view, &rects);
    for (k, r) in view.relations.iter().enumerate() {
        let points = if r.parent == r.child {
            self_loop(rects[r.child])
        } else {
            let (child, parent) = slots[k];
            routing::route(child, parent, None, None, Routing::Elbow).points
        };
        let d: String = points
            .iter()
            .enumerate()
            .map(|(k, pt)| format!("{}{} {}", if k == 0 { "M" } else { "L" }, n(pt.x), n(pt.y)))
            .collect();
        let _ = write!(
            out,
            r#"<path d="{d}" fill="none" stroke="{}" stroke-width="1.5" stroke-linejoin="round"/>"#,
            p.connector
        );
        // Child end: zero or many / zero or one. Parent end: one / zero or one.
        end_marker(
            &mut out,
            &points,
            true,
            if r.one_to_one {
                End::ZeroOrOne
            } else {
                End::ZeroOrMany
            },
            p,
        );
        end_marker(
            &mut out,
            &points,
            false,
            if r.optional {
                End::ZeroOrOne
            } else {
                End::ExactlyOne
            },
            p,
        );
    }
    for (i, t) in view.tables.iter().enumerate() {
        draw_table(&mut out, t, rects[i], view.is_stub(i), p);
    }
    if shift > 0.0 {
        out.push_str("</g>");
    }
    (w.max(1.0), h + shift, out)
}

/// Where each relation attaches: a slice of each table's width, so several
/// foreign keys on one table (or between the same two tables) get their own
/// lines. Slices are ordered by where the other table sits, so lines don't cross.
fn slots(view: &View, rects: &[Rect]) -> Vec<(Rect, Rect)> {
    let centre = |r: &Rect| r.x + r.w / 2.0;
    let mut out: Vec<(Rect, Rect)> = view
        .relations
        .iter()
        .map(|r| (rects[r.child], rects[r.parent]))
        .collect();
    for t in 0..rects.len() {
        // (relation, this table is the child end, the other table's centre)
        let mut ends: Vec<(usize, bool, f64)> = view
            .relations
            .iter()
            .enumerate()
            .filter(|(_, r)| r.parent != r.child)
            .filter_map(|(k, r)| {
                if r.child == t {
                    Some((k, true, centre(&rects[r.parent])))
                } else if r.parent == t {
                    Some((k, false, centre(&rects[r.child])))
                } else {
                    None
                }
            })
            .collect();
        if ends.len() < 2 {
            continue;
        }
        ends.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));
        let r = rects[t];
        let slice = r.w / ends.len() as f64;
        for (i, (k, is_child, _)) in ends.into_iter().enumerate() {
            let part = Rect {
                x: r.x + i as f64 * slice,
                y: r.y,
                w: slice,
                h: r.h,
            };
            if is_child {
                out[k].0 = part;
            } else {
                out[k].1 = part;
            }
        }
    }
    out
}

fn has_keys(t: &BoardTable) -> bool {
    t.definition
        .columns
        .iter()
        .any(|c| c.primary || c.unique || c.references.is_some())
}

fn box_size(t: &BoardTable, stub: bool) -> Size {
    let name_w = text::width(&t.name, NAME_SIZE, Weight::SemiBold) + 2.0 * PAD_X;
    if stub {
        return Size {
            w: name_w.max(MIN_W * 0.7),
            h: HEADER,
        };
    }
    let key_w = if has_keys(t) { KEY_W } else { 0.0 };
    let rows = t
        .definition
        .columns
        .iter()
        .map(|c| {
            key_w
                + text::width(&c.name, COL_SIZE, Weight::Regular)
                + 18.0
                + text::width(&type_label(c), TYPE_SIZE, Weight::Regular)
                + 2.0 * PAD_X
        })
        .fold(0.0, f64::max);
    Size {
        w: name_w.max(rows).max(MIN_W).ceil(),
        h: HEADER + ROW * t.definition.columns.len().max(1) as f64 + 6.0,
    }
}

fn draw_table(out: &mut String, t: &BoardTable, r: Rect, stub: bool, p: &Palette) {
    let dash = if stub {
        r#" stroke-dasharray="5 4""#
    } else {
        ""
    };
    let _ = write!(
        out,
        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="3" fill="{}" stroke="{}"{dash}/>"#,
        n(r.x + 0.5),
        n(r.y + 0.5),
        n(r.w - 1.0),
        n(r.h - 1.0),
        if stub { p.surface } else { p.card },
        p.card_edge
    );
    let name_color = if stub { p.ink_soft } else { p.ink };
    text_block(
        out,
        std::slice::from_ref(&t.name),
        NAME_SIZE,
        1.2,
        Weight::SemiBold,
        name_color,
        r.x + PAD_X,
        "left",
        r.y + (HEADER - NAME_SIZE * 1.2) / 2.0,
    );
    if stub {
        return;
    }
    let _ = write!(
        out,
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}"/>"#,
        n(r.x),
        n(r.y + HEADER),
        n(r.x + r.w),
        n(r.y + HEADER),
        p.card_edge
    );
    let key_w = if has_keys(t) { KEY_W } else { 0.0 };
    if t.definition.columns.is_empty() {
        text_block(
            out,
            &["No columns yet".to_string()],
            COL_SIZE,
            1.2,
            Weight::Regular,
            p.ink_soft,
            r.x + PAD_X,
            "left",
            r.y + HEADER + 3.0 + (ROW - COL_SIZE * 1.2) / 2.0,
        );
    }
    for (k, c) in t.definition.columns.iter().enumerate() {
        let top = r.y + HEADER + 3.0 + k as f64 * ROW;
        let row_top = top + (ROW - COL_SIZE * 1.2) / 2.0;
        let key = if c.primary {
            "PK"
        } else if c.references.is_some() {
            "FK"
        } else if c.unique {
            "UK"
        } else {
            ""
        };
        if !key.is_empty() {
            text_block(
                out,
                &[key.to_string()],
                9.5,
                1.2,
                Weight::Bold,
                p.ink_soft,
                r.x + PAD_X,
                "left",
                top + (ROW - 9.5 * 1.2) / 2.0,
            );
        }
        let weight = if c.primary {
            Weight::SemiBold
        } else {
            Weight::Regular
        };
        text_block(
            out,
            std::slice::from_ref(&c.name),
            COL_SIZE,
            1.2,
            weight,
            p.ink,
            r.x + PAD_X + key_w,
            "left",
            row_top,
        );
        let ty = if c.nullable {
            format!("{}?", type_label(c))
        } else {
            type_label(c)
        };
        text_block(
            out,
            &[ty],
            TYPE_SIZE,
            1.2,
            Weight::Regular,
            p.ink_soft,
            r.x + r.w - PAD_X,
            "right",
            top + (ROW - TYPE_SIZE * 1.2) / 2.0,
        );
    }
}

/// A loop from the table's right side, out and over, into its top.
fn self_loop(r: Rect) -> Vec<Point> {
    let y = r.y + HEADER / 2.0;
    let x_in = r.x + r.w - 28.0;
    vec![
        Point { x: r.x + r.w, y },
        Point {
            x: r.x + r.w + 24.0,
            y,
        },
        Point {
            x: r.x + r.w + 24.0,
            y: r.y - 24.0,
        },
        Point {
            x: x_in,
            y: r.y - 24.0,
        },
        Point { x: x_in, y: r.y },
    ]
}

#[derive(Clone, Copy)]
enum End {
    ExactlyOne,
    ZeroOrOne,
    ZeroOrMany,
}

/// Crow's-foot notation at one end of a relation. `start` is the child end
/// (the path's first point); otherwise the parent end (its last point).
fn end_marker(out: &mut String, points: &[Point], start: bool, end: End, p: &Palette) {
    let (tip, prev) = if start {
        (points[0], points[1])
    } else {
        (points[points.len() - 1], points[points.len() - 2])
    };
    let len = ((tip.x - prev.x).powi(2) + (tip.y - prev.y).powi(2))
        .sqrt()
        .max(1e-9);
    // Unit vector pointing into the box, and its perpendicular.
    let (ux, uy) = ((tip.x - prev.x) / len, (tip.y - prev.y) / len);
    let (px, py) = (-uy, ux);
    let at = |d: f64| (tip.x - ux * d, tip.y - uy * d);
    let stroke = p.connector;
    let bar = |out: &mut String, d: f64| {
        let (cx, cy) = at(d);
        let _ = write!(
            out,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{stroke}" stroke-width="1.5"/>"#,
            n(cx + px * 6.0),
            n(cy + py * 6.0),
            n(cx - px * 6.0),
            n(cy - py * 6.0)
        );
    };
    let circle = |out: &mut String, d: f64| {
        let (cx, cy) = at(d);
        let _ = write!(
            out,
            r#"<circle cx="{}" cy="{}" r="4" fill="{}" stroke="{stroke}" stroke-width="1.5"/>"#,
            n(cx),
            n(cy),
            p.background
        );
    };
    match end {
        End::ExactlyOne => {
            bar(out, 6.0);
            bar(out, 11.0);
        }
        End::ZeroOrOne => {
            bar(out, 6.0);
            circle(out, 16.0);
        }
        End::ZeroOrMany => {
            let (bx, by) = at(12.0);
            let _ = write!(
                out,
                r#"<path d="M{} {}L{} {}M{} {}L{} {}M{} {}L{} {}" fill="none" stroke="{stroke}" stroke-width="1.5"/>"#,
                n(bx),
                n(by),
                n(tip.x + px * 7.0),
                n(tip.y + py * 7.0),
                n(bx),
                n(by),
                n(tip.x),
                n(tip.y),
                n(bx),
                n(by),
                n(tip.x - px * 7.0),
                n(tip.y - py * 7.0)
            );
            circle(out, 17.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{self, er};
    use serde_json::json;

    fn board() -> Vec<BoardTable> {
        let mut board: Vec<BoardTable> = Vec::new();
        for (name, def) in [
            (
                "users",
                json!({ "columns": [
                    { "name": "id", "type": "bigint", "primary": true },
                    { "name": "email", "type": "string", "length": 191, "unique": true },
                ] }),
            ),
            (
                "orders",
                json!({ "columns": [
                    { "name": "id", "type": "bigint", "primary": true },
                    { "name": "user_id", "type": "bigint", "references": { "table": "users" } },
                    { "name": "parent_id", "type": "bigint", "nullable": true, "references": { "table": "orders" } },
                    { "name": "note", "type": "text", "nullable": true },
                ] }),
            ),
        ] {
            let id = schema::table_id();
            let p = schema::prepare(&id, name, &def, None, &board).unwrap();
            board.push(BoardTable {
                id,
                name: name.into(),
                definition: p.definition,
                live: true,
            });
        }
        board
    }

    #[test]
    fn parents_sit_above_children() {
        let b = board();
        let view = er::view(&[b[1].id.clone(), b[0].id.clone()], &b);
        let svg = render(&view, Some("Data"), Theme::Light);
        assert!(svg.starts_with("<svg"));
        let y_of = |name: &str| {
            let at = svg.find(&format!(">{name}<")).unwrap();
            let tspan = svg[..at].rfind("y=\"").unwrap();
            svg[tspan + 3..]
                .split('"')
                .next()
                .unwrap()
                .parse::<f64>()
                .unwrap()
        };
        assert!(y_of("users") < y_of("orders"), "users is the parent");
        assert!(svg.contains(">string(191)<"));
        assert!(svg.contains(">text?<"), "nullable types are marked");
        assert!(svg.contains(">PK<") && svg.contains(">FK<") && svg.contains(">UK<"));
        assert!(svg.contains(">Data<"), "title");
        assert!(svg.contains("<circle"), "crow's-foot ends drawn");
    }

    #[test]
    fn tables_outside_the_card_are_stubs() {
        let b = board();
        let view = er::view(&[b[1].id.clone()], &b);
        let svg = render(&view, None, Theme::Dark);
        assert!(svg.contains(r#"stroke-dasharray="5 4""#), "dashed stub");
        assert!(!svg.contains(">email<"), "a stub shows no columns");
        let (w, h, _) = body(&view, Theme::Dark);
        assert!(w > 0.0 && h > 0.0);
    }

    #[test]
    fn an_empty_card_still_renders() {
        let view = er::view(&[], &[]);
        assert!(render(&view, None, Theme::Light).contains("</svg>"));
    }
}
