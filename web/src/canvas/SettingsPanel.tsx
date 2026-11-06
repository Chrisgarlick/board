// Board settings (spec 10.3): title, description and the job's stack.

import { useState } from "preact/hooks";

import { api } from "../api";
import { board, report, status } from "../store";
import type { Board } from "../types";

type Field = "title" | "description" | "stack";

export function SettingsPanel({ onClose }: { onClose: () => void }) {
  const b = board.value;
  const [draft, setDraft] = useState<Record<Field, string>>({
    title: b?.title ?? "",
    description: b?.description ?? "",
    stack: b?.stack ?? "",
  });
  if (!b) return null;

  async function save(field: Field) {
    const current = board.value;
    if (!current) return;
    const value = draft[field].trim();
    const was = (current[field] ?? "") as string;
    if (value === was) return;
    if (field === "title" && !value) {
      setDraft({ ...draft, title: current.title });
      return;
    }
    try {
      const patch: Partial<Pick<Board, Field>> =
        field === "title" ? { title: value } : { [field]: value || null };
      board.value = await api.updateBoard(current.id, patch);
      if (field === "title") document.title = `${value} – Board`;
      status.value = { kind: "info", text: "Saved." };
    } catch (err) {
      report(err, "save the board settings");
    }
  }

  const input = (field: Field) => ({
    value: draft[field],
    onInput: (e: Event) => setDraft({ ...draft, [field]: (e.currentTarget as HTMLInputElement).value }),
    onBlur: () => void save(field),
  });

  return (
    <aside class="panel" aria-label="Board settings">
      <header class="panel-header">
        <h2>Board settings</h2>
        <button type="button" class="icon-button" aria-label="Close settings" onClick={onClose}>
          ×
        </button>
      </header>
      <form class="settings" onSubmit={(e) => e.preventDefault()}>
        <label>
          Title
          <input {...input("title")} required />
        </label>
        <label>
          Description
          <textarea rows={4} {...input("description")} placeholder="Who the client is, what the job covers" />
        </label>
        <label>
          Stack
          <input {...input("stack")} placeholder="e.g. Laravel + MySQL" />
          <span class="settings-help">
            What this job is built with. Claude uses it to choose database types; it doesn't change the schema rules.
          </span>
        </label>
      </form>
    </aside>
  );
}
