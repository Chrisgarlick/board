// A schema card (spec 10.3): a view onto some of the board's tables. Each
// table is a grid of columns with a logical type, its extras, keys and a
// foreign key picker across the whole board schema. The ER diagram, drawn by
// the server, sits below.

import { useRef, useState } from "preact/hooks";

import { columnId } from "../ids";
import {
  createTable,
  deleteTable,
  editTable,
  rememberTable,
  schemaVersion,
  setCardTables,
  tableErrors,
  tableList,
  tables,
  type TablePatch,
} from "../store";
import { COLUMN_TYPES, type Column, type ColumnType, type Generated, type Item, type SchemaCardContent, type Table } from "../types";

const GENERATED: Partial<Record<ColumnType, { value: Generated; label: string }[]>> = {
  integer: [{ value: "increment", label: "Auto-increment" }],
  bigint: [{ value: "increment", label: "Auto-increment" }],
  uuid: [{ value: "uuid", label: "Generated" }],
  string: [{ value: "uuid", label: "Generated UUID" }],
  datetime: [
    { value: "now", label: "Set on create" },
    { value: "now_on_update", label: "Set on every change" },
  ],
  date: [{ value: "now", label: "Set on create" }],
};

/** `c` with an optional field set, or removed when `value` is empty. */
function withField<K extends keyof Column>(c: Column, key: K, value: Column[K] | undefined): Column {
  const next = { ...c };
  if (value === undefined) delete next[key];
  else next[key] = value;
  return next;
}

/** Change a column's type, dropping extras that belong to the old one. */
function retype(c: Column, type: ColumnType): Column {
  const { length: _l, precision: _p, scale: _s, values: _v, generated, default: def, ...rest } = c;
  const next: Column = { ...rest, type };
  if (type === "string" && c.length) next.length = c.length;
  if (type === "decimal") {
    next.precision = c.precision ?? 10;
    next.scale = c.scale ?? 2;
  }
  if (type === "enum") next.values = c.values?.length ? c.values : ["draft", "published"];
  if (generated && GENERATED[type]?.some((g) => g.value === generated)) next.generated = generated;
  if (def !== undefined && type === c.type) next.default = def;
  return next;
}

function ERPreview({ item }: { item: Item }) {
  const dark = matchMedia("(prefers-color-scheme: dark)").matches;
  // The server draws the diagram (text as paths: an <img> can't load web
  // fonts). Bust the cache when the schema or the card changes.
  const src = `/api/items/${item.id}/render.svg?title=false&theme=${dark ? "dark" : "light"}&v=${schemaVersion.value}-${item.version}`;
  return (
    <figure class="er-preview">
      <img src={src} alt={`Diagram of the tables on ${item.title ?? "this card"}`} loading="lazy" />
    </figure>
  );
}

export function SchemaBody({ item }: { item: Item }) {
  const content = item.content as SchemaCardContent;
  const all = tables.value;
  const shown = content.table_ids.map((id) => all.get(id)).filter((t): t is Table => !!t);
  const others = tableList.value.filter((t) => !content.table_ids.includes(t.id));
  const [diagram, setDiagram] = useState(true);
  const [focusTable, setFocusTable] = useState<string | null>(null);

  async function addTable() {
    const taken = new Set([...all.values()].map((t) => t.name.toLowerCase()));
    let n = shown.length + 1;
    while (taken.has(`table_${n}`)) n++;
    const id = await createTable(
      `table_${n}`,
      {
        columns: [{ id: columnId(), name: "id", type: "bigint", primary: true, generated: "increment", nullable: false, unique: false }],
        indexes: [],
      },
      item.id,
    );
    if (id) setFocusTable(id);
  }

  return (
    <div class="schema" onPointerDown={(e) => e.stopPropagation()}>
      {shown.length === 0 && (
        <p class="schema-empty">
          No tables on this card yet. Add a new one, or show a table that's already on the board.
        </p>
      )}
      {shown.map((t) => (
        <TableEditor
          key={t.id}
          table={t}
          cardId={item.id}
          cardTables={content.table_ids}
          autofocus={focusTable === t.id}
        />
      ))}
      <footer class="schema-footer">
        <button type="button" class="text-button" onClick={() => void addTable()}>
          Add table
        </button>
        {others.length > 0 && (
          <select
            aria-label="Show a table that's already on the board"
            value=""
            onChange={(e) => {
              const id = e.currentTarget.value;
              if (id) setCardTables(item.id, [...content.table_ids, id]);
            }}
          >
            <option value="">Show existing table…</option>
            {others.map((t) => (
              <option key={t.id} value={t.id}>
                {t.name}
              </option>
            ))}
          </select>
        )}
        {shown.length > 0 && (
          <button
            type="button"
            class={`text-button${diagram ? " is-active" : ""}`}
            aria-pressed={diagram}
            onClick={() => setDiagram(!diagram)}
          >
            Diagram
          </button>
        )}
      </footer>
      {diagram && shown.length > 0 && <ERPreview item={item} />}
    </div>
  );
}

