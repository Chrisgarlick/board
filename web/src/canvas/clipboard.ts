// Copy, cut, paste and duplicate (spec 10.2). Uses the browser's copy and
// paste events (no clipboard permission prompt), so it works across tabs and
// boards. The clipboard holds plain JSON with a marker prefix.

import type { Item } from "../types";
import { connectors, create, item, membersOf, remove, select, selection, type NewConnector, type NewItem } from "../store";

const MARKER = "board-clipboard/1:";
const PASTE_OFFSET = 20;

interface Clip {
  items: (NewItem & { ref: string })[];
  connectors: NewConnector[];
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
  return { items, connectors: conns };
}

let pasteCount = 0;

/** Create a copy of a clip, shifted by `offset`, and select it. */
async function insert(clip: Clip, offset: number): Promise<void> {
  // Refs are the original ids; the batch maps them to the new ones.
  const created = await create(
    clip.items.map((n) => ({ ...n, x: n.x + offset, y: n.y + offset })),
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
