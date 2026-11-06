// The board: a pannable, zoomable cutting mat with cards on it.

import { useEffect, useRef, useState } from "preact/hooks";

import { api } from "../api";
import { Card } from "../canvas/Card";
import { TrashPanel } from "../canvas/TrashPanel";
import { clampZoom, fit, panBy, toCanvas, view, zoomAt } from "../canvas/viewport";
import { navigate } from "../router";
import { board, createCard, deleteItem, itemIds, items, load, report, selectedId } from "../store";

type Load = { kind: "loading" } | { kind: "ready" } | { kind: "missing" };

export function BoardView({ id }: { id: string }) {
  const [state, setState] = useState<Load>({ kind: "loading" });
  const [trashOpen, setTrashOpen] = useState(false);
  const [adding, setAdding] = useState<{ sx: number; sy: number; x: number; y: number } | null>(null);
  const mat = useRef<HTMLDivElement>(null);
  const pan = useRef<{ x: number; y: number } | null>(null);
  const spaceDown = useRef(false);

  useEffect(() => {
    let cancelled = false;
    setState({ kind: "loading" });
    api
      .getBoard(id)
      .then((full) => {
        if (cancelled) return;
        load(full.board, full.items);
        setState({ kind: "ready" });
        document.title = `${full.board.title} – Board`;
        requestAnimationFrame(fitAll);
      })
      .catch(() => !cancelled && setState({ kind: "missing" }));
    return () => {
      cancelled = true;
    };
  }, [id]);

  // Keyboard: Delete removes the selected card, Space-drag pans, Escape deselects.
  useEffect(() => {
    function typing(e: KeyboardEvent) {
      const t = e.target as HTMLElement;
      return t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName);
    }
    function down(e: KeyboardEvent) {
      if (typing(e)) return;
      if ((e.key === "Delete" || e.key === "Backspace") && selectedId.value) {
        e.preventDefault();
        void deleteItem(selectedId.value);
      } else if (e.key === "Escape") {
        selectedId.value = null;
        setAdding(null);
      } else if (e.key === " ") {
        spaceDown.current = true;
        mat.current?.classList.add("is-panning-ready");
        e.preventDefault();
      } else if (e.shiftKey && e.code === "Digit1") {
        fitAll();
      }
    }
    function up(e: KeyboardEvent) {
      if (e.key === " ") {
        spaceDown.current = false;
        mat.current?.classList.remove("is-panning-ready");
      }
    }
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
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

  function fitAll() {
    const el = mat.current;
    const list = [...items.values()].map((s) => s.item.value);
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

  function zoomBy(factor: number) {
    const el = mat.current;
    if (!el) return;
    zoomAt(el.clientWidth / 2, el.clientHeight / 2, clampZoom(view.value.zoom * factor));
  }

  if (state.kind === "loading") return <p class="page-message">Opening board…</p>;
  if (state.kind === "missing")
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
          <button type="button" class="zoom-level" title="Fit everything (Shift+1)" onClick={fitAll}>
            {Math.round(v.zoom * 100)}%
          </button>
          <button type="button" class="icon-button" aria-label="Zoom in" onClick={() => zoomBy(1.25)}>
            +
          </button>
        </div>
        <button
          type="button"
          class={`text-button${trashOpen ? " is-active" : ""}`}
          aria-pressed={trashOpen}
          onClick={() => setTrashOpen(!trashOpen)}
        >
          Trash
        </button>
      </header>

      <div
        ref={mat}
        class="mat"
        style={{
          "--grid-minor": `${minor}px`,
          "--grid-major": `${minor * 5}px`,
          "--grid-x": `${v.x}px`,
          "--grid-y": `${v.y}px`,
        }}
        onPointerDown={(e) => {
          if (e.button !== 0 && e.button !== 1) return;
          selectedId.value = null;
          setAdding(null);
          pan.current = { x: e.clientX, y: e.clientY };
          e.currentTarget.setPointerCapture(e.pointerId);
          e.currentTarget.classList.add("is-panning");
        }}
        onPointerMove={(e) => {
          const p = pan.current;
          if (!p) return;
          panBy(e.clientX - p.x, e.clientY - p.y);
          pan.current = { x: e.clientX, y: e.clientY };
        }}
        onPointerUp={(e) => {
          pan.current = null;
          e.currentTarget.classList.remove("is-panning");
        }}
        onDblClick={(e) => {
          if (e.target !== e.currentTarget && !(e.target as Element).classList.contains("world")) return;
          const r = e.currentTarget.getBoundingClientRect();
          const sx = e.clientX - r.left;
          const sy = e.clientY - r.top;
          setAdding({ sx, sy, ...toCanvas(sx, sy) });
        }}
      >
        <div class="world" style={{ transform: `translate(${v.x}px, ${v.y}px) scale(${v.zoom})` }}>
          {itemIds.value.map((itemId) => {
            const s = items.get(itemId);
            return s ? <Card key={itemId} state={s} /> : null;
          })}
        </div>

        {itemIds.value.length === 0 && !adding && (
          <p class="mat-hint">Double-click anywhere to add a note or a checklist.</p>
        )}

        {adding && (
          <div
            class="add-menu"
            role="menu"
            style={{ left: `${adding.sx}px`, top: `${adding.sy}px` }}
            onPointerDown={(e) => e.stopPropagation()}
          >
            {(["note", "checklist"] as const).map((t) => (
              <button
                key={t}
                type="button"
                role="menuitem"
                ref={(btn) => {
                  if (t === "note") btn?.focus();
                }}
                onClick={() => {
                  setAdding(null);
                  void createCard(t, adding.x, adding.y);
                }}
              >
                {t === "note" ? "Note" : "Checklist"}
              </button>
            ))}
          </div>
        )}
      </div>

      {trashOpen && b && <TrashPanel boardId={b.id} onClose={() => setTrashOpen(false)} />}
    </div>
  );
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
