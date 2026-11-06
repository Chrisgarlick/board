// The trash: items deleted from this board in the last 30 days.

import { useEffect, useState } from "preact/hooks";

import { api } from "../api";
import { report, undelete } from "../store";
import type { Item } from "../types";
import { ago } from "../time";

const WHO: Record<string, string> = { user: "you", claude: "Claude", api: "a script" };

export function TrashPanel({ boardId, onClose }: { boardId: string; onClose: () => void }) {
  const [list, setList] = useState<Item[] | null>(null);

  useEffect(() => {
    api
      .trash(boardId)
      .then((r) => setList(r.items))
      .catch((err) => report(err, "open the trash"));
  }, [boardId]);

  async function restore(id: string) {
    await undelete(id);
    setList((l) => l?.filter((i) => i.id !== id) ?? null);
  }

  return (
    <aside class="panel" aria-label="Trash">
      <header class="panel-header">
        <h2>Trash</h2>
        <button type="button" class="icon-button" aria-label="Close trash" onClick={onClose}>
          ×
        </button>
      </header>
      <p class="panel-note">Deleted cards stay here for 30 days.</p>
      {list === null ? (
        <p class="panel-note">Loading…</p>
      ) : list.length === 0 ? (
        <p class="panel-note">Nothing in the trash.</p>
      ) : (
        <ul class="trash-list">
          {list.map((item) => (
            <li key={item.id}>
              <div>
                <p class="trash-title">{item.title || "Untitled"}</p>
                <p class="trash-meta">
                  Deleted by {WHO[item.deleted_by ?? "user"]} {item.deleted_at ? ago(item.deleted_at) : ""}
                </p>
              </div>
              <button type="button" onClick={() => void restore(item.id)}>
                Restore
              </button>
            </li>
          ))}
        </ul>
      )}
    </aside>
  );
}
