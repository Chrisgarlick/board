// State for the open board.
//
// - Each item and connector has its own signal, so a change re-renders only
//   what changed.
// - Structural changes (create, move, delete, connect) go through batches:
//   one request, one transaction, and the server reports side effects (frame
//   members moving, membership changing) so the UI never re-implements them.
// - Text and style edits go through a per-item save queue (spec 10.4):
//   debounced, one save in flight, each using the version the last returned.
// - Every user action records how to undo and redo it (spec 10.4).

import { batch, computed, signal, type Signal } from "@preact/signals";

import { api, ApiError, type Op } from "./api";
import type {
  Board,
  Connector,
  Item,
  SitemapContent,
  SitemapNode,
  Table,
  TableDefinition,
  TableWritten,
} from "./types";

const SAVE_DELAY_MS = 600;
const UNDO_LIMIT = 100;

export interface Conflict {
  /** The item as it is on the server now. */
  theirs: Item;
  /** What this tab was trying to save. */
  mine: ContentPatch;
}

export interface ContentPatch {
  title?: string;
  content?: unknown;
  /** `null` clears the style. */
  style?: unknown;
}

export interface ItemState {
  item: Signal<Item>;
  conflict: Signal<Conflict | null>;
  saving: Signal<boolean>;
}

export type Tool = "select" | "hand" | "rect" | "diamond" | "ellipse" | "sticky" | "text" | "frame" | "connector";

export const board = signal<Board | null>(null);
/** Non-frame items, bottom to top. */
export const itemIds = signal<string[]>([]);
/** Frames, bottom to top. Drawn beneath everything else. */
export const frameIds = signal<string[]>([]);
export const items = new Map<string, ItemState>();
export const connectorIds = signal<string[]>([]);
export const connectors = new Map<string, Signal<Connector>>();

export const selection = signal<ReadonlySet<string>>(new Set());
export const selectedConnector = signal<string | null>(null);
export const editingId = signal<string | null>(null);
export const tool = signal<Tool>("select");
export const status = signal<{ kind: "info" | "error"; text: string } | null>(null);
/** Items changed by someone else (another tab, Claude, a script) in the last few seconds. */
export const fresh = signal<ReadonlySet<string>>(new Set());
/** The item whose history panel is open. */
export const historyFor = signal<string | null>(null);
/** Set when the open board is deleted elsewhere. */
export const boardGone = signal(false);

/** The board schema: live tables by id. */
export const tables = signal<ReadonlyMap<string, Table>>(new Map());
export const schemaVersion = signal(0);
/** The last validation error for a table whose latest edit didn't save. */
export const tableErrors = signal<ReadonlyMap<string, { path: string; message: string }>>(new Map());
export const tableList = computed(() =>
  [...tables.value.values()].sort((a, b) => a.name.localeCompare(b.name)),
);

/** A page in one of the board's sitemaps. */
export interface Page {
  node: SitemapNode;
  depth: number;
  cardId: string;
}

/** Every page on the board's sitemap cards, depth first, cards in reading order. */
export const pages = computed<Page[]>(() => {
  const cards = itemIds.value
    .map((id) => items.get(id)?.item.value)
    .filter((i): i is Item => !!i && i.card_type === "sitemap")
    .sort((a, b) => a.y - b.y || a.x - b.x);
  const out: Page[] = [];
  for (const card of cards) {
    const walk = (node: SitemapNode, depth: number) => {
      out.push({ node, depth, cardId: card.id });
      for (const child of node.children ?? []) walk(child, depth + 1);
    };
    walk((card.content as SitemapContent).root, 0);
  }
  return out;
});

export const selectedItems = computed(() =>
  [...selection.value].map((id) => items.get(id)?.item.value).filter((i): i is Item => !!i),
);

export function report(err: unknown, action: string): void {
  const message = err instanceof ApiError ? err.body.message : String(err);
  status.value = { kind: "error", text: `Couldn't ${action}: ${message}` };
}

export function select(ids: Iterable<string>): void {
  batch(() => {
    selection.value = new Set(ids);
    selectedConnector.value = null;
  });
}

export function item(id: string): Item | undefined {
  return items.get(id)?.item.value;
}

