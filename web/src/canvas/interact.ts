// Pointer interactions on the canvas: drag (with grid snap and alignment
// guides), resize, marquee selection and drawing connectors.
//
// Live drag state is two signals: which items are moving (changes at the
// start and end of a drag) and by how much (changes on every pointer move).
// Items only read the offset when they're in the moving set, so a drag
// re-renders the moving items and their connectors, nothing else.

import { batch, signal } from "@preact/signals";

import type { Item, Side } from "../types";
import {
  create,
  editingId,
  item,
  items,
  membersOf,
  place,
  select,
  selection,
  tool,
  type NewItem,
} from "../store";
import { anchorPoint, type Point, type Rect } from "./routing";
import { toCanvas, view } from "./viewport";

const GRID = 10;
const DRAG_THRESHOLD = 3;
/** How close (in screen pixels) an edge must be to snap to a guide. */
const GUIDE_SNAP = 6;
/** How close (in screen pixels) to an anchor dot counts as dropping on it. */
const ANCHOR_SNAP = 14;

export const moving = signal<ReadonlySet<string>>(new Set());
export const offset = signal<Point>({ x: 0, y: 0 });
export const resizing = signal<string | null>(null);
export const resizeTo = signal<{ w: number; h: number }>({ w: 0, h: 0 });
export const guides = signal<{ x: number[]; y: number[] }>({ x: [], y: [] });
export const marquee = signal<{ x: number; y: number; w: number; h: number } | null>(null);
/** A connector being drawn: from an item's anchor (or centre) to the pointer. */
export const connecting = signal<{ from: string; side: Side | "auto"; to: Point } | null>(null);
/** The item under the pointer while connecting, for anchor dots. */
export const hoverTarget = signal<string | null>(null);

/** An item's rectangle, including any live drag or resize. */
export function liveRect(i: Item): Rect {
  let r: Rect = { x: i.x, y: i.y, w: i.w, h: i.h };
  if (moving.value.has(i.id)) {
    const o = offset.value;
    r = { ...r, x: r.x + o.x, y: r.y + o.y };
  }
  if (resizing.value === i.id) r = { ...r, ...resizeTo.value };
  return r;
}

function matRect(): DOMRect | undefined {
  return document.querySelector(".mat")?.getBoundingClientRect();
}

function canvasPoint(e: PointerEvent | MouseEvent): Point {
  const r = matRect();
  return toCanvas(e.clientX - (r?.left ?? 0), e.clientY - (r?.top ?? 0));
}

function onPointer(move: (e: PointerEvent) => void, end: (e: PointerEvent) => void): void {
  function up(e: PointerEvent) {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", up);
    document.body.classList.remove("is-dragging");
    end(e);
  }
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", up);
}

// ---------- Selecting and dragging ----------

/**
 * Press on an item. Handles selection (shift toggles) and, if the pointer
 * moves, drags the whole selection. Frames bring their members along.
 */
