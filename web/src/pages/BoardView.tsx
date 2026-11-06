// The board: a pannable, zoomable cutting mat with everything on it.

import { useEffect, useRef, useState } from "preact/hooks";

import { api } from "../api";
import { duplicate, onCopy, onPaste } from "../canvas/clipboard";
import { ConnectorLayer } from "../canvas/ConnectorLayer";
import { newItemFor, pressCreate, pressMarquee } from "../canvas/interact";
import { FAR_ZOOM, ItemView } from "../canvas/ItemView";
import { Overlay } from "../canvas/Overlay";
import { addNext } from "../canvas/quickflow";
import { StyleBar } from "../canvas/StyleBar";
import { TOOL_KEYS, Toolbar } from "../canvas/Toolbar";
import { HistoryPanel } from "../canvas/HistoryPanel";
import { SchemaPanel } from "../canvas/SchemaPanel";
import { SettingsPanel } from "../canvas/SettingsPanel";
import { TrashPanel } from "../canvas/TrashPanel";
import { pageId } from "../ids";
import { listen } from "../live";
import { clampZoom, fit, panBy, toCanvas, view, zoomAt } from "../canvas/viewport";
import { navigate } from "../router";
import {
  board,
  boardGone,
  bottomZ,
  create,
  editingId,
  frameIds,
  historyFor,
  item,
  itemIds,
  items,
  load,
  place,
  redo,
  remove,
  report,
  select,
  selectedConnector,
  selectedItems,
  selection,
  tool,
  topZ,
  undo,
} from "../store";
import type { Item } from "../types";

type Load = { kind: "loading" } | { kind: "ready" } | { kind: "missing" };

/** Past this many items, only those near the viewport are drawn (spec 10.1). */
const CULL_ABOVE = 500;
const NUDGE = 1;
const NUDGE_BIG = 10;

