// A checklist: items in groups (first-seen order, ungrouped first), edited in
// place. Enter adds an item below; Backspace on an empty item removes it.

import { useLayoutEffect, useRef } from "preact/hooks";

import { editContent } from "../store";
import type { ChecklistContent, ChecklistItem } from "../types";

interface Group {
  name: string;
  /** Indexes into content.items. */
  indexes: number[];
}

function groups(items: ChecklistItem[]): Group[] {
  const out: Group[] = [{ name: "", indexes: [] }];
  items.forEach((item, i) => {
    const name = item.group ?? "";
    let g = out.find((x) => x.name === name);
    if (!g) {
      g = { name, indexes: [] };
      out.push(g);
    }
    g.indexes.push(i);
  });
  return out.filter((g) => g.name === "" || g.indexes.length > 0);
}

export function ChecklistBody({ id, content }: { id: string; content: ChecklistContent }) {
  const items = content.items;
  const root = useRef<HTMLDivElement>(null);
  // Which item to focus after the next render. Focusing in a layout effect
  // happens before the browser handles the next keystroke, so typing straight
  // after Enter lands in the new item.
  const focusNext = useRef<number | null>(null);

  useLayoutEffect(() => {
    const index = focusNext.current;
    if (index === null) return;
    focusNext.current = null;
    root.current?.querySelector<HTMLInputElement>(`input[data-index="${index}"]`)?.focus();
  });

  function focusItem(index: number) {
    focusNext.current = index;
  }

  function save(next: ChecklistItem[], immediate = false) {
    editContent(id, { content: { items: next } }, immediate);
  }

  function update(i: number, patch: Partial<ChecklistItem>, immediate = false) {
    save(items.map((item, j) => (j === i ? { ...item, ...patch } : item)), immediate);
  }

  function insertAfter(i: number, group: string | undefined) {
    const next = [...items];
    next.splice(i + 1, 0, group ? { text: "", done: false, group } : { text: "", done: false });
    save(next);
    focusItem(i + 1);
  }

  function remove(i: number) {
    save(items.filter((_, j) => j !== i));
    focusItem(Math.max(0, i - 1));
  }

  function renameGroup(from: string, to: string) {
    const name = to.trim();
    save(
      items.map((item) => {
        if ((item.group ?? "") !== from) return item;
        const { group: _old, ...rest } = item;
        return name ? { ...rest, group: name } : rest;
      }),
    );
  }

  function addGroup() {
    const n = groups(items).length;
    save([...items, { text: "", done: false, group: `Group ${n}` }]);
    focusItem(items.length);
  }

  const done = items.filter((i) => i.done).length;

  return (
    <div class="checklist" ref={root} onPointerDown={(e) => e.stopPropagation()}>
      {groups(items).map((g) => (
        <section class="checklist-group" key={g.name || "_"}>
          {g.name && (
            <input
              class="checklist-group-name"
              value={g.name}
              aria-label="Group name"
              onChange={(e) => renameGroup(g.name, e.currentTarget.value)}
            />
          )}
          <ul>
            {g.indexes.map((i) => {
              const item = items[i];
              if (!item) return null;
              return (
                <li key={i} class={item.done ? "is-done" : ""}>
                  <input
                    type="checkbox"
                    checked={item.done}
                    aria-label={item.done ? "Mark as not done" : "Mark as done"}
                    onChange={(e) => update(i, { done: e.currentTarget.checked }, true)}
                  />
                  <input
                    class="checklist-text"
                    data-index={i}
                    value={item.text}
                    placeholder="Item"
                    aria-label="Item text"
                    onInput={(e) => update(i, { text: e.currentTarget.value })}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        insertAfter(i, item.group);
                      } else if (e.key === "Backspace" && item.text === "" && items.length > 1) {
                        e.preventDefault();
                        remove(i);
                      }
                    }}
                  />
                </li>
              );
            })}
          </ul>
        </section>
      ))}
      <footer class="checklist-footer">
        <button type="button" class="text-button" onClick={() => insertAfter(items.length - 1, undefined)}>
          Add item
        </button>
        <button type="button" class="text-button" onClick={addGroup}>
          Add group
        </button>
        <span class="checklist-count">
          {done} of {items.length} done
        </span>
      </footer>
    </div>
  );
}
