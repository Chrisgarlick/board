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
  table_count: number;
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

export type Kind = "card" | "shape" | "sticky" | "text" | "frame";
export type CardType = "note" | "checklist" | "schema" | "sitemap";
export type ShapeKind = "rect" | "rounded" | "diamond" | "ellipse" | "parallelogram" | "cylinder" | "document";
export type Color = "grey" | "blue" | "green" | "yellow" | "orange" | "red" | "purple" | "pink";
export type Align = "left" | "center" | "right";
export type TextSize = "s" | "m" | "l" | "xl";

export interface ShapeContent {
  shape: ShapeKind;
  text: string;
  /** The sitemap page this journey step is on. */
  page_id?: string;
}

export interface SchemaCardContent {
  table_ids: string[];
}

export interface SitemapNode {
  id: string;
  title: string;
  slug?: string;
  purpose?: string;
  children?: SitemapNode[];
}

export interface SitemapContent {
  root: SitemapNode;
}

// ---------- The board schema (crates/board-model) ----------

export const COLUMN_TYPES = [
  "string",
  "text",
  "integer",
  "bigint",
  "decimal",
  "boolean",
  "date",
  "time",
  "datetime",
  "uuid",
  "json",
  "enum",
  "binary",
] as const;
export type ColumnType = (typeof COLUMN_TYPES)[number];
export type Generated = "increment" | "uuid" | "now" | "now_on_update";

export interface Reference {
  table_id: string;
  column_id: string;
  /** Filled in on read. */
  table?: string;
  column?: string;
}

export interface Column {
  /** Absent only on a column not yet saved. */
  id?: string;
  name: string;
  type: ColumnType;
  length?: number;
  precision?: number;
  scale?: number;
  values?: string[];
  nullable: boolean;
  default?: string;
  primary: boolean;
  unique: boolean;
  generated?: Generated;
  references?: Reference;
  label?: string;
  comment?: string;
  native_type?: string;
}

export interface TableIndex {
  column_ids: string[];
  unique: boolean;
  columns?: string[];
}

export interface TableDefinition {
  comment?: string;
  columns: Column[];
  indexes: TableIndex[];
}

export interface Table {
  id: string;
  board_id: string;
  name: string;
  definition: TableDefinition;
  version: number;
  created_by: Actor;
  updated_by: Actor;
  content_updated_at: string;
  deleted_at?: string;
  deleted_by?: Actor;
  created_at: string;
  updated_at: string;
}

export interface Warning {
  path: string;
  message: string;
}

/** What a schema write returns (crates/board/src/service/tables.rs). */
export interface TableWritten {
  table: Table;
  schema_version: number;
  warnings?: Warning[];
  updated?: Table[];
  item?: Item;
}

export interface StickyContent {
  text: string;
}

export interface TextContent {
  text: string;
  size: TextSize;
}

export interface FrameContent {
  description?: string;
}

export interface ItemStyle {
  color?: Color;
  align?: Align;
}

export interface Item {
  id: string;
  board_id: string;
  kind: Kind;
  card_type: CardType | null;
  frame_id: string | null;
  title: string | null;
  content:
    | NoteContent
    | ChecklistContent
    | SchemaCardContent
    | SitemapContent
    | ShapeContent
    | StickyContent
    | TextContent
    | FrameContent;
  style: ItemStyle | null;
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

export type Side = "top" | "right" | "bottom" | "left";
export type AnchorSpec = "auto" | Side;

export interface ConnectorStyle {
  line?: "solid" | "dashed" | "dotted";
  start?: "none" | "arrow" | "open_arrow" | "dot";
  end?: "none" | "arrow" | "open_arrow" | "dot";
  routing?: "straight" | "elbow" | "curved";
  color?: Color;
}

export interface Connector {
  id: string;
  board_id: string;
  from_item: string;
  to_item: string;
  from_anchor: AnchorSpec;
  to_anchor: AnchorSpec;
  label: string | null;
  style: ConnectorStyle | null;
  created_by: Actor;
  deleted_at?: string;
  created_at: string;
  updated_at: string;
}

export interface FullBoard {
  board: Board;
  items: Item[];
  connectors: Connector[];
  tables: Table[];
}

export interface Revision {
  id: number;
  version: number;
  title: string | null;
  content: unknown;
  style?: unknown;
  changed_by: Actor;
  created_at: string;
  updated_at: string;
}
