// Connectors: one SVG layer beneath the items (spec 10.1), plus HTML labels.
//
// Arrowheads are drawn as plain paths at each end (not SVG markers), so they
// take the line's colour and match what the phase 4 renderer will draw.

import { useRef, useState } from "preact/hooks";

import { connectorIds, connectors, item, selectedConnector, select, updateConnector } from "../store";
import type { Connector, ConnectorStyle } from "../types";
import { connecting, hoverTarget, liveRect } from "./interact";
import { anchorPoint, pathData, route, type Point, type Route } from "./routing";

const HEAD = 10;

export function ConnectorLayer() {
  return (
    <>
      <svg class="connector-layer" aria-hidden="false" role="group" aria-label="Connectors">
        {connectorIds.value.map((id) => (
          <ConnectorPath key={id} id={id} />
        ))}
        <Preview />
        <AnchorDots />
      </svg>
      {connectorIds.value.map((id) => (
        <ConnectorLabel key={id} id={id} />
      ))}
    </>
  );
}

function routeFor(c: Connector): Route | null {
  const a = item(c.from_item);
  const b = item(c.to_item);
  if (!a || !b) return null;
  return route(liveRect(a), liveRect(b), c.from_anchor, c.to_anchor, c.style?.routing ?? "elbow");
}

function ConnectorPath({ id }: { id: string }) {
  const c = connectors.get(id)?.value;
  if (!c) return null;
  const r = routeFor(c);
  if (!r) return null; // an end is deleted: hidden, not gone
  const style = c.style ?? {};
  const selected = selectedConnector.value === id;
  const d = pathData(r);
  return (
    <g
      class={`connector c-${style.color ?? "grey"} line-${style.line ?? "solid"}${selected ? " is-selected" : ""}`}
      onPointerDown={(e) => {
        e.stopPropagation();
        select([]);
        selectedConnector.value = id;
      }}
    >
      <path class="connector-hit" d={d} />
      <path class="connector-line" d={d} />
      <Head route={r} at="start" kind={style.start ?? "none"} />
      <Head route={r} at="end" kind={style.end ?? "arrow"} />
    </g>
  );
}

/** The direction a route arrives at its end (or leaves its start), as a unit vector. */
function direction(r: Route, at: "start" | "end"): { tip: Point; ux: number; uy: number } {
  const pts = r.points;
  let tip: Point;
  let prev: Point;
  if (at === "end") {
    tip = pts[pts.length - 1] as Point;
    prev = r.curve ? r.curve.c2 : (pts[pts.length - 2] as Point);
  } else {
    tip = pts[0] as Point;
    prev = r.curve ? r.curve.c1 : (pts[1] as Point);
  }
  const len = Math.hypot(tip.x - prev.x, tip.y - prev.y) || 1;
  return { tip, ux: (tip.x - prev.x) / len, uy: (tip.y - prev.y) / len };
}

function Head({ route: r, at, kind }: { route: Route; at: "start" | "end"; kind: NonNullable<ConnectorStyle["end"]> }) {
  if (kind === "none") return null;
  const { tip, ux, uy } = direction(r, at);
  if (kind === "dot") return <circle class="connector-head" cx={tip.x} cy={tip.y} r={4} />;
  const bx = tip.x - ux * HEAD;
  const by = tip.y - uy * HEAD;
  const px = -uy * (HEAD / 2);
  const py = ux * (HEAD / 2);
  const left = `${bx + px} ${by + py}`;
  const right = `${bx - px} ${by - py}`;
  if (kind === "open_arrow") return <path class="connector-head-open" d={`M${left}L${tip.x} ${tip.y}L${right}`} />;
  return <path class="connector-head" d={`M${left}L${tip.x} ${tip.y}L${right}Z`} />;
}

function ConnectorLabel({ id }: { id: string }) {
  const c = connectors.get(id)?.value;
  const [editing, setEditing] = useState(false);
  const before = useRef<string | null>(null);
  if (!c) return null;
  const r = routeFor(c);
  if (!r) return null;
  const selected = selectedConnector.value === id;
  if (!c.label && !editing && !selected) return null;

  const pos = { left: `${r.label.x}px`, top: `${r.label.y}px` };
  if (editing) {
    return (
      <input
        class="connector-label-input"
        style={pos}
        defaultValue={c.label ?? ""}
        aria-label="Connector label"
        onPointerDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter" || e.key === "Escape") e.currentTarget.blur();
        }}
        onBlur={(e) => {
          setEditing(false);
          const label = e.currentTarget.value.trim() || null;
          if (label !== before.current) void updateConnector(id, { label });
        }}
        ref={(input) => {
          if (input && document.activeElement !== input) input.focus();
        }}
      />
    );
  }
  return (
    <button
      type="button"
      class={`connector-label${c.label ? "" : " is-empty"}`}
      style={pos}
      onPointerDown={(e) => {
        e.stopPropagation();
        select([]);
        selectedConnector.value = id;
      }}
      onDblClick={() => {
        before.current = c.label;
        setEditing(true);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          before.current = c.label;
          setEditing(true);
        }
      }}
    >
      {c.label || "Add label"}
    </button>
  );
}

/** The line being drawn while connecting. */
function Preview() {
  const p = connecting.value;
  if (!p) return null;
  const from = item(p.from);
  if (!from) return null;
  const r = liveRect(from);
  const start = p.side === "auto" ? { x: r.x + r.w / 2, y: r.y + r.h / 2 } : anchorPoint(r, p.side);
  return <path class="connector-preview" d={`M${start.x} ${start.y}L${p.to.x} ${p.to.y}`} />;
}

/** Anchor dots on the item under the pointer while connecting. */
function AnchorDots() {
  const id = hoverTarget.value;
  if (!id || !connecting.value) return null;
  const i = item(id);
  if (!i) return null;
  const r = liveRect(i);
  return (
    <g class="anchor-dots">
      {(["top", "right", "bottom", "left"] as const).map((side) => {
        const a = anchorPoint(r, side);
        return <circle key={side} cx={a.x} cy={a.y} r={5} />;
      })}
    </g>
  );
}
