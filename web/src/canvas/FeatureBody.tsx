// A feature card (spec 10.3): what the feature is, who it's for, how you'll
// know it's done, and which tables and pages it touches. Table and page chips
// pan to the schema or sitemap card that shows them.

import { useLayoutEffect, useMemo, useRef, useState } from "preact/hooks";

import { renderMarkdown } from "../markdown";
import { cardsShowing, editContent, item, pages, rememberEdit, tableList, tables, type ContentPatch } from "../store";
import type { FeatureContent, FeatureStatus } from "../types";
import { reveal } from "./SchemaPanel";

const STATUSES: { value: FeatureStatus; label: string }[] = [
  { value: "idea", label: "Idea" },
  { value: "planned", label: "Planned" },
  { value: "in_progress", label: "In progress" },
  { value: "done", label: "Done" },
];

export function FeatureBody({ id, content }: { id: string; content: FeatureContent }) {
  const [editingSummary, setEditingSummary] = useState(false);
  const html = useMemo(() => renderMarkdown(content.summary), [content.summary]);
  const box = useRef<HTMLDivElement>(null);
  const focusNext = useRef<string | null>(null);
  // The content when the current text edit began, for undo.
  const before = useRef<ContentPatch | null>(null);

  useLayoutEffect(() => {
    const sel = focusNext.current;
    if (!sel) return;
    focusNext.current = null;
    box.current?.querySelector<HTMLInputElement>(sel)?.focus();
  });

  /** A typed edit: saved after a pause, one undo step when the field loses focus. */
  function type(patch: Partial<FeatureContent>) {
    before.current ??= { content: item(id)?.content };
    editContent(id, { content: { ...content, ...patch } });
  }
  function end() {
    const start = before.current;
    before.current = null;
    if (start) rememberEdit(id, start, { content: item(id)?.content });
  }
  /** A one-shot edit: saved now, its own undo step. */
  function commit(patch: Partial<FeatureContent>) {
    const start = { content: item(id)?.content };
    const after = { content: { ...content, ...patch } };
    editContent(id, after, true);
    rememberEdit(id, start, after);
  }

  const allTables = tables.value;
  const shownTables = content.table_ids.map((t) => allTables.get(t)).filter((t) => !!t);
  const allPages = pages.value;
  const shownPages = content.page_ids.map((p) => allPages.find((x) => x.node.id === p)).filter((p) => !!p);

  return (
    <div class="feature" ref={box} onPointerDown={(e) => e.stopPropagation()}>
      <div class="feature-status segmented" role="radiogroup" aria-label="Status">
        {STATUSES.map((s) => (
          <button
            key={s.value}
            type="button"
            role="radio"
            aria-checked={content.status === s.value}
            class={`status-${s.value}${content.status === s.value ? " is-active" : ""}`}
            onClick={() => commit({ status: s.value })}
          >
            {s.label}
          </button>
        ))}
      </div>

      <section class="feature-section">
        {editingSummary ? (
          <textarea
            class="feature-summary-editor"
            value={content.summary}
            aria-label="Summary (markdown)"
            placeholder="What it is and who it's for. Esc to finish."
            onInput={(e) => type({ summary: e.currentTarget.value })}
            onBlur={() => {
              end();
              setEditingSummary(false);
            }}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Escape") e.currentTarget.blur();
            }}
            ref={(t) => {
              if (t && document.activeElement !== t) t.focus();
            }}
          />
        ) : (
          <div
            class="feature-summary"
            tabIndex={0}
            onDblClick={() => setEditingSummary(true)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                setEditingSummary(true);
              }
            }}
          >
            {content.summary.trim() ? (
              <div class="markdown" dangerouslySetInnerHTML={{ __html: html }} />
            ) : (
              <p class="placeholder">Double-click to describe the feature.</p>
            )}
          </div>
        )}
      </section>

      <section class="feature-section">
        <h3>Stories</h3>
        <ul class="feature-list">
          {content.stories.map((story, k) => (
            <li key={k}>
              <input
                class="feature-story"
                data-story={k}
                value={story}
                placeholder="As a … I want … so that …"
                aria-label={`Story ${k + 1}`}
                onInput={(e) => type({ stories: content.stories.map((s, j) => (j === k ? e.currentTarget.value : s)) })}
                onBlur={end}
                onKeyDown={(e) => {
                  e.stopPropagation();
                  if (e.key === "Enter") {
                    e.preventDefault();
                    const stories = [...content.stories];
                    stories.splice(k + 1, 0, "");
                    commit({ stories });
                    focusNext.current = `[data-story="${k + 1}"]`;
                  } else if (e.key === "Backspace" && story === "") {
                    e.preventDefault();
                    commit({ stories: content.stories.filter((_, j) => j !== k) });
                    if (k > 0) focusNext.current = `[data-story="${k - 1}"]`;
                  }
                }}
              />
            </li>
          ))}
        </ul>
        <button
          type="button"
          class="text-button feature-add"
          onClick={() => {
            commit({ stories: [...content.stories, ""] });
            focusNext.current = `[data-story="${content.stories.length}"]`;
          }}
        >
          Add story
        </button>
      </section>

      <section class="feature-section">
        <h3>
          Done when{" "}
          {content.acceptance.length > 0 && (
            <span class="feature-count">
              {content.acceptance.filter((a) => a.done).length} of {content.acceptance.length}
            </span>
          )}
        </h3>
        <ul class="feature-list checklist">
          {content.acceptance.map((a, k) => (
            <li key={k} class={a.done ? "is-done" : ""}>
              <input
                type="checkbox"
                checked={a.done}
                aria-label={a.done ? "Mark as not met" : "Mark as met"}
                onChange={(e) =>
                  commit({
                    acceptance: content.acceptance.map((x, j) => (j === k ? { ...x, done: e.currentTarget.checked } : x)),
                  })
                }
              />
              <input
                class="checklist-text"
                data-acceptance={k}
                value={a.text}
                placeholder="A check that proves it works"
                aria-label={`Acceptance check ${k + 1}`}
                onInput={(e) =>
                  type({ acceptance: content.acceptance.map((x, j) => (j === k ? { ...x, text: e.currentTarget.value } : x)) })
                }
                onBlur={end}
                onKeyDown={(e) => {
                  e.stopPropagation();
                  if (e.key === "Enter") {
                    e.preventDefault();
                    const acceptance = [...content.acceptance];
                    acceptance.splice(k + 1, 0, { text: "", done: false });
                    commit({ acceptance });
                    focusNext.current = `[data-acceptance="${k + 1}"]`;
                  } else if (e.key === "Backspace" && a.text === "") {
                    e.preventDefault();
                    commit({ acceptance: content.acceptance.filter((_, j) => j !== k) });
                    if (k > 0) focusNext.current = `[data-acceptance="${k - 1}"]`;
                  }
                }}
              />
            </li>
          ))}
        </ul>
        <button
          type="button"
          class="text-button feature-add"
          onClick={() => {
            commit({ acceptance: [...content.acceptance, { text: "", done: false }] });
            focusNext.current = `[data-acceptance="${content.acceptance.length}"]`;
          }}
        >
          Add check
        </button>
      </section>

      <section class="feature-section feature-links">
        <h3>Tables</h3>
        <div class="chips">
          {shownTables.map((t) => {
            const card = cardsShowing(t.id)[0];
            return (
              <span key={t.id} class="chip-pair">
                <button
                  type="button"
                  class="chip"
                  title={card ? `Show ${t.name} on ${card.title}` : `${t.name} isn't on a schema card`}
                  disabled={!card}
                  onClick={() => card && reveal(card)}
                >
                  {t.name}
                </button>
                <button
                  type="button"
                  class="chip-remove"
                  aria-label={`Remove ${t.name}`}
                  onClick={() => commit({ table_ids: content.table_ids.filter((x) => x !== t.id) })}
                >
                  ×
                </button>
              </span>
            );
          })}
          <Picker
            label="Add table"
            options={tableList.value
              .filter((t) => !content.table_ids.includes(t.id))
              .map((t) => ({ value: t.id, label: t.name }))}
            onPick={(v) => commit({ table_ids: [...content.table_ids, v] })}
          />
        </div>
        <h3>Pages</h3>
        <div class="chips">
          {shownPages.map((p) => {
            const card = item(p.cardId);
            return (
              <span key={p.node.id} class="chip-pair">
                <button
                  type="button"
                  class="chip"
                  title={`Show the sitemap`}
                  onClick={() => card && reveal(card)}
                >
                  {p.node.slug ?? p.node.title}
                </button>
                <button
                  type="button"
                  class="chip-remove"
                  aria-label={`Remove ${p.node.title}`}
                  onClick={() => commit({ page_ids: content.page_ids.filter((x) => x !== p.node.id) })}
                >
                  ×
                </button>
              </span>
            );
          })}
          <Picker
            label="Add page"
            options={allPages
              .filter((p) => !content.page_ids.includes(p.node.id))
              .map((p) => ({ value: p.node.id, label: `${" ".repeat(p.depth)}${p.node.title || "Untitled"}${p.node.slug ? `  ${p.node.slug}` : ""}` }))}
            onPick={(v) => commit({ page_ids: [...content.page_ids, v] })}
          />
        </div>
      </section>
    </div>
  );
}

function Picker({
  label,
  options,
  onPick,
}: {
  label: string;
  options: { value: string; label: string }[];
  onPick: (value: string) => void;
}) {
  if (options.length === 0) return null;
  return (
    <select
      class="chip-picker"
      aria-label={label}
      value=""
      onChange={(e) => {
        if (e.currentTarget.value) onPick(e.currentTarget.value);
        e.currentTarget.value = "";
      }}
    >
      <option value="">{label}…</option>
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}