/** A frame's members, as the UI currently knows them. */
export function membersOf(frameId: string): Item[] {
  return [...items.values()].map((s) => s.item.value).filter((i) => i.frame_id === frameId);
}

// ---------- Loading and applying server state ----------

export function load(b: Board, list: Item[], conns: Connector[], tableRows: Table[] = []): void {
  batch(() => {
    for (const q of queues.values()) q.cancel();
    queues.clear();
    for (const q of tableQueues.values()) q.cancel();
    tableQueues.clear();
    tables.value = new Map(tableRows.map((t) => [t.id, t]));
    tableErrors.value = new Map();
    schemaVersion.value = b.schema_version;
    items.clear();
    connectors.clear();
    for (const i of list) items.set(i.id, makeState(i));
    for (const c of conns) connectors.set(c.id, signal(c));
    board.value = b;
    resort();
    connectorIds.value = conns.map((c) => c.id);
    selection.value = new Set();
    selectedConnector.value = null;
    editingId.value = null;
    undoStack.length = 0;
    redoStack.length = 0;
    canUndo.value = false;
    canRedo.value = false;
    fresh.value = new Set();
    historyFor.value = null;
    boardGone.value = false;
  });
}

function makeState(i: Item): ItemState {
  return { item: signal(i), conflict: signal(null), saving: signal(false) };
}

function resort(): void {
  const list = [...items.values()].map((s) => s.item.value);
  const byZ = (a: Item, b: Item) => a.z - b.z || a.id.localeCompare(b.id);
  frameIds.value = list.filter((i) => i.kind === "frame").sort(byZ).map((i) => i.id);
  itemIds.value = list.filter((i) => i.kind !== "frame").sort(byZ).map((i) => i.id);
}

/** Whether this tab is editing an item's text right now (any editor inside it has focus). */
function editingHere(id: string): boolean {
  if (editingId.value === id) return true;
  const focused = document.activeElement;
  return !!focused && focused !== document.body && focused.closest("[data-item]")?.getAttribute("data-item") === id;
}

/**
 * Insert or update items from the server. While an item is being edited
 * here, or has unsaved changes, its text, style and version stay local: the
 * next save then conflicts and shows "changed elsewhere", instead of
 * silently overwriting what someone else did.
 */
function upsertItems(list: Item[]): void {
  for (const incoming of list) {
    const state = items.get(incoming.id);
    if (!state) {
      items.set(incoming.id, makeState(incoming));
      continue;
    }
    const queue = queues.get(incoming.id);
    if (queue?.busy() || editingHere(incoming.id)) {
      // Take geometry and membership from the server; keep the local text,
      // version and style.
      const local = state.item.value;
      state.item.value = {
        ...local,
        x: incoming.x,
        y: incoming.y,
        w: incoming.w,
        h: incoming.h,
        z: incoming.z,
        locked: incoming.locked,
        frame_id: incoming.frame_id,
      };
    } else {
      state.item.value = incoming;
    }
  }
}

function removeItems(ids: string[]): void {
  for (const id of ids) {
    queues.get(id)?.cancel();
    queues.delete(id);
    items.delete(id);
  }
  const gone = new Set(ids);
  if ([...selection.value].some((id) => gone.has(id))) {
    selection.value = new Set([...selection.value].filter((id) => !gone.has(id)));
  }
  if (editingId.value && gone.has(editingId.value)) editingId.value = null;
}

function upsertConnector(c: Connector): void {
  const existing = connectors.get(c.id);
  if (existing) existing.value = c;
  else {
    connectors.set(c.id, signal(c));
    connectorIds.value = [...connectorIds.value, c.id];
  }
}

function removeConnector(id: string): void {
  connectors.delete(id);
  connectorIds.value = connectorIds.value.filter((c) => c !== id);
  if (selectedConnector.value === id) selectedConnector.value = null;
}

function isItem(v: unknown): v is Item {
  return !!v && typeof v === "object" && "kind" in v && "board_id" in v;
}

function isConnector(v: unknown): v is Connector {
  return !!v && typeof v === "object" && "from_item" in v;
}

