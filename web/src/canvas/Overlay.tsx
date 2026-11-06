// Screen-space layers over the canvas: the selection marquee (also the
// rubber band while creating), alignment guides, and quick-flow handles.

import type { Side } from "../types";
import { editingId, selectedItems, tool } from "../store";
import { guides, liveRect, marquee, moving, startConnect } from "./interact";
import { addNext, addNextAt } from "./quickflow";
import { view } from "./viewport";

export function Overlay() {
  const v = view.value;
  const m = marquee.value;
  const g = guides.value;
  return (
    <div class="overlay">
      {m && (
        <div
          aria-hidden="true"
          class={`marquee${tool.value === "select" ? "" : " is-creating"}`}
          style={{
            left: `${m.x * v.zoom + v.x}px`,
            top: `${m.y * v.zoom + v.y}px`,
            width: `${m.w * v.zoom}px`,
            height: `${m.h * v.zoom}px`,
          }}
        />
      )}
      {g.x.map((x) => (
        <div key={`x${x}`} aria-hidden="true" class="guide guide-v" style={{ left: `${x * v.zoom + v.x}px` }} />
      ))}
      {g.y.map((y) => (
        <div key={`y${y}`} aria-hidden="true" class="guide guide-h" style={{ top: `${y * v.zoom + v.y}px` }} />
      ))}
      <QuickFlowHandles />
    </div>
  );
}

const SIDES: Side[] = ["top", "right", "bottom", "left"];

/** + handles around a single selected shape: click or drag to add the next step. */
function QuickFlowHandles() {
  const list = selectedItems.value;
  const only = list.length === 1 ? list[0] : undefined;
  if (!only || only.kind !== "shape" || only.locked || editingId.value === only.id || moving.value.size) return null;
  const v = view.value;
  const r = liveRect(only);
  const out = 22;
  const pos: Record<Side, { left: number; top: number }> = {
    top: { left: (r.x + r.w / 2) * v.zoom + v.x, top: r.y * v.zoom + v.y - out },
    right: { left: (r.x + r.w) * v.zoom + v.x + out, top: (r.y + r.h / 2) * v.zoom + v.y },
    bottom: { left: (r.x + r.w / 2) * v.zoom + v.x, top: (r.y + r.h) * v.zoom + v.y + out },
    left: { left: r.x * v.zoom + v.x - out, top: (r.y + r.h / 2) * v.zoom + v.y },
  };
  return (
    <>
      {SIDES.map((side) => (
        <button
          key={side}
          type="button"
          class="flow-handle"
          aria-label={`Add a connected step to the ${side}`}
          title="Click to add the next step, or drag to place it"
          style={{ left: `${pos[side].left}px`, top: `${pos[side].top}px` }}
          onPointerDown={(e) => {
            let dragged = false;
            const start = { x: e.clientX, y: e.clientY };
            const watch = (ev: PointerEvent) => {
              if (Math.hypot(ev.clientX - start.x, ev.clientY - start.y) > 4) dragged = true;
            };
            window.addEventListener("pointermove", watch);
            window.addEventListener(
              "pointerup",
              () => {
                window.removeEventListener("pointermove", watch);
                if (!dragged) addNext(only.id, side);
              },
              { once: true },
            );
            startConnect(e, only.id, side, (p) => addNextAt(only.id, side, p));
          }}
        >
          +
        </button>
      ))}
    </>
  );
}