export function pressItem(e: PointerEvent, id: string): void {
  if (e.button !== 0) return;
  e.stopPropagation();
  if (tool.value === "connector") {
    startConnect(e, id, "auto");
    return;
  }
  if (editingId.value && editingId.value !== id) editingId.value = null;

  const wasSelected = selection.value.has(id);
  if (e.shiftKey) {
    const next = new Set(selection.value);
    if (wasSelected) next.delete(id);
    else next.add(id);
    select(next);
    if (wasSelected) return;
  } else if (!wasSelected) {
    select([id]);
  }

  const start = { x: e.clientX, y: e.clientY };
  const selected = [...selection.value].map(item).filter((i): i is Item => !!i && !i.locked);
  if (selected.length === 0) return;
  const selectedFrames = new Set(selected.filter((i) => i.kind === "frame").map((i) => i.id));
  // Items to commit: those not already carried by a selected frame.
  const roots = selected.filter((i) => !i.frame_id || !selectedFrames.has(i.frame_id));
  // Items that move on screen: roots plus every member of a moving frame.
  const visual = new Set(roots.map((i) => i.id));
  for (const f of selectedFrames) for (const m of membersOf(f)) visual.add(m.id);
  const primary = item(id) ?? roots[0]!;
  const others = [...items.values()].map((s) => s.item.value).filter((i) => !visual.has(i.id) && i.kind !== "frame");
  let dragging = false;

  onPointer(
    (ev) => {
      const zoom = view.value.zoom;
      if (!dragging) {
        if (Math.hypot(ev.clientX - start.x, ev.clientY - start.y) < DRAG_THRESHOLD) return;
        dragging = true;
        document.body.classList.add("is-dragging");
        moving.value = visual;
      }
      let dx = (ev.clientX - start.x) / zoom;
      let dy = (ev.clientY - start.y) / zoom;
      const found: { x: number[]; y: number[] } = { x: [], y: [] };
      if (!ev.altKey) {
        dx = Math.round((primary.x + dx) / GRID) * GRID - primary.x;
        dy = Math.round((primary.y + dy) / GRID) * GRID - primary.y;
        const snapped = snapToGuides({ ...primary, x: primary.x + dx, y: primary.y + dy }, others, GUIDE_SNAP / zoom);
        dx += snapped.dx;
        dy += snapped.dy;
        found.x = snapped.x;
        found.y = snapped.y;
      }
      batch(() => {
        offset.value = { x: dx, y: dy };
        guides.value = found;
      });
    },
    () => {
      if (!dragging) {
        // A plain click on an item in a multi-selection selects just it.
        if (!e.shiftKey && selection.value.size > 1) select([id]);
        return;
      }
      const o = offset.value;
      const changes = roots.map((i) => ({ id: i.id, x: i.x + o.x, y: i.y + o.y }));
      batch(() => {
        // Members of moving frames: show them where the server will put them.
        for (const vid of visual) {
          if (roots.some((r) => r.id === vid)) continue;
          const s = items.get(vid);
          if (s) s.item.value = { ...s.item.value, x: s.item.value.x + o.x, y: s.item.value.y + o.y };
        }
        moving.value = new Set();
        offset.value = { x: 0, y: 0 };
        guides.value = { x: [], y: [] };
      });
      if (o.x !== 0 || o.y !== 0) void place(changes);
    },
  );
}

/** Snap a moving rectangle's edges and centre to other items'. */
function snapToGuides(
  r: Rect,
  others: Item[],
  within: number,
): { dx: number; dy: number; x: number[]; y: number[] } {
  const xs = [r.x, r.x + r.w / 2, r.x + r.w];
  const ys = [r.y, r.y + r.h / 2, r.y + r.h];
  let best = { dx: Infinity, dy: Infinity };
  for (const o of others) {
    const oxs = [o.x, o.x + o.w / 2, o.x + o.w];
    const oys = [o.y, o.y + o.h / 2, o.y + o.h];
    for (const a of xs) for (const b of oxs) if (Math.abs(b - a) < Math.abs(best.dx)) best = { ...best, dx: b - a };
    for (const a of ys) for (const b of oys) if (Math.abs(b - a) < Math.abs(best.dy)) best = { ...best, dy: b - a };
  }
  const dx = Math.abs(best.dx) <= within ? best.dx : 0;
  const dy = Math.abs(best.dy) <= within ? best.dy : 0;
  // Lines to show: every edge or centre that lines up after snapping.
  const lineX = new Set<number>();
  const lineY = new Set<number>();
  for (const o of others) {
    for (const a of xs) for (const b of [o.x, o.x + o.w / 2, o.x + o.w]) if (Math.abs(a + dx - b) < 0.5) lineX.add(b);
    for (const a of ys) for (const b of [o.y, o.y + o.h / 2, o.y + o.h]) if (Math.abs(a + dy - b) < 0.5) lineY.add(b);
  }
  return { dx, dy, x: [...lineX], y: [...lineY] };
}

