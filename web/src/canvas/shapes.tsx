// SVG outlines for the flowchart shape set, drawn to fill a w × h box.

import type { ShapeKind } from "../types";

export const SHAPE_LABELS: Record<ShapeKind, string> = {
  rect: "Process",
  rounded: "Start or end",
  diamond: "Decision",
  ellipse: "Event",
  parallelogram: "Input or output",
  cylinder: "Data store",
  document: "Document",
};

export function ShapeOutline({ shape, w, h }: { shape: ShapeKind; w: number; h: number }) {
  // Inset by half the stroke so the outline isn't clipped.
  const i = 1;
  const W = w - i;
  const H = h - i;
  switch (shape) {
    case "rect":
      return <rect x={i} y={i} width={W - i} height={H - i} rx={2} />;
    case "rounded":
      return <rect x={i} y={i} width={W - i} height={H - i} rx={Math.min(h / 2, 36)} />;
    case "ellipse":
      return <ellipse cx={w / 2} cy={h / 2} rx={w / 2 - i} ry={h / 2 - i} />;
    case "diamond":
      return <polygon points={`${w / 2},${i} ${W},${h / 2} ${w / 2},${H} ${i},${h / 2}`} />;
    case "parallelogram": {
      const s = Math.min(24, w * 0.15);
      return <polygon points={`${s},${i} ${W},${i} ${W - s},${H} ${i},${H}`} />;
    }
    case "cylinder": {
      const ry = Math.min(14, h * 0.15);
      const rx = w / 2 - i;
      return (
        <>
          <path d={`M${i} ${ry} A${rx} ${ry} 0 0 1 ${W} ${ry} V${H - ry} A${rx} ${ry} 0 0 1 ${i} ${H - ry} Z`} />
          <path class="shape-detail" d={`M${i} ${ry} A${rx} ${ry} 0 0 0 ${W} ${ry}`} />
        </>
      );
    }
    case "document": {
      const wave = Math.min(10, h * 0.12);
      return (
        <path
          d={`M${i} ${i} H${W} V${H - wave} Q${w * 0.75} ${H - wave * 2.2} ${w / 2} ${H - wave} T${i} ${H - wave} Z`}
        />
      );
    }
  }
}

/** Small icons for the toolbar and shape menu. */
export function ShapeIcon({ shape }: { shape: ShapeKind }) {
  return (
    <svg class="shape-icon" viewBox="0 0 24 18" width="24" height="18" aria-hidden="true">
      <ShapeOutline shape={shape} w={24} h={18} />
    </svg>
  );
}
