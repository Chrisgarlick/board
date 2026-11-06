// The tool palette (spec 10.2). One key per tool; the active tool is pressed.

import type { ComponentChildren } from "preact";

import { canRedo, canUndo, redo, tool, undo, type Tool } from "../store";
import { ShapeIcon } from "./shapes";

interface ToolDef {
  tool: Tool;
  label: string;
  key: string;
  icon: ComponentChildren;
}

const svg = (children: ComponentChildren) => (
  <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" class="tool-icon">
    {children}
  </svg>
);

const TOOLS: ToolDef[] = [
  { tool: "select", label: "Select", key: "V", icon: svg(<path d="M6 3l12 9-5.5 1L15 19l-2.5 1-2.5-6L6 18z" />) },
  {
    tool: "hand",
    label: "Pan",
    key: "H",
    icon: svg(<path d="M8 11V5.5a1.5 1.5 0 013 0V10m0-5a1.5 1.5 0 013 0v5m0-4a1.5 1.5 0 013 0v7a6 6 0 01-6 6h-1a6 6 0 01-5-2.7L4 15a1.5 1.5 0 012.5-1.6L8 15" />),
  },
  { tool: "rect", label: "Process", key: "R", icon: <ShapeIcon shape="rect" /> },
  { tool: "diamond", label: "Decision", key: "D", icon: <ShapeIcon shape="diamond" /> },
  { tool: "ellipse", label: "Event", key: "O", icon: <ShapeIcon shape="ellipse" /> },
  { tool: "sticky", label: "Sticky note", key: "S", icon: svg(<path d="M5 4h14v10l-5 6H5z M14 20v-6h5" />) },
  { tool: "text", label: "Text", key: "T", icon: svg(<path d="M5 6V4h14v2 M12 4v16 M9 20h6" />) },
  { tool: "frame", label: "Frame", key: "F", icon: svg(<path d="M4 8h16 M4 16h16 M8 4v16 M16 4v16" />) },
  {
    tool: "connector",
    label: "Connector",
    key: "C",
    icon: svg(<path d="M5 19L19 5 M13 5h6v6" />),
  },
];

export const TOOL_KEYS: Record<string, Tool> = Object.fromEntries(TOOLS.map((t) => [t.key.toLowerCase(), t.tool]));

export function Toolbar() {
  const active = tool.value;
  return (
    <nav class="toolbar" aria-label="Tools">
      {TOOLS.map((t) => (
        <button
          key={t.tool}
          type="button"
          class={`tool${active === t.tool ? " is-active" : ""}`}
          aria-pressed={active === t.tool}
          title={`${t.label} (${t.key})`}
          aria-label={t.label}
          onClick={() => (tool.value = t.tool)}
        >
          {t.icon}
        </button>
      ))}
      <span class="toolbar-gap" />
      <button
        type="button"
        class="tool"
        title="Undo (Cmd+Z)"
        aria-label="Undo"
        disabled={!canUndo.value}
        onClick={() => void undo()}
      >
        {svg(<path d="M9 7H4V2 M4 7a8 8 0 1 1 1.5 9" />)}
      </button>
      <button
        type="button"
        class="tool"
        title="Redo (Cmd+Shift+Z)"
        aria-label="Redo"
        disabled={!canRedo.value}
        onClick={() => void redo()}
      >
        {svg(<path d="M15 7h5V2 M20 7a8 8 0 1 0-1.5 9" />)}
      </button>
    </nav>
  );
}