/** Run a batch and apply everything it returns. Returns the refs map. */
export async function runOps(ops: Op[]): Promise<Record<string, string>> {
  const b = board.value;
  if (!b || ops.length === 0) return {};
  const result = await api.batch(b.id, ops);
  batch(() => {
    const upserts: Item[] = [];
    for (const r of result.results) {
      if (isItem(r)) upserts.push(r);
      else if (isConnector(r)) upsertConnector(r);
      else if (r && typeof r === "object" && "deleted" in r) {
        removeItems((r as { deleted: string[] }).deleted);
      } else if (r && typeof r === "object" && "deleted_connector" in r) {
        removeConnector((r as { deleted_connector: string }).deleted_connector);
      }
    }
    upsertItems([...upserts, ...result.affected]);
    resort();
  });
  return result.refs;
}

// ---------- Live updates from elsewhere ----------

const FRESH_MS = 2500;

/** A change published by the server (crates/board/src/events.rs). */
export interface RemoteChange {
  type: string;
  item?: Item;
  connector?: Connector;
  id?: string;
  board?: Board;
  table?: Table;
  schema_version?: number;
}

/** Apply changes made elsewhere: other tabs, Claude, scripts. */
export function applyRemote(changes: RemoteChange[]): void {
  const b = board.value;
  if (!b) return;
  const touched: string[] = [];
  batch(() => {
    const upserts: Item[] = [];
    for (const c of changes) {
      switch (c.type) {
        case "item.created":
        case "item.updated":
        case "item.geometry":
        case "item.undeleted":
        case "item.moved":
          if (!c.item) break;
          if (c.item.board_id !== b.id) {
            removeItems([c.item.id]); // moved to another board
          } else {
            upserts.push(c.item);
            touched.push(c.item.id);
          }
          break;
        case "item.deleted":
          if (c.id) removeItems([c.id]);
          break;
        case "connector.created":
        case "connector.updated":
          if (c.connector) upsertConnector(c.connector);
          break;
        case "connector.deleted":
          if (c.id) removeConnector(c.id);
          break;
        case "board.updated":
          if (c.board) board.value = c.board;
          break;
        case "board.deleted":
          boardGone.value = true;
          break;
        case "table.created":
        case "table.updated":
        case "table.undeleted":
          if (c.table) upsertTables([c.table]);
          break;
        case "table.deleted":
          if (c.id) removeTable(c.id);
          break;
      }
      if (c.schema_version !== undefined) schemaVersion.value = Math.max(schemaVersion.value, c.schema_version);
    }
    upsertItems(upserts);
    resort();
    if (touched.length) fresh.value = new Set([...fresh.value, ...touched]);
  });
  if (touched.length) {
    setTimeout(() => {
      const next = new Set(fresh.value);
      for (const id of touched) next.delete(id);
      fresh.value = next;
    }, FRESH_MS);
  }
}

/** Reload the board after missing updates, keeping the view, selection and undo history. */
export async function reload(): Promise<void> {
  const b = board.value;
  if (!b) return;
  try {
    const full = await api.getBoard(b.id);
    batch(() => {
      const keep = new Set(full.items.map((i) => i.id));
      for (const id of [...items.keys()]) if (!keep.has(id)) removeItems([id]);
      upsertItems(full.items);
      connectors.clear();
      for (const c of full.connectors) connectors.set(c.id, signal(c));
      connectorIds.value = full.connectors.map((c) => c.id);
      board.value = full.board;
      const keepTables = new Set(full.tables.map((t) => t.id));
      for (const id of tables.value.keys()) if (!keepTables.has(id)) removeTable(id);
      upsertTables(full.tables);
      schemaVersion.value = full.board.schema_version;
      resort();
    });
  } catch (err) {
    if (err instanceof ApiError && err.status === 404) boardGone.value = true;
    else report(err, "refresh the board");
  }
}

// ---------- Undo and redo ----------

/** One step of an undo entry: batch ops, or a text/style edit through the save queue. */
type Step =
  | { ops: Op[] }
  | { edit: { id: string; patch: ContentPatch } }
  | { table: { id: string; patch: TablePatch } }
  | { run: () => Promise<void> };

interface Entry {
  undo: Step[];
  redo: Step[];
}