// ---------- Resize ----------

export function pressResize(e: PointerEvent, id: string, minW = 40, minH = 24): void {
  if (e.button !== 0) return;
  e.stopPropagation();
  const i = item(id);
  if (!i || i.locked) return;
  const start = { x: e.clientX, y: e.clientY };
  batch(() => {
    resizing.value = id;
    resizeTo.value = { w: i.w, h: i.h };
  });
  document.body.classList.add("is-dragging");
  onPointer(
    (ev) => {
      const zoom = view.value.zoom;
      let w = Math.max(minW, i.w + (ev.clientX - start.x) / zoom);
      let h = Math.max(minH, i.h + (ev.clientY - start.y) / zoom);
      if (!ev.altKey) {
        w = Math.max(minW, Math.round((i.x + w) / GRID) * GRID - i.x);
        h = Math.max(minH, Math.round((i.y + h) / GRID) * GRID - i.y);
      }
      resizeTo.value = { w: Math.round(w), h: Math.round(h) };
    },
    () => {
      const size = resizeTo.value;
      resizing.value = null;
      if (size.w !== i.w || size.h !== i.h) void place([{ id, w: size.w, h: size.h }]);
    },
  );
}

// ---------- Marquee ----------

/** Drag on empty canvas with the select tool: select what the box encloses. */
export function pressMarquee(e: PointerEvent): void {
  const start = canvasPoint(e);
  const additive = e.shiftKey;
  const base = additive ? new Set(selection.value) : new Set<string>();
  if (!additive) select([]);
  onPointer(
    (ev) => {
      const p = canvasPoint(ev);
      const box = {
        x: Math.min(start.x, p.x),
        y: Math.min(start.y, p.y),
        w: Math.abs(p.x - start.x),
        h: Math.abs(p.y - start.y),
      };
      marquee.value = box;
      const inside = [...items.values()]
        .map((s) => s.item.value)
        .filter((i) => i.x >= box.x && i.y >= box.y && i.x + i.w <= box.x + box.w && i.y + i.h <= box.y + box.h)
        .map((i) => i.id);
      select([...base, ...inside]);
    },
    () => {
      marquee.value = null;
    },
  );
}

// ---------- Creating by click or drag ----------

const SHAPE_FOR_TOOL = { rect: "rect", diamond: "diamond", ellipse: "ellipse" } as const;

export const DEFAULT_SIZE: Record<string, { w: number; h: number }> = {
  rect: { w: 160, h: 72 },
  rounded: { w: 160, h: 72 },
  parallelogram: { w: 160, h: 72 },
  document: { w: 160, h: 72 },
  diamond: { w: 150, h: 110 },
  ellipse: { w: 150, h: 90 },
  cylinder: { w: 130, h: 100 },
  sticky: { w: 180, h: 180 },
  text: { w: 240, h: 48 },
  frame: { w: 800, h: 600 },
};

/** Press on empty canvas with a creation tool: click places at default size, drag sets the size. */
export function pressCreate(e: PointerEvent): void {
  const t = tool.value;
  const start = canvasPoint(e);
  let end = start;
  const box = signalBox(start);
  onPointer(
    (ev) => {
      end = canvasPoint(ev);
      box(end);
    },
    () => {
      marquee.value = null;
      const dragged = Math.abs(end.x - start.x) > 8 && Math.abs(end.y - start.y) > 8;
      const key = t in SHAPE_FOR_TOOL ? SHAPE_FOR_TOOL[t as keyof typeof SHAPE_FOR_TOOL] : t;
      const size = DEFAULT_SIZE[key] ?? { w: 160, h: 72 };
      const rect = dragged
        ? { x: Math.min(start.x, end.x), y: Math.min(start.y, end.y), w: Math.abs(end.x - start.x), h: Math.abs(end.y - start.y) }
        : { x: start.x - size.w / 2, y: start.y - size.h / 2, ...size };
      const snapped = { ...rect, x: Math.round(rect.x / GRID) * GRID, y: Math.round(rect.y / GRID) * GRID };
      void createFromTool(t, snapped);
    },
  );
}

