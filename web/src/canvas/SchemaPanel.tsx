// The schema panel (spec 10.3): every table on the board and which schema
// cards show it, including tables shown on none.

import { columnId } from "../ids";
import {
  board,
  cardsShowing,
  createTable,
  deleteTable,
  schemaVersion,
  select,
  selectedItems,
  setCardTables,
  tableList,
} from "../store";
import type { Item, SchemaCardContent } from "../types";
import { fit } from "./viewport";

/** Select a card and bring it into view. */
export function reveal(card: Item): void {
  select([card.id]);
  const mat = document.querySelector<HTMLElement>(".mat");
  if (mat) fit(card, mat.clientWidth, mat.clientHeight, 96);
}

export function SchemaPanel({ onClose }: { onClose: () => void }) {
  const list = tableList.value;
  const selected = selectedItems.value;
  const card = selected.length === 1 && selected[0]!.card_type === "schema" ? selected[0]! : null;
  const onCard = card ? (card.content as SchemaCardContent).table_ids : [];
  const stack = board.value?.stack;

  async function add() {
    const taken = new Set(list.map((t) => t.name.toLowerCase()));
    let n = list.length + 1;
    while (taken.has(`table_${n}`)) n++;
    await createTable(
      `table_${n}`,
      {
        columns: [{ id: columnId(), name: "id", type: "bigint", primary: true, generated: "increment", nullable: false, unique: false }],
        indexes: [],
      },
      card?.id,
    );
  }

  return (
    <aside class="panel" aria-label="Schema">
      <header class="panel-header">
        <h2>Schema</h2>
        <button type="button" class="icon-button" aria-label="Close schema" onClick={onClose}>
          ×
        </button>
      </header>
      <p class="panel-note">
        {list.length === 0
          ? "No tables yet. Add one here, or on a schema card."
          : `${list.length} ${list.length === 1 ? "table" : "tables"}, version ${schemaVersion.value}.`}
        {stack ? ` Stack: ${stack}.` : ""}
      </p>
      <ul class="schema-list">
        {list.map((t) => {
          const cards = cardsShowing(t.id);
          return (
            <li key={t.id}>
              <div class="schema-list-main">
                <p class="trash-title">{t.name}</p>
                <p class="trash-meta">
                  {t.definition.columns.length} {t.definition.columns.length === 1 ? "column" : "columns"}
                  {cards.length === 0 ? ", not on any card" : ""}
                </p>
                {cards.length > 0 && (
                  <p class="schema-list-cards">
                    {cards.map((c) => (
                      <button key={c.id} type="button" class="chip" title="Show this card" onClick={() => reveal(c)}>
                        {c.title || "Schema"}
                      </button>
                    ))}
                  </p>
                )}
              </div>
              <div class="schema-list-actions">
                {card && !onCard.includes(t.id) && (
                  <button type="button" title={`Show on ${card.title}`} onClick={() => setCardTables(card.id, [...onCard, t.id])}>
                    Show
                  </button>
                )}
                <button type="button" class="text-button" aria-label={`Delete ${t.name}`} onClick={() => void deleteTable(t.id)}>
                  Delete
                </button>
              </div>
            </li>
          );
        })}
      </ul>
      <button type="button" onClick={() => void add()}>
        {card ? `New table on ${card.title}` : "New table"}
      </button>
      {!card && list.length > 0 && <p class="panel-note">Select a schema card to show tables on it.</p>}
    </aside>
  );
}