const undoStack: Entry[] = [];
const redoStack: Entry[] = [];
export const canUndo = signal(false);
export const canRedo = signal(false);

function remember(entry: Entry): void {
  undoStack.push(entry);
  if (undoStack.length > UNDO_LIMIT) undoStack.shift();
  redoStack.length = 0;
  canUndo.value = true;
  canRedo.value = false;
}

async function runSteps(steps: Step[]): Promise<void> {
  for (const step of steps) {
    if ("ops" in step) await runOps(step.ops);
    else if ("edit" in step) editContent(step.edit.id, step.edit.patch, true);
    else if ("table" in step) editTable(step.table.id, step.table.patch, true);
    else await step.run();
  }
}

export async function undo(): Promise<void> {
  const entry = undoStack.pop();
  canUndo.value = undoStack.length > 0;
  if (!entry) return;
  try {
    await runSteps(entry.undo);
    redoStack.push(entry);
    canRedo.value = true;
  } catch (err) {
    report(err, "undo");
  }
}

export async function redo(): Promise<void> {
  const entry = redoStack.pop();
  canRedo.value = redoStack.length > 0;
  if (!entry) return;
  try {
    await runSteps(entry.redo);
    undoStack.push(entry);
    canUndo.value = true;
  } catch (err) {
    report(err, "redo");
  }
}

// ---------- User actions ----------

export interface NewItem {
  /** Local name, so connectors in the same call can refer to it. */
  ref?: string;
  kind: Item["kind"];
  card_type?: string;
  title?: string;
  content: unknown;
  style?: unknown;
  x: number;
  y: number;
  w?: number;
  h?: number;
}

export interface NewConnector {
  from: string;
  to: string;
  from_anchor?: string;
  to_anchor?: string;
  label?: string | null;
  style?: unknown;
}

/** Create items (and connectors between them or existing items). Returns the new item ids, in order. */
export async function create(newItems: NewItem[], newConnectors: NewConnector[] = []): Promise<string[]> {
  const ops: Op[] = newItems.map((n, i) => {
    const { ref, ...rest } = n;
    return { op: "create_item", ref: ref ?? `item${i}`, ...rest, x: Math.round(n.x), y: Math.round(n.y) };
  });
  newConnectors.forEach((c, i) => ops.push({ op: "create_connector", ref: `conn${i}`, ...c }));
  try {
    const refs = await runOps(ops);
    const itemIdsCreated = newItems.map((n, i) => refs[n.ref ?? `item${i}`] as string);
    const connectorIdsCreated = newConnectors.map((_, i) => refs[`conn${i}`] as string);
    remember({
      undo: [
        {
          ops: [
            ...connectorIdsCreated.map((id) => ({ op: "delete_connector", connector: id })),
            ...itemIdsCreated.map((id) => ({ op: "delete_item", item: id })),
          ],
        },
      ],
      redo: [
        {
          ops: [
            ...itemIdsCreated.map((id) => ({ op: "undelete_item", item: id })),
            ...connectorIdsCreated.map((id) => ({ op: "undelete_connector", connector: id })),
          ],
        },
      ],
    });
    return itemIdsCreated;
  } catch (err) {
    report(err, "add that");
    return [];
  }
}

export interface Placement {
  id: string;
  x?: number;
  y?: number;
  w?: number;
  h?: number;
  z?: number;
  locked?: boolean;
}

/** Move, resize, restack or lock items, as one undoable step. */
export async function place(changes: Placement[]): Promise<void> {
  if (changes.length === 0) return;
  const before: Placement[] = changes.map((c) => {
    const i = item(c.id);
    const prev: Placement = { id: c.id };
    if (!i) return prev;
    if (c.x !== undefined) prev.x = i.x;
    if (c.y !== undefined) prev.y = i.y;
    if (c.w !== undefined) prev.w = i.w;
    if (c.h !== undefined) prev.h = i.h;
    if (c.z !== undefined) prev.z = i.z;
    if (c.locked !== undefined) prev.locked = i.locked;
    return prev;
  });
  const ops = (list: Placement[]) => list.map(({ id, ...g }) => ({ op: "set_geometry", item: id, ...g }));

  // Show the result straight away; the server's answer replaces it.
  batch(() => {
    for (const c of changes) {
      const state = items.get(c.id);
      if (!state) continue;
      const { id: _id, ...g } = c;
      state.item.value = { ...state.item.value, ...g };
    }
    resort();
  });
  try {
    await runOps(ops(changes));
    remember({ undo: [{ ops: ops(before) }], redo: [{ ops: ops(changes) }] });
  } catch (err) {
    batch(() => {
      for (const b of before) {
        const state = items.get(b.id);
        if (!state) continue;
        const { id: _id, ...g } = b;
        state.item.value = { ...state.item.value, ...g };
      }
      resort();
    });
    report(err, "move that");
  }
}

