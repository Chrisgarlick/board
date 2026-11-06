// A card: a header to drag by, a typed body, a corner to resize from.

import { useRef } from "preact/hooks";

import {
  editContent,
  editingId,
  keepMine,
  rememberEdit,
  remove,
  select,
  selection,
  takeTheirs,
  type ItemState,
} from "../store";
import type { ChecklistContent, NoteContent } from "../types";
import { ChecklistBody } from "./ChecklistBody";
import { liveRect, moving, pressItem, pressResize } from "./interact";
import { NoteBody } from "./NoteBody";

const MIN_W = 180;
const MIN_H = 120;

export function Card({ state, far }: { state: ItemState; far: boolean }) {
  const item = state.item.value;
  const conflict = state.conflict.value;
  const r = liveRect(item);
  const selected = selection.value.has(item.id);
  const lifted = moving.value.has(item.id);
  const editingTitle = editingId.value === item.id;
  const title = item.title ?? "";
  const label = item.card_type === "note" ? "Note" : "Checklist";
  const color = item.style?.color ? ` c-${item.style.color}` : "";

  return (
    <article
      class={`item card${color}${selected ? " is-selected" : ""}${lifted ? " is-lifted" : ""}${far ? " is-far" : ""}${item.locked ? " is-locked" : ""}`}
      data-item={item.id}
      style={{ left: `${r.x}px`, top: `${r.y}px`, width: `${r.w}px`, height: `${r.h}px` }}
      aria-label={`${label}: ${title || "untitled"}`}
      onPointerDown={(e) => {
        // Clicks in the body select the card; only the header drags it.
        e.stopPropagation();
        if (e.shiftKey) {
          const next = new Set(selection.value);
          if (next.has(item.id)) next.delete(item.id);
          else next.add(item.id);
          select(next);
        } else if (!selection.value.has(item.id)) {
          select([item.id]);
        }
      }}
    >
      <header class="card-header" onPointerDown={(e) => pressItem(e, item.id)}>
        {editingTitle ? (
          <CardTitleInput id={item.id} title={title} />
        ) : (
          <h2 class="card-title" onDblClick={() => (editingId.value = item.id)} title="Double-click to rename">
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
          onClick={() => void remove([item.id])}
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

      {!far && (
        <div class="card-body">
          {item.card_type === "note" ? (
            <NoteBody id={item.id} content={item.content as NoteContent} />
          ) : (
            <ChecklistBody id={item.id} content={item.content as ChecklistContent} />
          )}
        </div>
      )}

      {!item.locked && (
        <div class="card-resize" aria-hidden="true" onPointerDown={(e) => pressResize(e, item.id, MIN_W, MIN_H)} />
      )}
    </article>
  );
}

function CardTitleInput({ id, title }: { id: string; title: string }) {
  const before = useRef(title);
  return (
    <input
      class="card-title-input"
      value={title}
      aria-label="Card title"
      onPointerDown={(e) => e.stopPropagation()}
      onInput={(e) => editContent(id, { title: e.currentTarget.value })}
      onBlur={(e) => {
        rememberEdit(id, { title: before.current }, { title: e.currentTarget.value });
        if (editingId.value === id) editingId.value = null;
      }}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter" || e.key === "Escape") e.currentTarget.blur();
      }}
      ref={(input) => {
        if (input && document.activeElement !== input) input.focus();
      }}
    />
  );
}
