// A note: rendered markdown, double-click (or Enter when selected) to edit.

import { useMemo, useState } from "preact/hooks";

import { renderMarkdown } from "../markdown";
import { editContent } from "../store";
import type { NoteContent } from "../types";

export function NoteBody({ id, content }: { id: string; content: NoteContent }) {
  const [editing, setEditing] = useState(false);
  const html = useMemo(() => renderMarkdown(content.markdown), [content.markdown]);

  if (editing) {
    return (
      <textarea
        class="note-editor"
        value={content.markdown}
        aria-label="Note text (markdown)"
        placeholder="Write in markdown. Esc to finish."
        onPointerDown={(e) => e.stopPropagation()}
        onInput={(e) => editContent(id, { content: { markdown: e.currentTarget.value } })}
        onBlur={() => setEditing(false)}
        onKeyDown={(e) => {
          if (e.key === "Escape") e.currentTarget.blur();
        }}
        ref={(t) => t?.focus()}
      />
    );
  }

  return (
    <div
      class="note-view"
      tabIndex={0}
      onDblClick={() => setEditing(true)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          setEditing(true);
        }
      }}
    >
      {content.markdown.trim() ? (
        <div class="markdown" dangerouslySetInnerHTML={{ __html: html }} />
      ) : (
        <p class="placeholder">Double-click to write.</p>
      )}
    </div>
  );
}
