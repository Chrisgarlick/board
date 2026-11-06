// An item's history: its saved versions, who made each, and a way back.
// Edits by the same person within five minutes are one version (spec 4.3),
// so "undo what Claude did" is one restore.

import { useEffect, useState } from "preact/hooks";

import { api } from "../api";
import { historyFor, item, report, restoreRevision } from "../store";
import type { Revision } from "../types";
import { ago } from "../time";

const WHO: Record<string, string> = { user: "You", claude: "Claude", api: "A script" };

function preview(r: Revision): string {
  const c = r.content as Record<string, unknown>;
  if (typeof c.markdown === "string") return c.markdown;
  if (typeof c.text === "string") return c.text;
  if (Array.isArray(c.items)) return (c.items as { text: string }[]).map((i) => i.text).join(", ");
  return r.title ?? "";
}

export function HistoryPanel({ id }: { id: string }) {
  const [list, setList] = useState<Revision[] | null>(null);
  const current = item(id);

  useEffect(() => {
    setList(null);
    api
      .revisions(id)
      .then((r) => setList(r.revisions))
      .catch((err) => report(err, "load the history"));
  }, [id, current?.version]);

  return (
    <aside class="panel" aria-label="History">
      <header class="panel-header">
        <h2>History</h2>
        <button type="button" class="icon-button" aria-label="Close history" onClick={() => (historyFor.value = null)}>
          ×
        </button>
      </header>
      <p class="panel-note">
        {current?.title ?? "This item"}: saved versions, newest first. Edits by the same person within five minutes count as
        one version.
      </p>
      {list === null ? (
        <p class="panel-note">Loading…</p>
      ) : (
        <ol class="history-list">
          {list.map((r, n) => (
            <li key={r.id}>
              <div>
                <p class="trash-title">
                  {WHO[r.changed_by] ?? r.changed_by} {ago(r.updated_at)}
                  {n === 0 && <span class="history-current"> (current)</span>}
                </p>
                <p class="history-preview">{preview(r) || <span class="placeholder">Empty</span>}</p>
              </div>
              {n > 0 && (
                <button type="button" onClick={() => void restoreRevision(id, r.id)}>
                  Restore
                </button>
              )}
            </li>
          ))}
        </ol>
      )}
    </aside>
  );
}