export function BoardView({ id }: { id: string }) {
  const [state, setState] = useState<Load>({ kind: "loading" });
  const [panel, setPanel] = useState<"trash" | "schema" | "settings" | null>(null);
  const toggle = (p: "trash" | "schema" | "settings") => setPanel(panel === p ? null : p);
  const [adding, setAdding] = useState<{ sx: number; sy: number; x: number; y: number } | null>(null);
  const mat = useRef<HTMLDivElement>(null);
  const spaceDown = useRef(false);

  useEffect(() => {
    let cancelled = false;
    setState({ kind: "loading" });
    api
      .getBoard(id)
      .then((full) => {
        if (cancelled) return;
        load(full.board, full.items, full.connectors, full.tables);
        setState({ kind: "ready" });
        document.title = `${full.board.title} – Board`;
        requestAnimationFrame(() => fitTo(allItems()));
      })
      .catch(() => !cancelled && setState({ kind: "missing" }));
    const stop = listen(id);
    return () => {
      cancelled = true;
      stop();
    };
  }, [id]);

  useEffect(() => {
    function typing(e: Event) {
      const t = e.target as HTMLElement;
      return t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName);
    }
    function down(e: KeyboardEvent) {
      if (typing(e)) return;
      const cmd = e.metaKey || e.ctrlKey;
      const key = e.key.toLowerCase();

      if (cmd && key === "z") {
        e.preventDefault();
        void (e.shiftKey ? redo() : undo());
      } else if (cmd && key === "y") {
        e.preventDefault();
        void redo();
      } else if (cmd && key === "a") {
        e.preventDefault();
        select(items.keys());
      } else if (cmd && key === "d") {
        e.preventDefault();
        duplicate();
      } else if (cmd && key === "l") {
        e.preventDefault();
        const list = selectedItems.value;
        const lock = !list.every((i) => i.locked);
        void place(list.map((i) => ({ id: i.id, locked: lock })));
      } else if (cmd) {
        return; // leave other shortcuts (copy, paste, reload) to the browser
      } else if (e.key === "Delete" || e.key === "Backspace") {
        e.preventDefault();
        void remove([...selection.value], e.shiftKey, selectedConnector.value);
      } else if (e.key === "Escape") {
        select([]);
        editingId.value = null;
        tool.value = "select";
        setAdding(null);
      } else if (e.key === "Enter" && selection.value.size === 1) {
        e.preventDefault();
        editingId.value = [...selection.value][0] ?? null;
      } else if (e.key === "Tab" && selection.value.size === 1) {
        const only = selectedItems.value[0];
        if (only?.kind === "shape") {
          e.preventDefault();
          addNext(only.id, "bottom");
        }
      } else if (e.key === " ") {
        spaceDown.current = true;
        mat.current?.classList.add("is-panning-ready");
        e.preventDefault();
      } else if (e.shiftKey && e.code === "Digit1") {
        fitTo(allItems());
      } else if (e.shiftKey && e.code === "Digit2") {
        fitTo(selectedItems.value.length ? selectedItems.value : allItems());
      } else if (e.key === "]" || e.key === "[") {
        const ids = [...selection.value];
        const base = e.key === "]" ? topZ() + 1 : bottomZ() - ids.length;
        void place(ids.map((id, n) => ({ id, z: base + n })));
      } else if (e.key.startsWith("Arrow") && selection.value.size) {
        e.preventDefault();
        const step = e.shiftKey ? NUDGE_BIG : NUDGE;
        const dx = e.key === "ArrowLeft" ? -step : e.key === "ArrowRight" ? step : 0;
        const dy = e.key === "ArrowUp" ? -step : e.key === "ArrowDown" ? step : 0;
        void place(selectedItems.value.filter((i) => !i.locked).map((i) => ({ id: i.id, x: i.x + dx, y: i.y + dy })));
      } else if (key === "f" && selection.value.size > 0) {
        void frameSelection();
      } else if (key === "k") {
        const el = mat.current;
        if (!el) return;
        const sx = el.clientWidth / 2;
        const sy = el.clientHeight / 2;
        setAdding({ sx, sy, ...toCanvas(sx, sy) });
      } else if (TOOL_KEYS[key] && !e.shiftKey && !e.altKey) {
        tool.value = TOOL_KEYS[key]!;
      }
    }
    function up(e: KeyboardEvent) {
      if (e.key === " ") {
        spaceDown.current = false;
        mat.current?.classList.remove("is-panning-ready");
      }
    }
    const copy = (e: ClipboardEvent) => !typing(e) && onCopy(e, false);
    const cut = (e: ClipboardEvent) => !typing(e) && onCopy(e, true);
    const paste = (e: ClipboardEvent) => !typing(e) && onPaste(e);
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("copy", copy);
    window.addEventListener("cut", cut);
    window.addEventListener("paste", paste);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("copy", copy);
      window.removeEventListener("cut", cut);
      window.removeEventListener("paste", paste);
    };
  }, []);

  // Wheel and trackpad: scroll pans, pinch (or Ctrl+scroll) zooms. Registered
  // by hand because it must be non-passive to stop the page zooming.
  useEffect(() => {
    const el = mat.current;
    if (!el) return;
    function wheel(e: WheelEvent) {
      e.preventDefault();
      if (!el) return;
      const r = el.getBoundingClientRect();
      if (e.ctrlKey || e.metaKey) {
        // A trackpad pinch sends small deltas; a mouse wheel sends ~100 per
        // notch. Capping the delta keeps one notch to a ~25% step.
        const delta = Math.max(-25, Math.min(25, e.deltaY));
        zoomAt(e.clientX - r.left, e.clientY - r.top, view.value.zoom * Math.exp(-delta * 0.01));
      } else {
        panBy(-e.deltaX, -e.deltaY);
      }
    }
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  }, [state.kind]);

  function allItems(): Item[] {
    return [...items.values()].map((s) => s.item.value);
  }

  function fitTo(list: Item[]) {
    const el = mat.current;
    if (!el) return;
    if (list.length === 0) {
      view.value = { x: 80, y: 80, zoom: 1 };
      return;
    }
    const minX = Math.min(...list.map((i) => i.x));
    const minY = Math.min(...list.map((i) => i.y));
    const maxX = Math.max(...list.map((i) => i.x + i.w));
    const maxY = Math.max(...list.map((i) => i.y + i.h));
    fit({ x: minX, y: minY, w: maxX - minX, h: maxY - minY }, el.clientWidth, el.clientHeight);
  }

  async function frameSelection() {
    const list = selectedItems.value.filter((i) => i.kind !== "frame");
    if (list.length === 0) return;
    const pad = 40;
    const minX = Math.min(...list.map((i) => i.x)) - pad;
    const minY = Math.min(...list.map((i) => i.y)) - pad;
    const maxX = Math.max(...list.map((i) => i.x + i.w)) + pad;
    const maxY = Math.max(...list.map((i) => i.y + i.h)) + pad;
    const [frame] = await create([
      { kind: "frame", title: "Frame", content: {}, x: minX, y: minY, w: maxX - minX, h: maxY - minY },
    ]);
    if (frame) {
      select([frame]);
      editingId.value = frame;
    }
  }

  function zoomBy(factor: number) {
    const el = mat.current;
    if (!el) return;
    zoomAt(el.clientWidth / 2, el.clientHeight / 2, clampZoom(view.value.zoom * factor));
  }

  async function addFromMenu(kind: string, at: { x: number; y: number }) {
    setAdding(null);
    const cards: Record<string, { title: string; content: unknown }> = {
      note: { title: "Note", content: { markdown: "" } },
      checklist: { title: "Checklist", content: { items: [{ text: "", done: false }] } },
      feature: {
        title: "Feature",
        content: { status: "idea", summary: "", stories: [], acceptance: [], table_ids: [], page_ids: [] },
      },
      schema: { title: "Data model", content: { table_ids: [] } },
      sitemap: { title: "Sitemap", content: { root: { id: pageId(), title: "Home", slug: "/" } } },
    };
    const card = cards[kind];
    if (card) {
      const [created] = await create([{ kind: "card", card_type: kind, ...card, x: at.x, y: at.y }]);
      if (created) select([created]);
      return;
    }
    const n = newItemFor(kind, { x: at.x, y: at.y, w: kind === "sticky" ? 180 : 160, h: kind === "sticky" ? 180 : 72 });
    if (!n) return;
    const [created] = await create([n]);
    if (created) {
      select([created]);
      editingId.value = created;
    }
  }

  if (state.kind === "loading") return <p class="page-message">Opening board…</p>;
  if (state.kind === "missing" || boardGone.value)
    return (
      <main class="page-message">
        <p>This board doesn't exist, or it was deleted.</p>
        <button type="button" onClick={() => navigate("/")}>
          Back to boards
        </button>
      </main>
    );

  const v = view.value;
  const b = board.value;
  const minor = 20 * v.zoom;
  const far = v.zoom < FAR_ZOOM;
  const t = tool.value;

  // Viewport culling on big boards.
  let visible: (id: string) => boolean = () => true;
  const el = mat.current;
  if (items.size > CULL_ABOVE && el) {
    const tl = toCanvas(-200, -200);
    const br = toCanvas(el.clientWidth + 200, el.clientHeight + 200);
    visible = (itemId) => {
      const i = item(itemId);
      return !!i && i.x + i.w >= tl.x && i.x <= br.x && i.y + i.h >= tl.y && i.y <= br.y;
    };
  }

  return (
    <div class="board-page">
      <header class="topbar">
        <button type="button" class="text-button" onClick={() => navigate("/")}>
          Boards
        </button>
        <BoardTitle />
        <div class="topbar-spacer" />
        <div class="zoom" role="group" aria-label="Zoom">
          <button type="button" class="icon-button" aria-label="Zoom out" onClick={() => zoomBy(1 / 1.25)}>
            −
          </button>
          <button type="button" class="zoom-level" title="Fit everything (Shift+1)" onClick={() => fitTo(allItems())}>
            {Math.round(v.zoom * 100)}%
          </button>
          <button type="button" class="icon-button" aria-label="Zoom in" onClick={() => zoomBy(1.25)}>
            +
          </button>
        </div>
        {(
          [
            ["schema", "Schema"],
            ["settings", "Settings"],
            ["trash", "Trash"],
          ] as const
        ).map(([p, label]) => (
          <button
            key={p}
            type="button"
            class={`text-button${panel === p ? " is-active" : ""}`}
            aria-pressed={panel === p}
            onClick={() => toggle(p)}
          >
            {label}
          </button>
        ))}
      </header>

      <div
        ref={mat}
        class={`mat tool-${t}`}
        style={{
          "--grid-minor": `${minor}px`,
          "--grid-major": `${minor * 5}px`,
          "--grid-x": `${v.x}px`,
          "--grid-y": `${v.y}px`,
        }}
        onPointerDown={(e) => {
          const target = e.target as Element;
          // Only presses on the mat itself, the world, or a frame's body (which is see-through).
          if (target !== e.currentTarget && !target.classList.contains("world") && !target.classList.contains("frame")) return;
          setAdding(null);
          if (editingId.value) editingId.value = null;
          const panning = e.button === 1 || spaceDown.current || t === "hand";
          if (panning) {
            startPan(e);
          } else if (e.button === 0 && t === "select") {
            pressMarquee(e);
          } else if (e.button === 0 && t !== "connector") {
            pressCreate(e);
          }
        }}
        onDblClick={(e) => {
          const target = e.target as Element;
          if (target !== e.currentTarget && !target.classList.contains("world") && !target.classList.contains("frame")) return;
          const r = e.currentTarget.getBoundingClientRect();
          const sx = e.clientX - r.left;
          const sy = e.clientY - r.top;
          setAdding({ sx, sy, ...toCanvas(sx, sy) });
        }}
      >
        <div class="world" style={{ transform: `translate(${v.x}px, ${v.y}px) scale(${v.zoom})` }}>
          {frameIds.value.filter(visible).map((itemId) => {
            const s = items.get(itemId);
            return s ? <ItemView key={itemId} state={s} far={far} /> : null;
          })}
          <ConnectorLayer />
          {itemIds.value.filter(visible).map((itemId) => {
            const s = items.get(itemId);
            return s ? <ItemView key={itemId} state={s} far={far} /> : null;
          })}
        </div>

        <Overlay />
        <StyleBar />
        <Toolbar />

        {itemIds.value.length + frameIds.value.length === 0 && !adding && (
          <p class="mat-hint">
            Pick a tool on the left, or double-click anywhere to add a note, checklist, feature, schema, sitemap, sticky or shape.
          </p>
        )}

        {adding && (
          <div
            class="add-menu"
            role="menu"
            style={{ left: `${adding.sx}px`, top: `${adding.sy}px` }}
            onPointerDown={(e) => e.stopPropagation()}
          >
            {(
              [
                ["note", "Note"],
                ["checklist", "Checklist"],
                ["feature", "Feature"],
                ["schema", "Schema (tables)"],
                ["sitemap", "Sitemap"],
                ["sticky", "Sticky note"],
                ["rect", "Process step"],
                ["text", "Text"],
              ] as const
            ).map(([k, label], n) => (
              <button
                key={k}
                type="button"
                role="menuitem"
                ref={(btn) => {
                  if (n === 0) btn?.focus();
                }}
                onClick={() => void addFromMenu(k, adding)}
              >
                {label}
              </button>
            ))}
          </div>
        )}
      </div>

      {panel === "trash" && b && <TrashPanel boardId={b.id} onClose={() => setPanel(null)} />}
      {panel === "schema" && <SchemaPanel onClose={() => setPanel(null)} />}
      {panel === "settings" && <SettingsPanel onClose={() => setPanel(null)} />}
      {historyFor.value && !panel && <HistoryPanel id={historyFor.value} />}
    </div>
  );
}

