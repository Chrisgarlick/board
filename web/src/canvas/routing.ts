// Connector routing (spec section 8.1): where a connector leaves and enters
// its items, and the path between.
//
// This is a contract, not just UI code: the Rust renderer (phase 4) must draw
// the same paths. Both run fixtures/routing.json in their tests, so change the
// rules here, there and in the fixtures together.
//
// Rules:
// - An anchor is the middle of one side of an item's rectangle.
// - `auto` on both ends: compare the centres. If they're further apart
//   horizontally than vertically (ties count as horizontal), leave from the
//   right or left side facing the other item and enter on the opposite side;
//   otherwise use bottom/top the same way.
// - `auto` on one end only: that end picks the side of its item facing the
//   other end's anchor point, by the same horizontal-first comparison.
// - straight: one segment.
// - elbow: two horizontal sides meet at the midpoint x; two vertical sides at
//   the midpoint y; a horizontal and a vertical side make one corner.
//   Repeated or collinear points are removed.
// - curved: a cubic Bézier whose control points extend from each anchor along
//   its side's outward normal by max(40, distance / 3).
// - The label sits halfway along the path (by length; t = 0.5 for curves).

export type Side = "top" | "right" | "bottom" | "left";
export type AnchorSpec = "auto" | Side;
export type RoutingStyle = "straight" | "elbow" | "curved";

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Point {
  x: number;
  y: number;
}

export interface Route {
  from: Side;
  to: Side;
  /** The path's points: two for straight and curved, more for elbows. */
  points: Point[];
  /** Control points, for curved routing only. */
  curve?: { c1: Point; c2: Point };
  label: Point;
}

const NORMAL: Record<Side, Point> = {
  top: { x: 0, y: -1 },
  right: { x: 1, y: 0 },
  bottom: { x: 0, y: 1 },
  left: { x: -1, y: 0 },
};

const horizontal = (s: Side) => s === "left" || s === "right";

function centre(r: Rect): Point {
  return { x: r.x + r.w / 2, y: r.y + r.h / 2 };
}

export function anchorPoint(r: Rect, side: Side): Point {
  switch (side) {
    case "top":
      return { x: r.x + r.w / 2, y: r.y };
    case "right":
      return { x: r.x + r.w, y: r.y + r.h / 2 };
    case "bottom":
      return { x: r.x + r.w / 2, y: r.y + r.h };
    case "left":
      return { x: r.x, y: r.y + r.h / 2 };
  }
}

/** The side of `r` facing point `p`. */
function facing(r: Rect, p: Point): Side {
  const c = centre(r);
  const dx = p.x - c.x;
  const dy = p.y - c.y;
  if (Math.abs(dx) >= Math.abs(dy)) return dx >= 0 ? "right" : "left";
  return dy >= 0 ? "bottom" : "top";
}

function sides(a: Rect, b: Rect, fromA: AnchorSpec, toA: AnchorSpec): [Side, Side] {
  if (fromA !== "auto" && toA !== "auto") return [fromA, toA];
  if (fromA !== "auto") return [fromA, facing(b, anchorPoint(a, fromA))];
  if (toA !== "auto") return [facing(a, anchorPoint(b, toA)), toA];
  const from = facing(a, centre(b));
  const to: Side = { top: "bottom", bottom: "top", left: "right", right: "left" }[from] as Side;
  return [from, to];
}

function simplify(points: Point[]): Point[] {
  const out: Point[] = [];
  for (const p of points) {
    const last = out[out.length - 1];
    if (last && last.x === p.x && last.y === p.y) continue;
    const prev = out[out.length - 2];
    if (prev && last && ((prev.x === last.x && last.x === p.x) || (prev.y === last.y && last.y === p.y))) {
      out[out.length - 1] = p;
      continue;
    }
    out.push(p);
  }
  return out;
}

function along(points: Point[]): Point {
  const lengths: number[] = [];
  let total = 0;
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1] as Point;
    const b = points[i] as Point;
    const len = Math.hypot(b.x - a.x, b.y - a.y);
    lengths.push(len);
    total += len;
  }
  let remaining = total / 2;
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1] as Point;
    const b = points[i] as Point;
    const len = lengths[i - 1] as number;
    if (remaining <= len && len > 0) {
      const t = remaining / len;
      return { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
    }
    remaining -= len;
  }
  return points[0] as Point;
}

export function route(a: Rect, b: Rect, fromA: AnchorSpec, toA: AnchorSpec, routing: RoutingStyle): Route {
  const [from, to] = sides(a, b, fromA, toA);
  const p1 = anchorPoint(a, from);
  const p2 = anchorPoint(b, to);

  if (routing === "curved") {
    const k = Math.max(40, Math.hypot(p2.x - p1.x, p2.y - p1.y) / 3);
    const n1 = NORMAL[from];
    const n2 = NORMAL[to];
    const c1 = { x: p1.x + n1.x * k, y: p1.y + n1.y * k };
    const c2 = { x: p2.x + n2.x * k, y: p2.y + n2.y * k };
    const label = {
      x: 0.125 * p1.x + 0.375 * c1.x + 0.375 * c2.x + 0.125 * p2.x,
      y: 0.125 * p1.y + 0.375 * c1.y + 0.375 * c2.y + 0.125 * p2.y,
    };
    return { from, to, points: [p1, p2], curve: { c1, c2 }, label };
  }

  let points: Point[];
  if (routing === "straight") {
    points = [p1, p2];
  } else if (horizontal(from) && horizontal(to)) {
    const mx = (p1.x + p2.x) / 2;
    points = simplify([p1, { x: mx, y: p1.y }, { x: mx, y: p2.y }, p2]);
  } else if (!horizontal(from) && !horizontal(to)) {
    const my = (p1.y + p2.y) / 2;
    points = simplify([p1, { x: p1.x, y: my }, { x: p2.x, y: my }, p2]);
  } else if (horizontal(from)) {
    points = simplify([p1, { x: p2.x, y: p1.y }, p2]);
  } else {
    points = simplify([p1, { x: p1.x, y: p2.y }, p2]);
  }
  return { from, to, points, label: along(points) };
}

/** An SVG path `d` for a route. */
export function pathData(r: Route): string {
  const [p1, p2] = [r.points[0] as Point, r.points[r.points.length - 1] as Point];
  if (r.curve) {
    const { c1, c2 } = r.curve;
    return `M${p1.x} ${p1.y}C${c1.x} ${c1.y} ${c2.x} ${c2.y} ${p2.x} ${p2.y}`;
  }
  return r.points.map((p, i) => `${i ? "L" : "M"}${p.x} ${p.y}`).join("");
}
