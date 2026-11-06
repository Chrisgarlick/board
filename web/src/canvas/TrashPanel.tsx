// The trash: items and connectors deleted from this board in the last 30 days.

import { useEffect, useState } from "preact/hooks";

import { api } from "../api";
import { report, restoreConnector, restoreFromTrash } from "../store";
import type { Connector, Item } from "../types";
import { ago } from "../time";

const WHO: Record<string, string> = { user: "you", claude: "Claude", api: "a script" };

function describe(i: Item): string {
  if (i.title) return i.title;
  const text = (i.content as { text?: string }).text;
  const kind = { shape: "Shape", sticky: "Sticky note", text: "Text", frame: "Frame", card: "Card" }[i.kind];
  return text ? `${kind}: ${text}` : kind;
}

export function TrashPanel({ boardId, onClose }: { boardId: string; onClose: () => void }) {
  const [list, setList] = useState<{ items: Item[]; connectors: Connector[] } | null>(null);

  useEffect(() => {
    api
      .trash(boardId)
      .then(setList)
      .catch((err) => report(err, "open the trash"));
  }, [boardId]);

  async function restore(id: string) {
    await restoreFromTrash(id);
    setList((l) => (l ? { ...l, items: l.items.filter((i) => i.id !== id) } : l));
  }

  async function restoreLink(id: string) {
    await restoreConnector(id);
    setList((l) => (l ? { ...l, connectors: l.connectors.filter((c) => c.id !== id) } : l));
  }

  const empty = list && list.items.length === 0 && list.connectors.length === 0;

  return (
    <aside class="panel" aria-label="Trash">
      <header class="panel-header">
        <h2>Trash</h2>
        <button type="button" class="icon-button" aria-label="Close trash" onClick={onClose}>
          ×
        </button>
      </header>
      <p class="panel-note">Deleted things stay here for 30 days.</p>
      {list === null ? (
        <p class="panel-note">Loading…</p>
      ) : empty ? (
        <p class="panel-note">Nothing in the trash.</p>
      ) : (
        <ul class="trash-list">
          {list.items.map((i) => (
            <li key={i.id}>
              <div>
                <p class="trash-title">{describe(i)}</p>
                <p class="trash-meta">
                  Deleted by {WHO[i.deleted_by ?? "user"]} {i.deleted_at ? ago(i.deleted_at) : ""}
                </p>
              </div>
              <button type="button" onClick={() => void restore(i.id)}>
                Restore
              </button>
            </li>
          ))}
          {list.connectors.map((c) => (
            <li key={c.id}>
              <div>
                <p class="trash-title">{c.label ? `Connector: ${c.label}` : "Connector"}</p>
                <p class="trash-meta">Deleted {c.deleted_at ? ago(c.deleted_at) : ""}</p>
              </div>
              <button type="button" onClick={() => void restoreLink(c.id)}>
                Restore
              </button>
            </li>
          ))}
        </ul>
      )}
    </aside>
  );
}
