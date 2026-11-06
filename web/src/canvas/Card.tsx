// A card on the canvas: drag by the header, resize from the corner.
//
// During a drag the element is moved directly (no re-render per pointer
// move); the final position is committed to state and saved when the drag
// ends (spec section 10.1).

import { useRef, useState } from "preact/hooks";

import { deleteItem, editContent, keepMine, saveGeometry, selectedId, takeTheirs, topZ, type ItemState } from "../store";
import type { ChecklistContent, NoteContent } from "../types";
import { ChecklistBody } from "./ChecklistBody";
import { NoteBody } from "./NoteBody";
import { view } from "./viewport";

const MIN_W = 180;
const MIN_H = 120;

type Gesture = {
  kind: "move" | "resize";
  startX: number;
  startY: number;
  x: number;
  y: number;
  w: number;
  h: number;
  /** Becomes true once the pointer has moved far enough to count as a drag. */
  dragging: boolean;
};

/** Movement (in screen pixels) before a press becomes a drag, so clicks and double-clicks still work. */
const DRAG_THRESHOLD = 3;

export function Card({ state }: { state: ItemState }) {
  const item = state.item.value;
  const conflict = state.conflict.value;
  const selected = selectedId.value === item.id;
  const el = useRef<HTMLElement>(null);
  const [lifted, setLifted] = useState(false);
  const [editingTitle, setEditingTitle] = useState(false);

  // Drags listen on the window rather than capturing the pointer: capture
  // would redirect click and double-click away from the title.
  function begin(kind: Gesture["kind"], e: PointerEvent) {
    if (e.button !== 0 || item.locked) return;
    e.stopPropagation();
    selectedId.value = item.id;
    const g: Gesture = {
      kind,
      startX: e.clientX,
      startY: e.clientY,
      x: item.x,
      y: item.y,
      w: item.w,
      h: item.h,
      dragging: false,
    };

    function at(ev: PointerEvent): Gesture {
      const zoom = view.value.zoom;
      const dx = (ev.clientX - g.startX) / zoom;
      const dy = (ev.clientY - g.startY) / zoom;
      return g.kind === "move"
        ? { ...g, x: Math.round(g.x + dx), y: Math.round(g.y + dy) }
        : { ...g, w: Math.round(Math.max(MIN_W, g.w + dx)), h: Math.round(Math.max(MIN_H, g.h + dy)) };
    }

    function move(ev: PointerEvent) {
      if (!g.dragging) {
        if (Math.hypot(ev.clientX - g.startX, ev.clientY - g.startY) < DRAG_THRESHOLD) return;
        g.dragging = true;
        setLifted(true);
        document.body.classList.add("is-dragging");
      }
      const n = at(ev);
      const node = el.current;
      if (!node) return;
      node.style.left = `${n.x}px`;
      node.style.top = `${n.y}px`;
      node.style.width = `${n.w}px`;
      node.style.height = `${n.h}px`;
    }

    function end(ev: PointerEvent) {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      document.body.classList.remove("is-dragging");
      if (!g.dragging) return;
      setLifted(false);
      const n = at(ev);
      const moved = n.x !== item.x || n.y !== item.y;
      const resized = n.w !== item.w || n.h !== item.h;
      if (!moved && !resized) return;
      const top = topZ();
      const z = item.z < top ? top + 1 : undefined;
      void saveGeometry(item.id, moved ? { x: n.x, y: n.y, ...(z ? { z } : {}) } : { w: n.w, h: n.h });
    }

    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  }

  const title = item.title ?? "";
  const label = item.card_type === "note" ? "Note" : "Checklist";

  return (
    <article
      ref={el}
      class={`card${selected ? " is-selected" : ""}${lifted ? " is-lifted" : ""}`}
      style={{ left: `${item.x}px`, top: `${item.y}px`, width: `${item.w}px`, height: `${item.h}px`, zIndex: item.z }}
      aria-label={`${label}: ${title || "untitled"}`}
      onPointerDown={(e) => {
        e.stopPropagation();
        selectedId.value = item.id;
      }}
    >
      <header class="card-header" onPointerDown={(e) => begin("move", e)}>
        {editingTitle ? (
          <input
            class="card-title-input"
            value={title}
            aria-label="Card title"
            // Typing shouldn't start a drag.
            onPointerDown={(e) => e.stopPropagation()}
            onInput={(e) => editContent(item.id, { title: e.currentTarget.value })}
            onBlur={() => setEditingTitle(false)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === "Escape") e.currentTarget.blur();
            }}
            ref={(input) => input?.focus()}
          />
        ) : (
          <h2 class="card-title" onDblClick={() => setEditingTitle(true)} title="Double-click to rename">
            {title || <span class="placeholder">Untitled</span>}
          </h2>
        )}
        <span class="card-meta" aria-live="polite">
          {state.saving.value ? "Saving" : ""}
        </span>
        <button
          type="button"
          class="icon-button card-delete"
          aria-label="Move to trash"
          title="Move to trash (Delete)"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={() => void deleteItem(item.id)}
        >
          ×
        </button>
      </header>

      {conflict && (
        <div class="card-conflict" role="alert">
          <p>
            This card was changed elsewhere
            {conflict.theirs.updated_by === "claude" ? " by Claude" : ""}.
          </p>
          <div class="card-conflict-actions">
            <button type="button" onClick={() => takeTheirs(item.id)}>
              Load their version
            </button>
            <button type="button" onClick={() => keepMine(item.id)}>
              Keep mine
            </button>
          </div>
        </div>
      )}

      <div class="card-body">
        {item.card_type === "note" ? (
          <NoteBody id={item.id} content={item.content as NoteContent} />
        ) : (
          <ChecklistBody id={item.id} content={item.content as ChecklistContent} />
        )}
      </div>

      <div class="card-resize" aria-hidden="true" onPointerDown={(e) => begin("resize", e)} />
    </article>
  );
}