/** Delete items (and/or a connector) as one undoable step. */
export async function remove(ids: string[], withContents = false, connectorId?: string | null): Promise<void> {
  const ops: Op[] = ids.map((id) => ({ op: "delete_item", item: id, with_contents: withContents }));
  if (connectorId) ops.unshift({ op: "delete_connector", connector: connectorId });
  if (ops.length === 0) return;
  // Everything that will go, for undo: the items, plus frame contents.
  const gone = new Set(ids);
  if (withContents) for (const id of ids) for (const m of membersOf(id)) gone.add(m.id);
  // Don't let a pending text save land on an item that's about to be deleted.
  for (const id of gone) queues.get(id)?.cancel();
  try {
    await runOps(ops);
    const restore: Op[] = [...gone].map((id) => ({ op: "undelete_item", item: id }));
    if (connectorId) restore.push({ op: "undelete_connector", connector: connectorId });
    remember({ undo: [{ ops: restore }], redo: [{ ops }] });
    const n = gone.size + (connectorId ? 1 : 0);
    status.value = { kind: "info", text: n === 1 ? "Moved to the trash." : `Moved ${n} things to the trash.` };
  } catch (err) {
    report(err, "delete that");
  }
}

export async function restoreFromTrash(id: string): Promise<void> {
  try {
    await runOps([{ op: "undelete_item", item: id }]);
    select([id]);
  } catch (err) {
    report(err, "restore that");
  }
}

/** Put an item back to an earlier revision (a normal versioned change on the server). */
export async function restoreRevision(id: string, revision: number): Promise<void> {
  const state = items.get(id);
  if (!state) return;
  queues.get(id)?.cancel();
  try {
    const restored = await api.restoreRevision(id, revision, state.item.value.version);
    state.item.value = restored;
    status.value = { kind: "info", text: "Restored that version." };
  } catch (err) {
    report(err, "restore that version");
  }
}

export async function restoreConnector(id: string): Promise<void> {
  try {
    await runOps([{ op: "undelete_connector", connector: id }]);
  } catch (err) {
    report(err, "restore that");
  }
}

/** Change connectors' anchors, label or style, as one undoable step. */
export async function updateConnector(id: string, patch: Partial<Connector>): Promise<void> {
  const c = connectors.get(id)?.value;
  if (!c) return;
  const before: Record<string, unknown> = {};
  const after: Record<string, unknown> = {};
  for (const key of ["from_anchor", "to_anchor", "label", "style"] as const) {
    if (key in patch) {
      before[key] = c[key] ?? null;
      after[key] = patch[key] ?? null;
    }
  }
  connectors.get(id)!.value = { ...c, ...patch };
  const op = (p: Record<string, unknown>) => [{ op: "update_connector", connector: id, ...p }];
  try {
    await runOps(op(after));
    remember({ undo: [{ ops: op(before) }], redo: [{ ops: op(after) }] });
  } catch (err) {
    connectors.get(id)!.value = c;
    report(err, "change the connector");
  }
}

/**
 * Record a text or style change for undo. Call when an edit session ends,
 * with the values from when it started.
 */
export function rememberEdit(id: string, before: ContentPatch, after: ContentPatch): void {
  if (JSON.stringify(before) === JSON.stringify(after)) return;
  remember({ undo: [{ edit: { id, patch: before } }], redo: [{ edit: { id, patch: after } }] });
}