function signalBox(start: Point) {
  return (p: Point) => {
    marquee.value = {
      x: Math.min(start.x, p.x),
      y: Math.min(start.y, p.y),
      w: Math.abs(p.x - start.x),
      h: Math.abs(p.y - start.y),
    };
  };
}

export function newItemFor(t: string, rect: Rect): NewItem | null {
  const base = { x: rect.x, y: rect.y, w: Math.round(rect.w), h: Math.round(rect.h) };
  if (t in SHAPE_FOR_TOOL) {
    return { kind: "shape", content: { shape: SHAPE_FOR_TOOL[t as keyof typeof SHAPE_FOR_TOOL], text: "" }, ...base };
  }
  if (t === "sticky") return { kind: "sticky", content: { text: "" }, style: { color: "yellow" }, ...base };
  if (t === "text") return { kind: "text", content: { text: "", size: "m" }, ...base };
  if (t === "frame") return { kind: "frame", title: "Frame", content: {}, ...base };
  return null;
}

async function createFromTool(t: string, rect: Rect): Promise<void> {
  const n = newItemFor(t, rect);
  if (!n) return;
  const [id] = await create([n]);
  tool.value = "select";
  if (id) {
    select([id]);
    editingId.value = id;
  }
}

// ---------- Connecting ----------

function nearestAnchor(i: Item, p: Point, zoom: number): Side | null {
  const r = { x: i.x, y: i.y, w: i.w, h: i.h };
  let best: { side: Side; d: number } | null = null;
  for (const side of ["top", "right", "bottom", "left"] as Side[]) {
    const a = anchorPoint(r, side);
    const d = Math.hypot(a.x - p.x, a.y - p.y) * zoom;
    if (d <= ANCHOR_SNAP && (!best || d < best.d)) best = { side, d };
  }
  return best?.side ?? null;
}

function itemAt(clientX: number, clientY: number, except: string): Item | undefined {
  for (const el of document.elementsFromPoint(clientX, clientY)) {
    const id = (el as HTMLElement).closest?.("[data-item]")?.getAttribute("data-item");
    if (id && id !== except) return item(id);
  }
  return undefined;
}

/**
 * Draw a connector from `fromId`. Dropping on another item connects to it
 * (to its anchor dot if dropped on one). Dropping on empty canvas calls
 * `onEmpty`, if given, with the drop point (quick flow uses this to create
 * the next shape).
 */
export function startConnect(
  e: PointerEvent,
  fromId: string,
  side: Side | "auto",
  onEmpty?: (p: Point) => void,
): void {
  e.stopPropagation();
  const from = item(fromId);
  if (!from) return;
  const startSide = side === "auto" ? (nearestAnchor(from, canvasPoint(e), view.value.zoom) ?? "auto") : side;
  connecting.value = { from: fromId, side: startSide, to: canvasPoint(e) };
  document.body.classList.add("is-dragging");
  let moved = false;
  onPointer(
    (ev) => {
      moved = true;
      const target = itemAt(ev.clientX, ev.clientY, fromId);
      batch(() => {
        hoverTarget.value = target?.id ?? null;
        connecting.value = { from: fromId, side: startSide, to: canvasPoint(ev) };
      });
    },
    (ev) => {
      const p = canvasPoint(ev);
      const target = itemAt(ev.clientX, ev.clientY, fromId);
      batch(() => {
        connecting.value = null;
        hoverTarget.value = null;
      });
      if (!moved) return;
      if (target) {
        const toSide = nearestAnchor(target, p, view.value.zoom);
        void create([], [
          { from: fromId, to: target.id, from_anchor: startSide, ...(toSide ? { to_anchor: toSide } : {}) },
        ]);
        tool.value = "select";
      } else if (onEmpty) {
        onEmpty(p);
      }
    },
  );
}
