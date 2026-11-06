// The floating style bar over the selection (spec 10.2): colour, shape type,
// alignment, text size, arrange, lock; or a connector's line, ends and routing.

import { useLayoutEffect, useRef } from "preact/hooks";

import {
  bottomZ,
  connectors,
  historyFor,
  place,
  remove,
  reshape,
  restyle,
  selectedConnector,
  selectedItems,
  topZ,
  updateConnector,
} from "../store";
import type { Color, ConnectorStyle, Item, ShapeKind, TextContent } from "../types";
import { liveRect } from "./interact";
import { SHAPE_LABELS } from "./shapes";
import { view } from "./viewport";

const COLORS: Color[] = ["grey", "blue", "green", "yellow", "orange", "red", "purple", "pink"];
const SHAPES = Object.keys(SHAPE_LABELS) as ShapeKind[];

function Swatches({ current, onPick }: { current: string | undefined; onPick: (c: Color) => void }) {
  return (
    <div class="swatches" role="group" aria-label="Colour">
      {COLORS.map((c) => (
        <button
          key={c}
          type="button"
          class={`swatch c-${c}${current === c ? " is-active" : ""}`}
          aria-label={c}
          aria-pressed={current === c}
          title={c[0]!.toUpperCase() + c.slice(1)}
          onClick={() => onPick(c)}
        />
      ))}
    </div>
  );
}

function bounds(list: Item[]) {
  const rects = list.map(liveRect);
  const x = Math.min(...rects.map((r) => r.x));
  const y = Math.min(...rects.map((r) => r.y));
  const right = Math.max(...rects.map((r) => r.x + r.w));
  const bottom = Math.max(...rects.map((r) => r.y + r.h));
  return { x, y, w: right - x, h: bottom - y };
}

/** Room to leave for the toolbar on the left of the canvas. */
const TOOLBAR_CLEARANCE = 64;
const EDGE = 8;

export function StyleBar() {
  const connectorId = selectedConnector.value;
  if (connectorId) return <ConnectorBar id={connectorId} />;
  return <ItemBar />;
}

function ItemBar() {
  const el = useRef<HTMLDivElement>(null);
  const list = selectedItems.value;
  const v = view.value;
  const b = list.length ? bounds(list) : { x: 0, y: 0, w: 0, h: 0 };
  const centre = b.x * v.zoom + v.x + (b.w * v.zoom) / 2;
  const top = b.y * v.zoom + v.y;
  const bottom = (b.y + b.h) * v.zoom + v.y;

  // Keep the bar inside the canvas and clear of the toolbar; if there's no
  // room above the selection, put it below.
  useLayoutEffect(() => {
    const node = el.current;
    const mat = node?.parentElement;
    if (!node || !mat) return;
    const half = node.offsetWidth / 2;
    const min = TOOLBAR_CLEARANCE + half;
    const max = mat.clientWidth - EDGE - half;
    node.style.left = `${Math.max(min, Math.min(max, centre))}px`;
    const below = top - node.offsetHeight - 18 < EDGE;
    node.style.top = `${below ? bottom : top}px`;
    node.classList.toggle("is-below", below);
  });

  if (list.length === 0) return null;
  const ids = list.map((i) => i.id);
  const shapes = list.every((i) => i.kind === "shape");
  const texts = list.every((i) => i.kind === "text");
  const alignable = list.every((i) => i.kind === "shape" || i.kind === "text");
  const first = list[0]!;
  const locked = list.every((i) => i.locked);

  return (
    <div ref={el} class="style-bar" role="toolbar" aria-label="Style">
      <Swatches current={first.style?.color} onPick={(color) => restyle(ids, { color })} />
      {shapes && (
        <select
          aria-label="Shape"
          value={list.length === 1 ? (first.content as { shape: ShapeKind }).shape : ""}
          onChange={(e) => reshape(ids, { shape: e.currentTarget.value })}
        >
          {list.length > 1 && <option value="">Shape</option>}
          {SHAPES.map((s) => (
            <option key={s} value={s}>
              {SHAPE_LABELS[s]}
            </option>
          ))}
        </select>
      )}
      {alignable && (
        <div class="segmented" role="group" aria-label="Text alignment">
          {(["left", "center", "right"] as const).map((a) => (
            <button
              key={a}
              type="button"
              class={(first.style?.align ?? (first.kind === "text" ? "left" : "center")) === a ? "is-active" : ""}
              aria-label={`Align ${a}`}
              onClick={() => restyle(ids, { align: a })}
            >
              {a === "left" ? "⟸" : a === "center" ? "≡" : "⟹"}
            </button>
          ))}
        </div>
      )}
      {texts && (
        <select
          aria-label="Text size"
          value={(first.content as TextContent).size}
          onChange={(e) => reshape(ids, { size: e.currentTarget.value })}
        >
          <option value="s">Small</option>
          <option value="m">Medium</option>
          <option value="l">Large</option>
          <option value="xl">Heading</option>
        </select>
      )}
      {list.length >= 2 && <ArrangeMenu list={list} />}
      <button type="button" title="Bring to front (])" onClick={() => void place(ids.map((id, n) => ({ id, z: topZ() + 1 + n })))}>
        Front
      </button>
      <button type="button" title="Send to back ([)" onClick={() => void place(ids.map((id, n) => ({ id, z: bottomZ() - ids.length + n })))}>
        Back
      </button>
      <button
        type="button"
        aria-pressed={locked}
        title="Lock (Cmd+L)"
        onClick={() => void place(ids.map((id) => ({ id, locked: !locked })))}
      >
        {locked ? "Unlock" : "Lock"}
      </button>
      {list.length === 1 && (
        <button type="button" title="Saved versions of this item" onClick={() => (historyFor.value = first.id)}>
          History
        </button>
      )}
      <button type="button" class="danger" title="Delete" onClick={() => void remove(ids)}>
        Delete
      </button>
    </div>
  );
}