function TableEditor({
  table: t,
  cardId,
  cardTables,
  autofocus,
}: {
  table: Table;
  cardId: string;
  cardTables: string[];
  autofocus: boolean;
}) {
  const error = tableErrors.value.get(t.id);
  const cols = t.definition.columns;
  // The table as it was when the current text edit began, for undo.
  const before = useRef<TablePatch | null>(null);
  const [open, setOpen] = useState<string | null>(null);

  function begin() {
    before.current ??= { name: t.name, definition: t.definition };
  }
  function end() {
    const start = before.current;
    before.current = null;
    const now = tables.value.get(t.id);
    if (start && now) rememberTable(t.id, start, { name: now.name, definition: now.definition });
  }
  /** A typed edit: saved after a pause, one undo step per field edit. */
  function type(patch: TablePatch) {
    begin();
    editTable(t.id, patch);
  }
  /** A one-shot edit (select, checkbox, button): saved now, its own undo step. */
  function commit(patch: TablePatch) {
    const start = { name: t.name, definition: t.definition };
    editTable(t.id, patch, true);
    rememberTable(t.id, start, { name: patch.name ?? t.name, definition: patch.definition ?? t.definition });
  }
  function setColumns(columns: Column[], typed = false) {
    const definition = { ...t.definition, columns };
    if (typed) type({ definition });
    else commit({ definition });
  }
  function setColumn(k: number, c: Column, typed = false) {
    setColumns(
      cols.map((old, j) => (j === k ? c : old)),
      typed,
    );
  }
  function addColumn() {
    const taken = new Set(cols.map((c) => c.name.toLowerCase()));
    let n = cols.length + 1;
    while (taken.has(`column_${n}`)) n++;
    const id = columnId();
    setColumns([...cols, { id, name: `column_${n}`, type: "string", nullable: false, primary: false, unique: false }]);
    requestAnimationFrame(() => document.querySelector<HTMLInputElement>(`[data-col="${id}"]`)?.select());
  }
  function removeColumn(k: number) {
    const gone = cols[k]?.id;
    // Indexes on the removed column go with it (the server would refuse them).
    commit({
      definition: {
        ...t.definition,
        columns: cols.filter((_, j) => j !== k),
        indexes: t.definition.indexes.filter((ix) => !gone || !ix.column_ids.includes(gone)),
      },
    });
  }

  const errorAt = (k: number) => !!error && error.path.startsWith(`definition.columns[${k}]`);

  return (
    <section class="schema-table" data-table={t.id} aria-label={`Table ${t.name}`}>
      <header class="schema-table-header">
        <input
          class="schema-table-name"
          value={t.name}
          aria-label="Table name"
          spellcheck={false}
          onInput={(e) => type({ name: e.currentTarget.value })}
          onBlur={end}
          ref={(input) => {
            if (input && autofocus && document.activeElement !== input && !before.current) {
              input.focus();
              input.select();
            }
          }}
        />
        <button
          type="button"
          class="text-button schema-table-action"
          title="Stop showing this table on this card (it stays in the schema)"
          onClick={() => setCardTables(cardId, cardTables.filter((id) => id !== t.id))}
        >
          Hide
        </button>
        <button
          type="button"
          class="text-button schema-table-action"
          title="Move this table to the trash"
          onClick={() => void deleteTable(t.id)}
        >
          Delete
        </button>
      </header>
      {error && (
        <p class="schema-error" role="alert">
          {error.message}
        </p>
      )}
      <table class="column-grid">
        <thead>
          <tr>
            <th scope="col">Column</th>
            <th scope="col">Type</th>
            <th scope="col" class="col-check" title="Primary key">
              PK
            </th>
            <th scope="col" class="col-check" title="Nullable">
              Null
            </th>
            <th scope="col" class="col-check" title="Unique">
              Uniq
            </th>
            <th scope="col">References</th>
            <th scope="col">
              <span class="visually-hidden">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {cols.map((c, k) => {
            const key = c.id ?? `new-${k}`;
            const expanded = open === key;
            return (
              <ColumnRow
                key={key}
                column={c}
                tableId={t.id}
                invalid={errorAt(k)}
                expanded={expanded}
                onToggle={() => setOpen(expanded ? null : key)}
                onType={(next) => setColumn(k, next, true)}
                onCommit={(next) => setColumn(k, next)}
                onEnd={end}
                onRemove={() => removeColumn(k)}
              />
            );
          })}
        </tbody>
      </table>
      <button type="button" class="text-button schema-add-column" onClick={addColumn}>
        Add column
      </button>
    </section>
  );
}