/** Change items' style (merged into each item's existing style), as one undoable step. */
export function restyle(ids: string[], patch: Record<string, unknown>): void {
  const undo: Step[] = [];
  const redo: Step[] = [];
  for (const id of ids) {
    const i = item(id);
    if (!i) continue;
    const before = i.style ?? null;
    const merged: Record<string, unknown> = { ...(i.style ?? {}), ...patch };
    for (const k of Object.keys(merged)) if (merged[k] === undefined || merged[k] === null) delete merged[k];
    const after = Object.keys(merged).length ? merged : null;
    editContent(id, { style: after }, true);
    undo.push({ edit: { id, patch: { style: before } } });
    redo.push({ edit: { id, patch: { style: after } } });
  }
  if (undo.length) remember({ undo, redo });
}

/**
 * Change items' content fields (merged), e.g. a shape's type, as one
 * undoable step. A field set to `undefined` is removed.
 */
export function reshape(ids: string[], patch: Record<string, unknown>): void {
  const undo: Step[] = [];
  const redo: Step[] = [];
  for (const id of ids) {
    const i = item(id);
    if (!i) continue;
    const before = i.content;
    const after: Record<string, unknown> = { ...(i.content as object), ...patch };
    for (const k of Object.keys(after)) if (after[k] === undefined) delete after[k];
    editContent(id, { content: after }, true);
    undo.push({ edit: { id, patch: { content: before } } });
    redo.push({ edit: { id, patch: { content: after } } });
  }
  if (undo.length) remember({ undo, redo });
}

// ---------- Content saves ----------

class SaveQueue {
  private pending: ContentPatch | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private inFlight = false;

  constructor(private readonly id: string) {}

  busy(): boolean {
    return this.inFlight || this.pending !== null;
  }

  /** Queue a change. Later changes to the same field replace earlier ones. */
  push(patch: ContentPatch, immediate: boolean): void {
    this.pending = { ...(this.pending ?? {}), ...patch };
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.flush(), immediate ? 0 : SAVE_DELAY_MS);
  }

  cancel(): void {
    if (this.timer) clearTimeout(this.timer);
    this.pending = null;
  }

  private async flush(): Promise<void> {
    this.timer = null;
    const state = items.get(this.id);
    if (!state || this.inFlight || !this.pending || state.conflict.value) return;

    const patch = this.pending;
    this.pending = null;
    this.inFlight = true;
    state.saving.value = true;
    try {
      const saved = await api.updateItem(this.id, { version: state.item.value.version, ...patch });
      // The server's copy, plus any edits made while this save was in flight.
      const current = state.item.value;
      const merged = this.pending ? applyLocal(saved, this.pending) : saved;
      // Geometry may have changed locally meanwhile (a drag); keep it.
      state.item.value = { ...merged, x: current.x, y: current.y, w: current.w, h: current.h, z: current.z };
    } catch (err) {
      if (err instanceof ApiError && err.status === 404) {
        // Deleted (here or elsewhere) while the save was on its way: nothing to save.
        this.pending = null;
      } else if (err instanceof ApiError && err.status === 409 && err.body.current) {
        state.conflict.value = {
          theirs: err.body.current as Item,
          mine: { ...patch, ...(this.pending ?? {}) },
        };
        this.pending = null;
      } else {
        // Keep the change and try again on the next edit.
        this.pending = { ...patch, ...(this.pending ?? {}) };
        report(err, "save");
      }
    } finally {
      this.inFlight = false;
      state.saving.value = false;
    }
    if (this.pending) void this.flush();
  }
}

function applyLocal(i: Item, patch: ContentPatch): Item {
  return {
    ...i,
    ...(patch.title !== undefined ? { title: patch.title } : {}),
    ...(patch.content !== undefined ? { content: patch.content as Item["content"] } : {}),
    ...(patch.style !== undefined ? { style: patch.style as Item["style"] } : {}),
  };
}

const queues = new Map<string, SaveQueue>();

/** Change an item's title, content or style locally and save it (debounced). */
export function editContent(id: string, patch: ContentPatch, immediate = false): void {
  const state = items.get(id);
  if (!state) return;
  state.item.value = applyLocal(state.item.value, patch);
  let queue = queues.get(id);
  if (!queue) {
    queue = new SaveQueue(id);
    queues.set(id, queue);
  }
  queue.push(patch, immediate);
}