function ArrangeMenu({ list }: { list: Item[] }) {
  function align(how: string) {
    const b = bounds(list);
    const changes = list.map((i) => {
      switch (how) {
        case "left":
          return { id: i.id, x: b.x };
        case "centre":
          return { id: i.id, x: Math.round(b.x + b.w / 2 - i.w / 2) };
        case "right":
          return { id: i.id, x: b.x + b.w - i.w };
        case "top":
          return { id: i.id, y: b.y };
        case "middle":
          return { id: i.id, y: Math.round(b.y + b.h / 2 - i.h / 2) };
        default:
          return { id: i.id, y: b.y + b.h - i.h };
      }
    });
    void place(changes);
  }

  function distribute(axis: "x" | "y") {
    const size = axis === "x" ? "w" : "h";
    const sorted = [...list].sort((a, b) => a[axis] - b[axis]);
    const first = sorted[0]!;
    const last = sorted[sorted.length - 1]!;
    const total = sorted.reduce((s, i) => s + i[size], 0);
    const gap = (last[axis] + last[size] - first[axis] - total) / (sorted.length - 1);
    let at = first[axis];
    void place(
      sorted.map((i) => {
        const change = { id: i.id, [axis]: Math.round(at) };
        at += i[size] + gap;
        return change;
      }),
    );
  }

  return (
    <select
      aria-label="Arrange"
      value=""
      onChange={(e) => {
        const v = e.currentTarget.value;
        if (v === "dx") distribute("x");
        else if (v === "dy") distribute("y");
        else if (v) align(v);
        e.currentTarget.value = "";
      }}
    >
      <option value="">Arrange</option>
      <option value="left">Align left</option>
      <option value="centre">Align centres</option>
      <option value="right">Align right</option>
      <option value="top">Align top</option>
      <option value="middle">Align middles</option>
      <option value="bottom">Align bottom</option>
      {list.length >= 3 && <option value="dx">Distribute across</option>}
      {list.length >= 3 && <option value="dy">Distribute down</option>}
    </select>
  );
}

function ConnectorBar({ id }: { id: string }) {
  const c = connectors.get(id)?.value;
  if (!c) return null;
  const style: ConnectorStyle = c.style ?? {};
  const set = (patch: Record<string, string>) => {
    const next: Record<string, unknown> = { ...style, ...patch };
    void updateConnector(id, { style: next as ConnectorStyle });
  };
  return (
    <div class="style-bar is-docked" role="toolbar" aria-label="Connector style">
      <Swatches current={style.color} onPick={(color) => set({ color })} />
      <select aria-label="Line" value={style.line ?? "solid"} onChange={(e) => set({ line: e.currentTarget.value })}>
        <option value="solid">Solid</option>
        <option value="dashed">Dashed</option>
        <option value="dotted">Dotted</option>
      </select>
      <select aria-label="Start" value={style.start ?? "none"} onChange={(e) => set({ start: e.currentTarget.value })}>
        <option value="none">No start</option>
        <option value="arrow">Arrow start</option>
        <option value="open_arrow">Open arrow start</option>
        <option value="dot">Dot start</option>
      </select>
      <select aria-label="End" value={style.end ?? "arrow"} onChange={(e) => set({ end: e.currentTarget.value })}>
        <option value="arrow">Arrow end</option>
        <option value="open_arrow">Open arrow end</option>
        <option value="dot">Dot end</option>
        <option value="none">No end</option>
      </select>
      <select aria-label="Routing" value={style.routing ?? "elbow"} onChange={(e) => set({ routing: e.currentTarget.value })}>
        <option value="elbow">Elbow</option>
        <option value="straight">Straight</option>
        <option value="curved">Curved</option>
      </select>
      <button type="button" class="danger" onClick={() => void remove([], false, id)}>
        Delete
      </button>
    </div>
  );
}

