// Everything on the canvas except connectors: shapes, stickies, text, frames
// and cards. Each kind shares positioning, selection, live drag/resize and
// in-place text editing.

import type { ComponentChildren } from "preact";
import { useRef } from "preact/hooks";

import {
  editContent,
  editingId,
  rememberEdit,
  selection,
  type ContentPatch,
  type ItemState,
} from "../store";
import type { Item, ShapeContent, StickyContent, TextContent } from "../types";
import { Card } from "./Card";
import { liveRect, moving, pressItem, pressResize } from "./interact";
import { ShapeOutline, SHAPE_LABELS } from "./shapes";

/** Cards collapse to their title below this zoom (spec 10.1). */
export const FAR_ZOOM = 0.4;

export function ItemView({ state, far }: { state: ItemState; far: boolean }) {
  const i = state.item.value;
  switch (i.kind) {
    case "shape":
      return <ShapeView item={i} />;
    case "sticky":
      return <StickyView item={i} />;
    case "text":
      return <TextView item={i} />;
    case "frame":
      return <FrameView item={i} />;
    case "card":
      return <Card state={state} far={far} />;
  }
}

/** The positioned box every kind sits in. */
export function Box({
  item: i,
  class: cls,
  label,
  children,
  draggable = true,
}: {
  item: Item;
  class: string;
  label: string;
  children: ComponentChildren;
  /** False for kinds that drag by a handle (cards, frames) rather than the whole body. */
  draggable?: boolean;
}) {
  const r = liveRect(i);
  const selected = selection.value.has(i.id);
  const lifted = moving.value.has(i.id);
  const single = selected && selection.value.size === 1;
  const color = i.style?.color ? ` c-${i.style.color}` : "";
  return (
    <div
      class={`item ${cls}${color}${selected ? " is-selected" : ""}${lifted ? " is-lifted" : ""}${i.locked ? " is-locked" : ""}`}
      data-item={i.id}
      role="group"
      aria-label={label}
      style={{ left: `${r.x}px`, top: `${r.y}px`, width: `${r.w}px`, height: `${r.h}px` }}
      onPointerDown={draggable ? (e) => pressItem(e, i.id) : undefined}
      onDblClick={draggable ? () => (editingId.value = i.id) : undefined}
    >
      {children}
      {single && !i.locked && editingId.value !== i.id && i.kind !== "card" && (
        <div class="resize-handle" aria-hidden="true" onPointerDown={(e) => pressResize(e, i.id)} />
      )}
      {i.locked && <span class="lock-badge" title="Locked (Cmd+L to unlock)" aria-label="Locked" />}
    </div>
  );
}

/**
 * A plain-text editor that fills its parent. Saves as you type; when editing
 * ends, records one undo step for the whole edit.
 */
export function TextEditor({
  item: i,
  value,
  patch,
  label,
  class: cls = "",
}: {
  item: Item;
  value: string;
  /** Turn the edited text into a content patch. */
  patch: (text: string) => ContentPatch;
  label: string;
  class?: string;
}) {
  const before = useRef<ContentPatch>(patch(value));
  return (
    <textarea
      class={`text-editor ${cls}`}
      value={value}
      aria-label={label}
      onPointerDown={(e) => e.stopPropagation()}
      onInput={(e) => editContent(i.id, patch(e.currentTarget.value))}
      onBlur={(e) => {
        rememberEdit(i.id, before.current, patch(e.currentTarget.value));
        if (editingId.value === i.id) editingId.value = null;
      }}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Escape" || (e.key === "Enter" && (e.metaKey || e.ctrlKey))) e.currentTarget.blur();
      }}
      ref={(t) => {
        if (t && document.activeElement !== t) {
          t.focus();
          t.setSelectionRange(t.value.length, t.value.length);
        }
      }}
    />
  );
}

function ShapeView({ item: i }: { item: Item }) {
  const c = i.content as ShapeContent;
  const r = liveRect(i);
  const align = i.style?.align ?? "center";
  const editing = editingId.value === i.id;
  return (
    <Box item={i} class={`shape shape-${c.shape}`} label={`${SHAPE_LABELS[c.shape]}: ${c.text || "empty"}`}>
      <svg class="shape-outline" width={r.w} height={r.h} aria-hidden="true">
        <ShapeOutline shape={c.shape} w={r.w} h={r.h} />
      </svg>
      {editing ? (
        <TextEditor
          item={i}
          value={c.text}
          patch={(text) => ({ content: { ...c, text } })}
          label="Shape text"
          class={`align-${align}`}
        />
      ) : (
        <div class={`shape-text align-${align}`}>{c.text}</div>
      )}
    </Box>
  );
}

function StickyView({ item: i }: { item: Item }) {
  const c = i.content as StickyContent;
  const editing = editingId.value === i.id;
  return (
    <Box item={i} class="sticky" label={`Sticky note: ${c.text || "empty"}`}>
      {editing ? (
        <TextEditor item={i} value={c.text} patch={(text) => ({ content: { text } })} label="Sticky note text" />
      ) : (
        <div class="sticky-text">{c.text || <span class="placeholder">Double-click to write</span>}</div>
      )}
    </Box>
  );
}

function TextView({ item: i }: { item: Item }) {
  const c = i.content as TextContent;
  const align = i.style?.align ?? "left";
  const editing = editingId.value === i.id;
  return (
    <Box item={i} class={`free-text size-${c.size}`} label={`Text: ${c.text || "empty"}`}>
      {editing ? (
        <TextEditor
          item={i}
          value={c.text}
          patch={(text) => ({ content: { ...c, text } })}
          label="Text"
          class={`align-${align}`}
        />
      ) : (
        <div class={`free-text-body align-${align}`}>{c.text || <span class="placeholder">Text</span>}</div>
      )}
    </Box>
  );
}

function FrameView({ item: i }: { item: Item }) {
  const editing = editingId.value === i.id;
  const title = i.title ?? "";
  return (
    <Box item={i} class="frame" label={`Frame: ${title}`} draggable={false}>
      <div
        class="frame-tab"
        onPointerDown={(e) => pressItem(e, i.id)}
        onDblClick={() => (editingId.value = i.id)}
        title="Drag to move the frame and everything in it"
      >
        {editing ? (
          <FrameTitleInput item={i} />
        ) : (
          <span class="frame-title">{title || <span class="placeholder">Untitled frame</span>}</span>
        )}
      </div>
    </Box>
  );
}

function FrameTitleInput({ item: i }: { item: Item }) {
  const before = useRef(i.title ?? "");
  return (
    <input
      class="frame-title-input"
      value={i.title ?? ""}
      aria-label="Frame title"
      onPointerDown={(e) => e.stopPropagation()}
      onInput={(e) => editContent(i.id, { title: e.currentTarget.value })}
      onBlur={(e) => {
        rememberEdit(i.id, { title: before.current }, { title: e.currentTarget.value });
        if (editingId.value === i.id) editingId.value = null;
      }}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter" || e.key === "Escape") e.currentTarget.blur();
      }}
      ref={(input) => {
        if (input && document.activeElement !== input) {
          input.focus();
          // Select the title so typing replaces it (a new frame starts as "Frame").
          input.select();
        }
      }}
    />
  );
}