/** Resolve a conflict by taking the server's version. */
export function takeTheirs(id: string): void {
  const state = items.get(id);
  const conflict = state?.conflict.value;
  if (!state || !conflict) return;
  batch(() => {
    state.item.value = conflict.theirs;
    state.conflict.value = null;
  });
}

/** Resolve a conflict by saving this tab's version over the server's. */
export function keepMine(id: string): void {
  const state = items.get(id);
  const conflict = state?.conflict.value;
  if (!state || !conflict) return;
  batch(() => {
    state.item.value = applyLocal(conflict.theirs, conflict.mine);
    state.conflict.value = null;
  });
  editContent(id, conflict.mine, true);
}

export function topZ(): number {
  let top = 0;
  for (const s of items.values()) top = Math.max(top, s.item.value.z);
  return top;
}

export function bottomZ(): number {
  let bottom = 0;
  for (const s of items.values()) bottom = Math.min(bottom, s.item.value.z);
  return bottom;
}

// ---------- The board schema ----------

export interface TablePatch {
  name?: string;
  definition?: TableDefinition;
}

function upsertTables(list: Table[]): void {
  const next = new Map(tables.value);
  for (const t of list) {
    // Keep local edits that haven't saved yet; the next save then conflicts
    // with the newer version instead of overwriting it.
    if (tableQueues.get(t.id)?.busy()) continue;
    next.set(t.id, t);
  }
  tables.value = next;
}

function removeTable(id: string): void {
  tableQueues.get(id)?.cancel();
  tableQueues.delete(id);
  const next = new Map(tables.value);
  next.delete(id);
  tables.value = next;
}

function setTableError(id: string, error: { path: string; message: string } | null): void {
  const next = new Map(tableErrors.value);
  if (error) next.set(id, error);
  else next.delete(id);
  tableErrors.value = next;
}

/** Apply what a schema write returned: the table, side effects, the card. */
function applyWritten(w: TableWritten): void {
  batch(() => {
    if (w.table.deleted_at) removeTable(w.table.id);
    else upsertTables([w.table]);
    if (w.updated) upsertTables(w.updated);
    if (w.item) takeOwn(w.item);
    schemaVersion.value = Math.max(schemaVersion.value, w.schema_version);
  });
}

/**
 * Apply an item from the response to this tab's own request. Unlike remote
 * changes, it applies while the item has focus (the button that caused it is
 * usually inside it), unless the item has unsaved edits of its own.
 */
function takeOwn(i: Item): void {
  const state = items.get(i.id);
  if (!state) return upsertItems([i]);
  if (queues.get(i.id)?.busy()) return;
  state.item.value = i;
}

class TableQueue {
  private pending: TablePatch | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private inFlight = false;

  constructor(private readonly id: string) {}

  busy(): boolean {
    return this.inFlight || this.pending !== null;
  }

  push(patch: TablePatch, immediate: boolean): void {
    this.pending = { ...(this.pending ?? {}), ...patch };
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.flush(), immediate ? 0 : SAVE_DELAY_MS);
  }

  cancel(): void {
    if (this.timer) clearTimeout(this.timer);
    this.pending = null;
  }

  private async flush(): Promise<void> {
    this.timer = null;
    const current = tables.value.get(this.id);
    if (!current || this.inFlight || !this.pending) return;
    const patch = this.pending;
    this.pending = null;
    this.inFlight = true;
    try {
      const w = await api.updateTable(this.id, { version: current.version, ...patch });
      setTableError(this.id, null);
      const later = this.pending as TablePatch | null;
      this.inFlight = false;
      if (later) {
        // More edits arrived meanwhile: take the new version, keep them local.
        const next = new Map(tables.value);
        next.set(this.id, { ...w.table, ...later, definition: later.definition ?? w.table.definition });
        tables.value = next;
        if (w.updated) upsertTables(w.updated);
        schemaVersion.value = Math.max(schemaVersion.value, w.schema_version);
      } else {
        applyWritten(w);
      }
    } catch (err) {
      this.inFlight = false;
      if (err instanceof ApiError && err.status === 422) {
        // Keep the edit on screen and say what's wrong; the next edit retries.
        setTableError(this.id, { path: err.body.path ?? "", message: err.body.message });
      } else if (err instanceof ApiError && err.status === 409 && err.body.current) {
        this.pending = null;
        upsertTables([err.body.current as Table]);
        setTableError(this.id, null);
        status.value = { kind: "error", text: "That table was changed elsewhere, so it's been reloaded. Make your change again." };
      } else if (err instanceof ApiError && err.status === 404) {
        removeTable(this.id);
      } else {
        this.pending = { ...patch, ...(this.pending ?? {}) };
        report(err, "save the table");
      }
    }
    if (this.pending) void this.flush();
  }
}

