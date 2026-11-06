// Shapes returned by the API (crates/board/src/service).

export interface Board {
  id: string;
  title: string;
  description: string | null;
  stack: string | null;
  schema_version: number;
  archived: boolean;
  created_at: string;
  updated_at: string;
}

export interface BoardSummary extends Board {
  item_count: number;
}

export type Actor = "user" | "claude" | "api";

export interface NoteContent {
  markdown: string;
}

export interface ChecklistItem {
  text: string;
  done: boolean;
  group?: string;
}

export interface ChecklistContent {
  items: ChecklistItem[];
}

export interface Item {
  id: string;
  board_id: string;
  kind: "card";
  card_type: "note" | "checklist";
  frame_id: string | null;
  title: string | null;
  content: NoteContent | ChecklistContent;
  style: { color?: string } | null;
  x: number;
  y: number;
  w: number;
  h: number;
  z: number;
  locked: boolean;
  version: number;
  created_by: Actor;
  updated_by: Actor;
  content_updated_at: string;
  deleted_at?: string;
  deleted_by?: Actor;
  created_at: string;
  updated_at: string;
}

export interface FullBoard {
  board: Board;
  items: Item[];
  connectors: unknown[];
  tables: unknown[];
}

export interface Revision {
  id: number;
  version: number;
  title: string | null;
  content: unknown;
  changed_by: Actor;
  created_at: string;
  updated_at: string;
}
