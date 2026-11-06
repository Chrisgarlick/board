// The board list: a flat index of boards with search (spec section 15, Q3).

import { useEffect, useRef, useState } from "preact/hooks";

import { api } from "../api";
import { navigate } from "../router";
import { report } from "../store";
import { ago } from "../time";
import type { BoardSummary } from "../types";

export function BoardList() {
  const [q, setQ] = useState("");
  const [archived, setArchived] = useState(false);
  const [boards, setBoards] = useState<BoardSummary[] | null>(null);
  const [newTitle, setNewTitle] = useState("");
  const [renaming, setRenaming] = useState<string | null>(null);
  const searchTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  function refresh(query = q, showArchived = archived) {
    api
      .listBoards(query, showArchived)
      .then((r) => setBoards(r.boards))
      .catch((err) => report(err, "load your boards"));
  }

  useEffect(() => {
    document.title = "Boards";
    refresh();
  }, []);

  async function create(e: Event) {
    e.preventDefault();
    const title = newTitle.trim();
    if (!title) return;
    try {
      const b = await api.createBoard(title);
      navigate(`/boards/${b.id}`);
    } catch (err) {
      report(err, "create the board");
    }
  }

  async function update(id: string, patch: { title?: string; archived?: boolean }) {
    try {
      await api.updateBoard(id, patch);
      refresh();
    } catch (err) {
      report(err, "update the board");
    }
  }

  return (
    <main class="boards">
      <header class="boards-header">
        <h1>Boards</h1>
        <form class="new-board" onSubmit={create}>
          <input
            value={newTitle}
            placeholder="New board, e.g. Acme rebuild"
            aria-label="New board title"
            onInput={(e) => setNewTitle(e.currentTarget.value)}
          />
          <button type="submit" disabled={!newTitle.trim()}>
            Create board
          </button>
        </form>
      </header>

      <div class="boards-tools">
        <input
          type="search"
          value={q}
          placeholder="Search boards"
          aria-label="Search boards"
          onInput={(e) => {
            const value = e.currentTarget.value;
            setQ(value);
            clearTimeout(searchTimer.current);
            searchTimer.current = setTimeout(() => refresh(value), 200);
          }}
        />
        <label class="toggle">
          <input
            type="checkbox"
            checked={archived}
            onChange={(e) => {
              const value = e.currentTarget.checked;
              setArchived(value);
              refresh(q, value);
            }}
          />
          Show archived
        </label>
      </div>

      {boards === null ? (
        <p class="boards-empty">Loading…</p>
      ) : boards.length === 0 ? (
        <p class="boards-empty">
          {q
            ? `No boards match “${q}”.`
            : archived
              ? "No archived boards."
              : "No boards yet. Name your first one above."}
        </p>
      ) : (
        <ul class="board-rows">
          {boards.map((b) => (
            <li key={b.id} class="board-row">
              {renaming === b.id ? (
                <input
                  class="board-row-rename"
                  defaultValue={b.title}
                  aria-label="Board title"
                  ref={(input) => input?.focus()}
                  onBlur={(e) => {
                    setRenaming(null);
                    const t = e.currentTarget.value.trim();
                    if (t && t !== b.title) void update(b.id, { title: t });
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") e.currentTarget.blur();
                    if (e.key === "Escape") setRenaming(null);
                  }}
                />
              ) : (
                <a
                  class="board-row-title"
                  href={`/boards/${b.id}`}
                  onClick={(e) => {
                    e.preventDefault();
                    navigate(`/boards/${b.id}`);
                  }}
                >
                  {b.title}
                </a>
              )}
              <span class="board-row-meta">
                {b.item_count === 1 ? "1 card" : `${b.item_count} cards`}
              </span>
              <span class="board-row-meta">Edited {ago(b.updated_at)}</span>
              <span class="board-row-actions">
                <button type="button" class="text-button" onClick={() => setRenaming(b.id)}>
                  Rename
                </button>
                <button type="button" class="text-button" onClick={() => void update(b.id, { archived: !b.archived })}>
                  {b.archived ? "Unarchive" : "Archive"}
                </button>
              </span>
            </li>
          ))}
        </ul>
      )}
    </main>
  );
}