const tableQueues = new Map<string, TableQueue>();

/** Change a table locally and save it (debounced). */
export function editTable(id: string, patch: TablePatch, immediate = false): void {
  const t = tables.value.get(id);
  if (!t) return;
  const next = new Map(tables.value);
  next.set(id, { ...t, ...patch, definition: patch.definition ?? t.definition });
  tables.value = next;
  let queue = tableQueues.get(id);
  if (!queue) {
    queue = new TableQueue(id);
    tableQueues.set(id, queue);
  }
  queue.push(patch, immediate);
}

/** Record a table edit for undo. Call when an edit ends, with the values from when it started. */
export function rememberTable(id: string, before: TablePatch, after: TablePatch): void {
  if (JSON.stringify(before) === JSON.stringify(after)) return;
  remember({ undo: [{ table: { id, patch: before } }], redo: [{ table: { id, patch: after } }] });
}

/** Create a table, optionally showing it on a schema card. Returns its id. */
export async function createTable(
  name: string,
  definition: TableDefinition,
  cardId?: string,
): Promise<string | null> {
  const b = board.value;
  if (!b) return null;
  try {
    const w = await api.createTable(b.id, { name, definition, ...(cardId ? { item_id: cardId } : {}) });
    applyWritten(w);
    const id = w.table.id;
    remember({
      undo: [{ run: async () => applyWritten(await api.deleteTable(id, true)) }],
      redo: [{ run: async () => applyWritten(await api.undeleteTable(id)) }],
    });
    return id;
  } catch (err) {
    report(err, "add the table");
    return null;
  }
}

/** Move a table to the trash. Asks first when other tables reference it. */
export async function deleteTable(id: string): Promise<void> {
  const t = tables.value.get(id);
  if (!t) return;
  tableQueues.get(id)?.cancel();
  try {
    let w: TableWritten;
    try {
      w = await api.deleteTable(id, false);
    } catch (err) {
      if (!(err instanceof ApiError && err.body.error === "table_referenced")) throw err;
      if (!confirm(`${err.body.message}\n\nDelete ${t.name} and remove those references?`)) return;
      w = await api.deleteTable(id, true);
    }
    applyWritten(w);
    remember({
      undo: [{ run: async () => applyWritten(await api.undeleteTable(id)) }],
      redo: [{ run: async () => applyWritten(await api.deleteTable(id, true)) }],
    });
    status.value = { kind: "info", text: `Moved ${t.name} to the trash.` };
  } catch (err) {
    report(err, "delete the table");
  }
}

export async function restoreTable(id: string): Promise<boolean> {
  try {
    applyWritten(await api.undeleteTable(id));
    return true;
  } catch (err) {
    report(err, "restore the table");
    return false;
  }
}

/** The schema cards that show a table. */
export function cardsShowing(tableId: string): Item[] {
  return itemIds.value
    .map((id) => items.get(id)?.item.value)
    .filter(
      (i): i is Item =>
        !!i && i.card_type === "schema" && ((i.content as { table_ids?: string[] }).table_ids ?? []).includes(tableId),
    );
}

/** Show or hide tables on a schema card, as one undoable step. */
export function setCardTables(cardId: string, tableIds: string[]): void {
  const card = item(cardId);
  if (!card) return;
  const before = { content: card.content };
  const after = { content: { table_ids: tableIds } };
  editContent(cardId, after, true);
  rememberEdit(cardId, before, after);
}