function startPan(e: PointerEvent) {
  let last = { x: e.clientX, y: e.clientY };
  const target = e.currentTarget as HTMLElement;
  target.classList.add("is-panning");
  function move(ev: PointerEvent) {
    panBy(ev.clientX - last.x, ev.clientY - last.y);
    last = { x: ev.clientX, y: ev.clientY };
  }
  function up() {
    target.classList.remove("is-panning");
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
  }
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
}

function BoardTitle() {
  const b = board.value;
  const [draft, setDraft] = useState<string | null>(null);
  if (!b) return null;

  async function save(title: string) {
    setDraft(null);
    const t = title.trim();
    const current = board.value;
    if (!current || !t || t === current.title) return;
    try {
      board.value = await api.updateBoard(current.id, { title: t });
      document.title = `${t} – Board`;
    } catch (err) {
      report(err, "rename the board");
    }
  }

  return draft === null ? (
    <h1 class="board-title" title="Double-click to rename" onDblClick={() => setDraft(b.title)}>
      {b.title}
    </h1>
  ) : (
    <input
      class="board-title-input"
      value={draft}
      aria-label="Board title"
      onInput={(e) => setDraft(e.currentTarget.value)}
      onBlur={(e) => void save(e.currentTarget.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") setDraft(null);
      }}
      ref={(input) => input?.focus()}
    />
  );
}
