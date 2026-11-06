//! Connector routing, the Rust twin of `web/src/canvas/routing.ts`.
//!
//! Both must draw the same paths; both run `fixtures/routing.json` in their
//! tests. The rules are documented at the top of routing.ts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "top" => Some(Self::Top),
            "right" => Some(Self::Right),
            "bottom" => Some(Self::Bottom),
            "left" => Some(Self::Left),
            _ => None,
        }
    }

    fn horizontal(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    fn normal(self) -> (f64, f64) {
        match self {
            Self::Top => (0.0, -1.0),
            Self::Right => (1.0, 0.0),
            Self::Bottom => (0.0, 1.0),
            Self::Left => (-1.0, 0.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routing {
    Straight,
    Elbow,
    Curved,
}

impl Routing {
    pub fn parse(s: &str) -> Self {
        match s {
            "straight" => Self::Straight,
            "curved" => Self::Curved,
            _ => Self::Elbow,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Curve {
    pub c1: Point,
    pub c2: Point,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Route {
    pub from: Side,
    pub to: Side,
    pub points: Vec<Point>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curve: Option<Curve>,
    pub label: Point,
}

fn centre(r: Rect) -> Point {
    Point {
        x: r.x + r.w / 2.0,
        y: r.y + r.h / 2.0,
    }
}

pub fn anchor_point(r: Rect, side: Side) -> Point {
    match side {
        Side::Top => Point {
            x: r.x + r.w / 2.0,
            y: r.y,
        },
        Side::Right => Point {
            x: r.x + r.w,
            y: r.y + r.h / 2.0,
        },
        Side::Bottom => Point {
            x: r.x + r.w / 2.0,
            y: r.y + r.h,
        },
        Side::Left => Point {
            x: r.x,
            y: r.y + r.h / 2.0,
        },
    }
}

/// The side of `r` facing `p`: horizontal first, ties count as horizontal.
fn facing(r: Rect, p: Point) -> Side {
    let c = centre(r);
    let (dx, dy) = (p.x - c.x, p.y - c.y);
    if dx.abs() >= dy.abs() {
        if dx >= 0.0 { Side::Right } else { Side::Left }
    } else if dy >= 0.0 {
        Side::Bottom
    } else {
        Side::Top
    }
}

fn sides(a: Rect, b: Rect, from: Option<Side>, to: Option<Side>) -> (Side, Side) {
    match (from, to) {
        (Some(f), Some(t)) => (f, t),
        (Some(f), None) => (f, facing(b, anchor_point(a, f))),
        (None, Some(t)) => (facing(a, anchor_point(b, t)), t),
        (None, None) => {
            let f = facing(a, centre(b));
            (f, f.opposite())
        }
    }
}

fn simplify(points: &[Point]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::new();
    for &p in points {
        if let Some(last) = out.last()
            && last.x == p.x
            && last.y == p.y
        {
            continue;
        }
        if out.len() >= 2 {
            let prev = out[out.len() - 2];
            let last = out[out.len() - 1];
            if (prev.x == last.x && last.x == p.x) || (prev.y == last.y && last.y == p.y) {
                let n = out.len();
                out[n - 1] = p;
                continue;
            }
        }
        out.push(p);
    }
    out
}

fn along(points: &[Point]) -> Point {
    let lengths: Vec<f64> = points
        .windows(2)
        .map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt())
        .collect();
    let mut remaining = lengths.iter().sum::<f64>() / 2.0;
    for (i, &len) in lengths.iter().enumerate() {
        if remaining <= len && len > 0.0 {
            let t = remaining / len;
            let (a, b) = (points[i], points[i + 1]);
            return Point {
                x: a.x + (b.x - a.x) * t,
                y: a.y + (b.y - a.y) * t,
            };
        }
        remaining -= len;
    }
    points[0]
}

/// Route a connector between `a` and `b`. `None` anchors mean `auto`.
pub fn route(a: Rect, b: Rect, from: Option<Side>, to: Option<Side>, routing: Routing) -> Route {
    let (from, to) = sides(a, b, from, to);
    let p1 = anchor_point(a, from);
    let p2 = anchor_point(b, to);

    if routing == Routing::Curved {
        let dist = ((p2.x - p1.x).powi(2) + (p2.y - p1.y).powi(2)).sqrt();
        let k = (dist / 3.0).max(40.0);
        let (n1, n2) = (from.normal(), to.normal());
        let c1 = Point {
            x: p1.x + n1.0 * k,
            y: p1.y + n1.1 * k,
        };
        let c2 = Point {
            x: p2.x + n2.0 * k,
            y: p2.y + n2.1 * k,
        };
        let label = Point {
            x: 0.125 * p1.x + 0.375 * c1.x + 0.375 * c2.x + 0.125 * p2.x,
            y: 0.125 * p1.y + 0.375 * c1.y + 0.375 * c2.y + 0.125 * p2.y,
        };
        return Route {
            from,
            to,
            points: vec![p1, p2],
            curve: Some(Curve { c1, c2 }),
            label,
        };
    }

    let points = match routing {
        Routing::Straight => vec![p1, p2],
        _ if from.horizontal() && to.horizontal() => {
            let mx = (p1.x + p2.x) / 2.0;
            simplify(&[p1, Point { x: mx, y: p1.y }, Point { x: mx, y: p2.y }, p2])
        }
        _ if !from.horizontal() && !to.horizontal() => {
            let my = (p1.y + p2.y) / 2.0;
            simplify(&[p1, Point { x: p1.x, y: my }, Point { x: p2.x, y: my }, p2])
        }
        _ if from.horizontal() => simplify(&[p1, Point { x: p2.x, y: p1.y }, p2]),
        _ => simplify(&[p1, Point { x: p1.x, y: p2.y }, p2]),
    };
    let label = along(&points);
    Route {
        from,
        to,
        points,
        curve: None,
        label,
    }
}

/// An SVG path `d` for a route.
pub fn path_data(r: &Route) -> String {
    let p1 = r.points[0];
    let p2 = r.points[r.points.len() - 1];
    if let Some(c) = &r.curve {
        return format!(
            "M{} {}C{} {} {} {} {} {}",
            n(p1.x),
            n(p1.y),
            n(c.c1.x),
            n(c.c1.y),
            n(c.c2.x),
            n(c.c2.y),
            n(p2.x),
            n(p2.y)
        );
    }
    r.points
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{}{} {}", if i == 0 { "M" } else { "L" }, n(p.x), n(p.y)))
        .collect()
}

/// A number for SVG output: at most two decimals, no trailing zeros.
pub fn n(v: f64) -> String {
    let s = format!("{:.2}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::Value;

    #[derive(Deserialize)]
    struct Fixtures {
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        name: String,
        from: Rect,
        to: Rect,
        from_anchor: String,
        to_anchor: String,
        routing: String,
        expect: Value,
    }

    #[test]
    fn matches_the_shared_fixtures() {
        let text = include_str!("../../../../fixtures/routing.json");
        let fixtures: Fixtures = serde_json::from_str(text).unwrap();
        assert!(!fixtures.cases.is_empty());
        for c in fixtures.cases {
            let r = route(
                c.from,
                c.to,
                Side::parse(&c.from_anchor),
                Side::parse(&c.to_anchor),
                Routing::parse(&c.routing),
            );
            assert_eq!(
                norm(serde_json::to_value(&r).unwrap()),
                norm(c.expect),
                "{}",
                c.name
            );
        }
    }

    /// JSON with every number as a float, so 100 and 100.0 compare equal.
    fn norm(v: Value) -> Value {
        match v {
            Value::Number(n) => serde_json::json!(n.as_f64().unwrap()),
            Value::Array(a) => Value::Array(a.into_iter().map(norm).collect()),
            Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, norm(v))).collect()),
            other => other,
        }
    }

    #[test]
    fn numbers_are_compact() {
        assert_eq!(n(100.0), "100");
        assert_eq!(n(137.5), "137.5");
        assert_eq!(n(1.0 / 3.0), "0.33");
        assert_eq!(n(-0.001), "0");
    }
}
