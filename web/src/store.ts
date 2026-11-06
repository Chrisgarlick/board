// State for the open board.
//
// Each item has its own signal, so editing one card re-renders that card only.
// Content saves go through a per-item queue (spec section 10.4): saves are
// debounced, only one is in flight at a time, and each uses the version the
// previous one returned, so a tab never conflicts with itself.

import { batch, signal, type Signal } from "@preact/signals";

import { api, ApiError, type Geometry } from "./api";
import type { Board, Item } from "./types";

const SAVE_DELAY_MS = 600;

export interface Conflict {
  /** The item as it is on the server now. */
  theirs: Item;
  /** What this tab was trying to save. */
  mine: ContentPatch;
}

export interface ContentPatch {
  title?: string;
  content?: unknown;
}

export interface ItemState {
  item: Signal<Item>;
  conflict: Signal<Conflict | null>;
  saving: Signal<boolean>;
}

export const board = signal<Board | null>(null);
export const itemIds = signal<string[]>([]);
export const items = new Map<string, ItemState>();
export const selectedId = signal<string | null>(null);
export const status = signal<{ kind: "info" | "error"; text: string } | null>(null);

export function report(err: unknown, action: string): void {
  const message = err instanceof ApiError ? err.body.message : String(err);
  status.value = { kind: "error", text: `Couldn't ${action}: ${message}` };
}

// ---------- Loading ----------

export function load(b: Board, list: Item[]): void {
  batch(() => {
    for (const q of queues.values()) q.cancel();
    queues.clear();
    items.clear();
    for (const item of list) items.set(item.id, makeState(item));
    board.value = b;
    itemIds.value = sortByZ(list).map((i) => i.id);
    selectedId.value = null;
  });
}

function makeState(item: Item): ItemState {
  return { item: signal(item), conflict: signal(null), saving: signal(false) };
}

function sortByZ(list: Item[]): Item[] {
  return [...list].sort((a, b) => a.z - b.z || a.id.localeCompare(b.id));
}

function resort(): void {
  const list = [...items.values()].map((s) => s.item.value);
  itemIds.value = sortByZ(list).map((i) => i.id);
}

export function topZ(): number {
  let top = 0;
  for (const s of items.values()) top = Math.max(top, s.item.value.z);
  return top;
}

// ---------- Create and delete ----------

export async function createCard(
  cardType: "note" | "checklist",
  x: number,
  y: number,
): Promise<void> {
  const b = board.value;
  if (!b) return;
  const content = cardType === "note" ? { markdown: "" } : { items: [{ text: "", done: false }] };
  const title = cardType === "note" ? "Note" : "Checklist";
  try {
    const item = await api.createItem(b.id, {
      kind: "card",
      card_type: cardType,
      title,
      content,
      x: Math.round(x),
      y: Math.round(y),
    });
    batch(() => {
      items.set(item.id, makeState(item));
      resort();
      selectedId.value = item.id;
    });
  } catch (err) {
    report(err, "add the card");
  }
}

export async function deleteItem(id: string): Promise<void> {
  const state = items.get(id);
  if (!state) return;
  queues.get(id)?.cancel();
  queues.delete(id);
  batch(() => {
    items.delete(id);
    resort();
    if (selectedId.value === id) selectedId.value = null;
  });
  try {
    await api.deleteItem(id);
    status.value = { kind: "info", text: `Moved “${state.item.value.title ?? "item"}” to the trash.` };
  } catch (err) {
    // Put it back so what's on screen matches the server.
    batch(() => {
      items.set(id, state);
      resort();
    });
    report(err, "delete the card");
  }
}

export async function undelete(id: string): Promise<void> {
  try {
    const item = await api.undeleteItem(id);
    batch(() => {
      items.set(item.id, makeState(item));
      resort();
      selectedId.value = item.id;
    });
  } catch (err) {
    report(err, "restore the card");
  }
}

// ---------- Geometry ----------

/** Save a move or resize. Last write wins, so no version is needed. */
export async function saveGeometry(id: string, g: Geometry): Promise<void> {
  const state = items.get(id);
  if (!state) return;
  const before = state.item.value;
  state.item.value = { ...before, ...g };
  if (g.z !== undefined) resort();
  try {
    const saved = await api.setGeometry(id, g);
    // Keep any content edits made meanwhile; take geometry from the server.
    const now = state.item.value;
    state.item.value = {
      ...now,
      x: saved.x,
      y: saved.y,
      w: saved.w,
      h: saved.h,
      z: saved.z,
      locked: saved.locked,
    };
  } catch (err) {
    state.item.value = { ...state.item.value, x: before.x, y: before.y, w: before.w, h: before.h };
    report(err, "move the card");
  }
}

// ---------- Content ----------

class SaveQueue {
  private pending: ContentPatch | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private inFlight = false;

  constructor(private readonly id: string) {}

  /** Queue a change. Later changes to the same field replace earlier ones. */
  push(patch: ContentPatch, immediate: boolean): void {
    this.pending = { ...this.pending, ...patch };
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
      // The server's copy, plus any edits typed while this save was in flight.
      state.item.value = this.pending ? applyLocal(saved, this.pending) : saved;
    } catch (err) {
      if (err instanceof ApiError && err.status === 409 && err.body.current) {
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

function applyLocal(item: Item, patch: ContentPatch): Item {
  return {
    ...item,
    ...(patch.title !== undefined ? { title: patch.title } : {}),
    ...(patch.content !== undefined ? { content: patch.content as Item["content"] } : {}),
  };
}

const queues = new Map<string, SaveQueue>();

/** Change an item's title or content locally and save it (debounced). */
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
