// Quick flow (spec 10.2): from a selected shape, add the next connected shape
// in one gesture. Click a + handle (or press Tab) to add one at the default
// spacing; drag from a handle and drop on empty canvas to add one there.

import type { Item, ShapeContent, Side } from "../types";
import { create, editingId, item, select } from "../store";
import { DEFAULT_SIZE } from "./interact";
import type { Point } from "./routing";

const GAP = 80;
const OPPOSITE: Record<Side, Side> = { top: "bottom", bottom: "top", left: "right", right: "left" };

function nextShape(from: Item) {
  const shape = from.kind === "shape" ? (from.content as ShapeContent).shape : "rect";
  // A decision usually leads to a step, not another decision.
  const kind = shape === "diamond" ? "rect" : shape;
  return { kind, size: DEFAULT_SIZE[kind] ?? { w: 160, h: 72 } };
}

async function addConnected(from: Item, side: Side, centre: Point): Promise<void> {
  const { kind, size } = nextShape(from);
  const style = from.style?.color ? { color: from.style.color } : undefined;
  const [id] = await create(
    [
      {
        ref: "next",
        kind: "shape",
        content: { shape: kind, text: "" },
        ...(style ? { style } : {}),
        x: Math.round(centre.x - size.w / 2),
        y: Math.round(centre.y - size.h / 2),
        w: size.w,
        h: size.h,
      },
    ],
    [{ from: from.id, to: "next", from_anchor: side, to_anchor: OPPOSITE[side] }],
  );
  if (id) {
    select([id]);
    editingId.value = id;
  }
}

/** Add the next shape beside `fromId`, on `side`, at the default spacing. */
export function addNext(fromId: string, side: Side): void {
  const from = item(fromId);
  if (!from) return;
  const { size } = nextShape(from);
  const cx = from.x + from.w / 2;
  const cy = from.y + from.h / 2;
  const centre =
    side === "right"
      ? { x: from.x + from.w + GAP + size.w / 2, y: cy }
      : side === "left"
        ? { x: from.x - GAP - size.w / 2, y: cy }
        : side === "bottom"
          ? { x: cx, y: from.y + from.h + GAP + size.h / 2 }
          : { x: cx, y: from.y - GAP - size.h / 2 };
  void addConnected(from, side, centre);
}

/** Add the next shape centred on `point` (a drop on empty canvas). */
export function addNextAt(fromId: string, side: Side, point: Point): void {
  const from = item(fromId);
  if (from) void addConnected(from, side, point);
}
