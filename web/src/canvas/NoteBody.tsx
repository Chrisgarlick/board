// A note: rendered markdown, double-click (or Enter when selected) to edit.

import { useMemo, useState } from "preact/hooks";

import { renderMarkdown } from "../markdown";
import { editContent, readOnly } from "../store";
import type { NoteContent } from "../types";

export function NoteBody({ id, content }: { id: string; content: NoteContent }) {
  const [editing, setEditing] = useState(false);
  const html = useMemo(() => renderMarkdown(content.markdown), [content.markdown]);

  const ro = readOnly.value;
  if (editing && !ro) {
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
      onDblClick={() => !ro && setEditing(true)}
      onKeyDown={(e) => {
        if (e.key === "Enter" && !ro) {
          e.preventDefault();
          setEditing(true);
        }
      }}
    >
      {content.markdown.trim() ? (
        <div class="markdown" dangerouslySetInnerHTML={{ __html: html }} />
      ) : (
        <p class="placeholder">{ro ? "Empty." : "Double-click to write."}</p>
      )}
    </div>
  );
}