function ColumnRow({
  column: c,
  tableId,
  invalid,
  expanded,
  onToggle,
  onType,
  onCommit,
  onEnd,
  onRemove,
}: {
  column: Column;
  tableId: string;
  invalid: boolean;
  expanded: boolean;
  onToggle: () => void;
  onType: (c: Column) => void;
  onCommit: (c: Column) => void;
  onEnd: () => void;
  onRemove: () => void;
}) {
  const generated = GENERATED[c.type];
  const refValue = c.references ? `${c.references.table_id}|${c.references.column_id}` : "";
  const num = (v: string) => (v.trim() === "" ? undefined : Math.max(0, Math.round(Number(v))));
  const optional = (v: string) => (v.trim() === "" ? undefined : v);

  return (
    <>
      <tr class={invalid ? "is-invalid" : ""}>
        <td>
          <input
            class="col-name"
            data-col={c.id}
            value={c.name}
            aria-label="Column name"
            spellcheck={false}
            onInput={(e) => onType({ ...c, name: e.currentTarget.value })}
            onBlur={onEnd}
          />
        </td>
        <td class="col-type">
          <select aria-label="Type" value={c.type} onChange={(e) => onCommit(retype(c, e.currentTarget.value as ColumnType))}>
            {COLUMN_TYPES.map((type) => (
              <option key={type} value={type}>
                {type}
              </option>
            ))}
          </select>
          {c.type === "string" && (
            <input
              class="col-extra"
              inputMode="numeric"
              placeholder="len"
              aria-label="Length"
              value={c.length ?? ""}
              onChange={(e) => onCommit(withField(c, "length", num(e.currentTarget.value)))}
            />
          )}
          {c.type === "decimal" && (
            <>
              <input
                class="col-extra"
                inputMode="numeric"
                aria-label="Precision (total digits)"
                title="Precision"
                value={c.precision ?? ""}
                onChange={(e) => onCommit(withField(c, "precision", num(e.currentTarget.value)))}
              />
              <input
                class="col-extra"
                inputMode="numeric"
                aria-label="Scale (digits after the point)"
                title="Scale"
                value={c.scale ?? ""}
                onChange={(e) => onCommit(withField(c, "scale", num(e.currentTarget.value)))}
              />
            </>
          )}
        </td>
        <td class="col-check">
          <input
            type="checkbox"
            aria-label="Primary key"
            checked={c.primary}
            onChange={(e) => {
              const primary = e.currentTarget.checked;
              onCommit({ ...c, primary, ...(primary ? { nullable: false } : {}) });
            }}
          />
        </td>
        <td class="col-check">
          <input
            type="checkbox"
            aria-label="Nullable"
            checked={c.nullable}
            disabled={c.primary}
            onChange={(e) => onCommit({ ...c, nullable: e.currentTarget.checked })}
          />
        </td>
        <td class="col-check">
          <input
            type="checkbox"
            aria-label="Unique"
            checked={c.unique}
            onChange={(e) => onCommit({ ...c, unique: e.currentTarget.checked })}
          />
        </td>
        <td>
          <ReferencePicker
            value={refValue}
            tableId={tableId}
            onPick={(table_id, column_id, type) => {
              if (!table_id || !column_id) {
                const { references: _r, ...rest } = c;
                onCommit(rest);
              } else {
                // Match the referenced column's type, so the key lines up.
                const typed = type && type !== c.type ? retype(c, type) : c;
                onCommit({ ...typed, references: { table_id, column_id } });
              }
            }}
          />
        </td>
        <td class="col-actions">
          <button
            type="button"
            class="icon-button"
            aria-expanded={expanded}
            aria-label={expanded ? "Fewer details" : "More details"}
            title="Default, label, comment"
            onClick={onToggle}
          >
            {expanded ? "▾" : "▸"}
          </button>
          <button type="button" class="icon-button" aria-label={`Remove ${c.name}`} title="Remove column" onClick={onRemove}>
            ×
          </button>
        </td>
      </tr>
      {expanded && (
        <tr class="col-details">
          <td colSpan={7}>
            <div class="col-details-grid">
              {c.type === "enum" && (
                <label>
                  Values
                  <input
                    defaultValue={(c.values ?? []).join(", ")}
                    placeholder="draft, published"
                    onChange={(e) =>
                      onCommit({
                        ...c,
                        values: e.currentTarget.value
                          .split(",")
                          .map((v) => v.trim())
                          .filter(Boolean),
                      })
                    }
                  />
                </label>
              )}
              {generated && (
                <label>
                  Set by the database
                  <select
                    value={c.generated ?? ""}
                    onChange={(e) => {
                      const g = e.currentTarget.value as Generated | "";
                      const { generated: _g, ...rest } = c;
                      onCommit(g ? withField({ ...rest, generated: g }, "default", undefined) : rest);
                    }}
                  >
                    <option value="">No</option>
                    {generated.map((g) => (
                      <option key={g.value} value={g.value}>
                        {g.label}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              {!c.generated && (
                <label>
                  Default
                  <input
                    value={c.default ?? ""}
                    placeholder="none"
                    onInput={(e) => onType(withField(c, "default", optional(e.currentTarget.value)))}
                    onBlur={onEnd}
                  />
                </label>
              )}
              <label>
                Label
                <input
                  value={c.label ?? ""}
                  placeholder="From the name"
                  onInput={(e) => onType(withField(c, "label", optional(e.currentTarget.value)))}
                  onBlur={onEnd}
                />
              </label>
              <label>
                Native type
                <input
                  value={c.native_type ?? ""}
                  placeholder="e.g. varchar(191)"
                  spellcheck={false}
                  onInput={(e) => onType(withField(c, "native_type", optional(e.currentTarget.value)))}
                  onBlur={onEnd}
                />
              </label>
              <label class="col-comment">
                Comment
                <input
                  value={c.comment ?? ""}
                  onInput={(e) => onType(withField(c, "comment", optional(e.currentTarget.value)))}
                  onBlur={onEnd}
                />
              </label>
            </div>
          </td>
        </tr>
      )}
    </>
  );
}

/** Every column on the board, grouped by table, for foreign keys. */
function ReferencePicker({
  value,
  tableId,
  onPick,
}: {
  value: string;
  tableId: string;
  onPick: (tableId: string | null, columnId: string | null, type: ColumnType | null) => void;
}) {
  const list = tableList.value;
  return (
    <select
      class="col-ref"
      aria-label="References"
      value={value}
      onChange={(e) => {
        const [t, c] = e.currentTarget.value.split("|");
        const target = list.find((x) => x.id === t)?.definition.columns.find((x) => x.id === c);
        onPick(t || null, c || null, target?.type ?? null);
      }}
    >
      <option value="">None</option>
      {list.map((t) => (
        <optgroup key={t.id} label={t.id === tableId ? `${t.name} (this table)` : t.name}>
          {t.definition.columns
            .filter((c) => c.id)
            .map((c) => (
              <option key={c.id} value={`${t.id}|${c.id}`}>
                {t.name}.{c.name}
              </option>
            ))}
        </optgroup>
      ))}
    </select>
  );
}
