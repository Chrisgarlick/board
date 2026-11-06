// Copy, cut, paste and duplicate (spec 10.2). Uses the browser's copy and
// paste events (no clipboard permission prompt), so it works across tabs and
// boards. The clipboard holds plain JSON with a marker prefix.

import { pageId } from "../ids";
import type { Item, SitemapNode } from "../types";
import {
  board,
  connectors,
  create,
  item,
  membersOf,
  remove,
  select,
  selection,
  tables,
  type NewConnector,
  type NewItem,
} from "../store";

const MARKER = "board-clipboard/1:";
const PASTE_OFFSET = 20;

interface Clip {
  items: (NewItem & { ref: string })[];
  connectors: NewConnector[];
  /** The board it was copied from, and its table names, for pasting elsewhere. */
  board?: string;
  tableNames?: Record<string, string>;
}

/** The selection (plus frame contents) and the connectors between them. */
function collect(): Clip | null {
  const ids = new Set<string>();
  for (const id of selection.value) {
    ids.add(id);
    const i = item(id);
    if (i?.kind === "frame") for (const m of membersOf(id)) ids.add(m.id);
  }
  const list = [...ids].map(item).filter((i): i is Item => !!i);
  if (list.length === 0) return null;
  const items = list.map((i) => {
    const n: NewItem & { ref: string } = { ref: i.id, kind: i.kind, content: i.content, x: i.x, y: i.y, w: i.w, h: i.h };
    if (i.card_type) n.card_type = i.card_type;
    if (i.title !== null) n.title = i.title;
    if (i.style) n.style = i.style;
    return n;
  });
  const conns: NewConnector[] = [];
  for (const c of connectors.values()) {
    const v = c.value;
    if (ids.has(v.from_item) && ids.has(v.to_item)) {
      conns.push({
        from: v.from_item,
        to: v.to_item,
        from_anchor: v.from_anchor,
        to_anchor: v.to_anchor,
        label: v.label,
        ...(v.style ? { style: v.style } : {}),
      });
    }
  }
  const tableNames: Record<string, string> = {};
  for (const t of tables.value.values()) tableNames[t.id] = t.name;
  return { items, connectors: conns, ...(board.value ? { board: board.value.id } : {}), tableNames };
}

/** A sitemap tree with new page ids, so a copy doesn't claim the original's pages. */
function freshPages(node: SitemapNode): SitemapNode {
  return { ...node, id: pageId(), children: (node.children ?? []).map(freshPages) };
}

/**
 * Content as it should be pasted here. Page ids are always new. On another
 * board, page links are dropped and schema cards keep the tables that exist
 * there (by name), since ids from the other board mean nothing here.
 */
function adapt(n: NewItem, clip: Clip): NewItem {
  const elsewhere = clip.board !== undefined && clip.board !== board.value?.id;
  if (n.card_type === "sitemap") {
    const c = n.content as { root: SitemapNode };
    return { ...n, content: { root: freshPages(c.root) } };
  }
  if (!elsewhere) return n;
  if (n.kind === "shape") {
    const { page_id: _gone, ...rest } = n.content as { page_id?: string };
    return { ...n, content: rest };
  }
  if (n.card_type === "feature") {
    // Pages and table ids from another board mean nothing here.
    const c = n.content as { table_ids?: string[] };
    const here = new Set([...tables.value.values()].map((t) => t.name.toLowerCase()));
    const names = (c.table_ids ?? []).map((id) => clip.tableNames?.[id]).filter((name): name is string => !!name && here.has(name.toLowerCase()));
    return { ...n, content: { ...(n.content as object), table_ids: [], page_ids: [], tables: names } };
  }
  if (n.card_type === "schema") {
    const here = new Set([...tables.value.values()].map((t) => t.name.toLowerCase()));
    const ids = (n.content as { table_ids?: string[] }).table_ids ?? [];
    const names = ids.map((id) => clip.tableNames?.[id]).filter((name): name is string => !!name && here.has(name.toLowerCase()));
    return { ...n, content: { tables: names } };
  }
  return n;
}

let pasteCount = 0;

/** Create a copy of a clip, shifted by `offset`, and select it. */
async function insert(clip: Clip, offset: number): Promise<void> {
  // Refs are the original ids; the batch maps them to the new ones.
  const created = await create(
    clip.items.map((n) => ({ ...adapt(n, clip), x: n.x + offset, y: n.y + offset })),
    clip.connectors,
  );
  if (created.length) select(created);
}

export function onCopy(e: ClipboardEvent, cut: boolean): void {
  const clip = collect();
  if (!clip || !e.clipboardData) return;
  e.preventDefault();
  e.clipboardData.setData("text/plain", MARKER + JSON.stringify(clip));
  pasteCount = 0;
  if (cut) void remove([...selection.value]);
}

export function onPaste(e: ClipboardEvent): void {
  const text = e.clipboardData?.getData("text/plain") ?? "";
  if (!text.startsWith(MARKER)) return;
  e.preventDefault();
  try {
    const clip = JSON.parse(text.slice(MARKER.length)) as Clip;
    pasteCount += 1;
    void insert(clip, PASTE_OFFSET * pasteCount);
  } catch {
    // Not ours after all; let it go.
  }
}

export function duplicate(): void {
  const clip = collect();
  if (clip) void insert(clip, PASTE_OFFSET);
}
