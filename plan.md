# Board: Build Plan

How to get Board built, in order. The spec is `planning-board-tech-spec-rust.md`; section numbers below (§) refer to it. Each phase ends in something you can use, and lists what has to be decided before it starts.

Phases 0–8 are this repo. Two tracks run outside it: **Typeset** changes (needed for phase 7) and **Design** (which starts reading Board after phase 6).

---

## Decisions to make, and when

| Decide | Before | Spec | Notes |
|---|---|---|---|
| ~~Frontend framework~~ | Phase 1 | §15 Q4 | **Decided:** Preact with signals |
| ~~Folders or a flat list~~ | Phase 1 | §15 Q3 | **Decided:** flat list with search; attaching boards to clients comes later |
| ~~Shape set, nested frames~~ | Phase 2 | §15 Q9 | **Decided:** shape set as specced; no nesting in v1, can be added later |
| ~~Which MCP clients~~ | Phase 3 | §15 Q5 | **Decided:** Claude Code only for now, with a static bearer token; OAuth for claude.ai / Desktop can be added later |
| ~~Export font~~ | Phase 4 | §15 Q6 | **Decided:** Inter for every export in v1; brand fonts per board can come later |
| ~~One schema per board, or several~~ | Phase 5 | §15 Q7 | **Decided:** one per board, as specced; build on it after real use |
| ~~What Design is written in~~ | Phase 6 | §15 Q8 | **Decided:** Rust, so Design depends on the `board-model` crate (git tag) |
| Typeset's actual input format | Phase 7 | §15 Q1 | **Blocks phase 7.** Send a real payload |
| How Typeset reaches Board (same droplet, network, auth) | Phase 7 | §15 Q2 | |

---

## Phase 0: Project setup

**Goal:** an empty app that builds, tests and runs, with the delivered code in place.

- [x] `git init`; add `.gitignore` (`target/`, `web/node_modules/`, `web/dist/`, `*.db*`, `board.toml`).
- [x] Cargo workspace (§2 layout): `crates/board`, `crates/board-model` (empty for now), and a temporary `crates/typeset-doc` until it moves to the Typeset repo.
- [x] Move `typeset_doc.rs` → `crates/typeset-doc/src/lib.rs` and `export.rs` → `crates/board/src/export/mod.rs`. Fix the `use` path. Run the 8 existing tests and confirm they still pass.
- [x] Dependencies: `axum` 0.8, `tokio`, `sqlx` (sqlite, runtime-checked), `serde`, `serde_json`, `serde_path_to_error`, `ulid`, `thiserror`, `tracing`, `clap` (CLI), `toml` (config), `sha2`, `hmac`, `base64`, `rust-embed`.
- [x] Config file `board.toml`: bind address (default `127.0.0.1:7070`), database path, public origin, `server_secret` (generated on first run if missing), Typeset URL. Example committed as `board.example.toml`. (Allow-listed hosts are Typeset's setting, not Board's: Board never fetches from other hosts.)
- [x] CLI skeleton with `clap`: `board serve`, `board login`, `board token create|list|revoke`, `board pull` (stub).
- [x] Frontend toolchain in `web/`: TypeScript and esbuild with a watch script. Preact with `@preact/signals`, JSX compiled by esbuild. In debug builds, serve `web/dist` from disk so you can work on it without recompiling; in release, embed with `rust-embed`. A `build.rs` recompiles the app when `web/dist` changes, so a binary built before the frontend never serves missing or stale files.
- [x] `/healthz` route.
- [x] CI, run locally: `scripts/ci.sh` (frontend type-check and build, `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`), a pre-push hook in `.githooks/`, and a manual-only GitHub workflow that runs the same script.

**Status:** done (2026-11-06). Also added: `README.md`, `anyhow`/`rand`/`tracing-subscriber` dependencies, and route tests.

**Done when:** `./scripts/ci.sh` passes, `board serve` serves a blank page and `/healthz`, and the frontend rebuilds on save.

---

## Phase 1: Core (M1)

**Goal:** a deployed board where you can make boards and drag note and checklist cards around, safely.

### Backend

- [x] `migrations/0001_init.sql`: everything in §4.1 (boards, items, connectors, schema_tables, revisions, FTS, sessions, api_tokens). Create the whole schema now, even the parts used later; one initial migration is simpler than five.
- [x] `db.rs`: pool with `journal_mode=WAL`, `foreign_keys=ON`, `busy_timeout=5000`, size about 5; a `write_tx()` helper that opens `BEGIN IMMEDIATE` (§4.2). Test two concurrent writers to prove no `SQLITE_BUSY`.
- [x] Error type mapping to HTTP: 404, 409 (with current record), 422 (`{ error, path, message }`, §5.5), 413.
- [x] `auth.rs`:
  - [x] Tokens: create (32 random bytes, base64url, shown once), store sha256, constant-time compare, scope and actor (§3.2).
  - [x] Sessions: `board login` creates a one-time code (10 minutes); `/login/{code}` sets the cookie (`HttpOnly`, `Secure`, `SameSite=Strict`, 30 days) and redirects (§3.1).
  - [x] Middleware: cookie or bearer → `Caller { scope, actor }`; `Origin` check on writes from cookie sessions; route exemptions (§3.4).
- [x] Content types: `ItemContent` for `note` and `checklist` cards only for now, with `deny_unknown_fields` and `serde_path_to_error` (§5.1, §5.5).
- [x] Service layer: boards CRUD; items create/get/update (versioned)/geometry/delete/undelete; revisions with 5-minute per-actor coalescing and prune to 20 (§4.3); restore.
- [x] Batch endpoint with `ref`s, one transaction (§6 Batch). Build it now: the UI's multi-select and undo depend on it.
- [x] REST routes for boards and items (§6), per-route body limits.
- [x] Auto-placement for items created without a position (§7).
- [x] FTS sync on item writes; `/api/search`.
- [x] Trash: soft delete everywhere, `/api/boards/{id}/trash`, daily purge task (items deleted more than 30 days ago) and expired-session cleanup.
- [x] Tests: version conflict returns 409 with the current item; revision coalescing (same actor within 5 minutes merges, a different actor starts a new one); batch rollback on failure; soft delete hides items and undelete restores them.

### Frontend

- [x] Board list page: create, rename, archive, search by title (flat list; clients come later).
- [x] Canvas: CSS-transform pan and zoom (scroll, pinch, `Space`-drag), §10.1.
- [x] Cards as absolutely positioned elements; drag by header, resize from corner; geometry saved on drag end.
- [x] Note editor (textarea plus markdown preview, no raw HTML) and checklist editor.
- [x] Content saving: debounced, **one save in flight per item**, next save uses the returned `version`; "changed elsewhere" banner on a real 409 (§10.4).
- [x] `X-Client-Id` header on every request (random per tab).
- [x] Trash panel with Undelete.

### Deploy

Board runs on the Kritano droplet next to Motion (`deploy/README.md`); `deploy/deploy.sh` builds and installs it.

- [x] Release build (GitHub Actions, static musl binary); systemd unit; nginx with certbot TLS at `https://board.kritano.com`; bound to localhost only. Deployed 2026-11-06.
- [x] Sign in: passkeys (added 2026-11-06), with `board login` as the recovery path. A `laptop` admin token for scripts is optional.
- [x] Nightly `scripts/backup.sh` via a systemd timer, 14 days kept on the droplet. Still yours: an off-droplet `BACKUP_REMOTE`, and one test restore.

**Status:** code done (2026-11-06); deploy is yours to run. Also added beyond the list:

- `login_codes` table for one-time links; `GET /api/me` and `POST /api/logout`.
- Browser tests (`e2e/`, run with `./scripts/e2e.sh`): 19 checks covering login, notes, checklists, drag, resize, reload, two-tab conflicts, trash and zoom.
- Logs go to stderr, so `board token create` and `board login` output can be captured safely.

**Done when:** you can log in at your Board domain, make a board, add and drag notes and checklists, edit them from two tabs and see a conflict banner, delete and undelete a card, and last night's backup restores.

---

## Phase 2: Drawing (M2)

**Goal:** Board works as a planning canvas: you can draw a flowchart by hand quickly.

### Backend

- [x] `ItemContent` for `shape`, `sticky`, `text`, `frame`; `style` validation per kind (§5.1, §5.2).
- [x] Connectors: create/update/delete (soft), validation (same board, live ends, not self), hidden when an end is deleted (§4.1).
- [x] Frame membership computed by the server on every create/move/resize; moving a frame moves its members in the same transaction; delete with or without contents (§4.5).
- [x] Cross-board move rules, keeping connectors between moved items (§6).
- [x] Item cap per board (5,000).
- [x] Tests: membership after moves and resizes; frame move moves members; connector visibility follows soft delete; cross-board move keeps internal connectors and drops the rest.

### Shared routing fixtures

- [x] Write `fixtures/routing.json` now: connector inputs (item rectangles, anchors, routing style) and the expected path points, covering `auto` anchor choice, elbows and curves (§8.1). The UI implements against it in this phase; the Rust renderer uses the same file in phase 4.

### Frontend

- [x] Shapes as inline SVG with an HTML text layer; stickies; text; frames drawn beneath members (§10.1).
- [x] Connector layer: anchors on hover, pinned vs `auto` ends, straight/elbow/curved routing, labels, arrowheads, dashed/dotted lines. Re-route live while dragging. Passes `fixtures/routing.json`.
- [x] Toolbar and shortcuts (§10.2): `V`, `H`, `R`, `D`, `O`, `S`, `T`, `F`, `C`, `K`.
- [x] Quick flow: `+` handles, drag-to-create-connected-shape, `Tab` for the next step.
- [x] Text editing in shapes, stickies, text and connector labels (`Enter`/double-click, `Esc`).
- [x] Selection: click, shift-click, marquee; multi-drag as one batch.
- [x] Arrange: snap to grid, alignment guides, align/distribute, z-order, lock.
- [x] Clipboard: copy, cut, paste, duplicate, including connectors between selected items.
- [x] Style bar: colour, change shape type, connector style.
- [x] Local undo/redo stack (100 operations, inverses sent as batches), §10.4.
- [x] Zoom to fit / selection; card bodies collapse below 40% zoom; viewport culling past about 500 items.

**Status:** done (2026-11-06). Notes:

- Batches return `affected` items (side effects such as frame members moving) so the UI applies the server's answer instead of repeating its rules.
- Routing rules and 9 shared fixture cases are in `web/src/canvas/routing.ts` and `fixtures/routing.json`; the UI's test runs them (`npm test` in `web/`, part of `scripts/ci.sh`).
- Browser tests: `e2e/drawing.mjs` (27 checks) draws a flowchart with the shape tool, quick-flow handles, Tab, the connector tool, marquee, colour, framing, frame drag, undo/redo, delete/undo, duplicate, copy/paste, align, lock, stickies and text.
- Not built: dragging a connector's ends to reconnect them (the API supports it; the UI can add it later).

**Done when:** you can draw a 15-step flowchart with decisions, labels and a frame around it using mostly the keyboard, move the frame and have everything follow, and undo your way back to empty.

---

## Phase 3: Claude (M3)

**Goal:** Claude can read and draw on boards, and you see its changes live.

### Backend

- [x] SSE (`events.rs`): `tokio::sync::broadcast` per board; every event type from §6 Live updates; `batch` events for batches, frame moves and flows; `origin` on every event; `resync` when a subscriber lags.
- [x] `flow/mermaid.rs`: the Mermaid flowchart subset parser (§5.3) with line-numbered errors, and the reverse (frame → Mermaid). Input caps (64 KB, 500 nodes).
- [x] `flow/layout.rs`: layered layout (§5.3): cycle breaking, longest-path layers, barycentre ordering, fixed spacing, `TB`/`LR`, frame wrapping, placement in free space or `near` an item.
- [x] Endpoints: `POST /api/boards/{id}/flows`, `GET …/flows/{frame_id}/mermaid`. (`POST /api/items/{id}/convert-to-shapes` moves to phase 8, with diagram cards.)
- [x] `mcp.rs` with `rmcp` on `/mcp` (check the crate's current docs first): every tool in §7 that exists so far: boards, items, `add_items`, `draw_flow`, `get_flow`, `update_item`, `set_geometry`, `move_items_to_board`, `delete_items` (5-minute user-edit guard), connectors, revisions, search. Tool descriptions include the exact content shapes, style fields and Mermaid subset.
- [ ] `board://{id}` resource. Deferred: `get_board` covers it, and the resource API in rmcp 3.9 is awkward. Revisit if Claude Code makes good use of resources.
- [ ] OpenAPI via `utoipa`, served at `/api/openapi.json`. Deferred to phase 8: MCP covers Claude, and the REST shapes are still growing (phases 5 and 6).
- [x] Tests: parser (every node shape, edge style, labels, chains, subgraphs, each unsupported construct), Mermaid round trip, layout determinism and no overlaps, MCP tools against a test database.

### Frontend

- [x] SSE client: merge events, skip own `origin`, apply `batch` events in one frame, reload on `resync` or reconnect, flag (never overwrite) items being edited locally.
- [x] Brief highlight on items Claude created or changed.
- [x] Revisions panel per item: list, preview, restore. Makes "undo what Claude did" one click.

### Connect Claude Code

- [x] `board token create --name claude-mcp --scope admin --actor claude`.
- [x] `claude mcp add --transport http board https://<board-domain>/mcp --header "Authorization: Bearer …"`.
- [x] Try: "draw the checkout flow for a booking site on board X", then tidy it by hand; "read the flow I drew and suggest missing steps".

**Status:** done (2026-11-06), except the two deferred items above. Notes:

- Every REST item and connector write now runs through the batch machinery, so REST, the UI and MCP publish identical live events.
- MCP tools: `list_boards`, `create_board`, `update_board`, `get_board`, `get_item`, `search`, `add_items`, `draw_flow`, `get_flow`, `update_item`, `set_geometry`, `move_items_to_board`, `delete_items`, `connect`, `update_connector`, `delete_connector`, `list_revisions`, `restore_revision`.
- `/mcp` accepts only `localhost`, `127.0.0.1` and the `public_origin` host (rmcp's DNS-rebinding guard).
- Fixed while testing: connectors to a deleted item stayed on screen; and a live update arriving while someone was editing an item would take the new version and let their save overwrite the other change silently. Now their save conflicts and shows the banner.
- Tests: 10 new integration tests (`tests/live.rs`: events, flows, MCP over JSON-RPC), Mermaid parser and layout unit tests, and `e2e/live.mjs` (12 browser checks).

**Done when:** Claude draws a laid-out flowchart in a frame from one prompt, it appears live in your open tab, and you can undo it from the revisions panel and trash.

---

## Phase 4: Rendering (M4)

**Goal:** drawings leave Board as PNG and SVG files, made by Board itself.

- [x] Add `ttf-parser`, `usvg`, `resvg`, `tiny-skia`, `insta` (tests). Bundle Inter regular and semibold in `fonts/` with its licence.
- [x] `render/text.rs`: glyph advances, greedy wrapping, hard breaks for long words, step-down to a minimum size, then ellipsis (§8.1).
- [x] `render/routing.rs`: connector routing in Rust, passing the same `fixtures/routing.json` as the UI.
- [x] `render/svg.rs`: frame and selection → SVG (shapes, stickies, text, cards as titled boxes, connectors, labels, arrowheads), print and dark themes mapped from palette names, text-as-paths option via `usvg`.
- [x] `render/png.rs`: `resvg` at scale 1–4, external resources disabled, size and time limits (§11), in-memory LRU keyed by an input hash.
- [x] Endpoints (§8.2): `render.svg`, `render.png`, selection render, signed image URLs (same HMAC as §3.3, 7-day default).
- [x] MCP `get_item_image`: returns the PNG as image content plus a signed URL.
- [x] UI: Export as PNG / SVG / Copy image link on frames and selections.
- [x] Tests: SVG snapshots for a representative frame, wrapping edge cases, routing fixtures, PNG smoke test, and checking a signed URL with an expired or tampered signature fails.

**Status:** done (2026-11-06). Notes:

- Inter 4.1 (Regular, SemiBold, Bold; OFL) is bundled in `crates/board/fonts/` and embedded in the binary. resvg is built without system fonts and without raster images, so renders match on every machine and an SVG can't load other files.
- Exports use Inter; the canvas uses Atkinson Hyperlegible. Layout, routing and colours match; the typeface differs by design.
- Signed links: `/i/{id}.png` and `/i/{id}.svg`, 7 days by default (1–90).
- Fixed while testing: the wider style bar covered a selected frame's tab, so the frame couldn't be dragged by it.
- Tests: render unit tests (text fitting, routing fixtures in Rust, a reviewed SVG snapshot, PNG size and limits), `tests/images.rs` (4), an MCP image check, and `e2e/export.mjs` (7 browser checks).

**Done when:** you can export a flowchart frame as a PNG that matches what's on screen, paste its image link into an email, and Claude can call `get_item_image` and describe what it sees.

---

## Phase 5: Board schema and sitemap (M5)

**Goal:** design a client's database and page structure properly.

### Backend

- [x] `board-model` crate: `SchemaTable`, `SchemaColumn` with the logical `ColumnType` enum and extras, `SchemaIndex`, `SitemapNode`, derive `schemars::JsonSchema` (§5.4, §14).
- [x] Tables service: create/update (versioned)/delete (refuse if referenced unless `force`)/undelete; table revisions; `schema_version` bump in the same transaction (§4.2, §5.4).
- [x] Ids: `tbl_`/`col_`/`pg_`; server assigns; **keep the id when a column (or page) without an id matches by name (or slug)**.
- [x] Input by name: resolve `{ table, column }`, table names, page slugs to ids on write; return ids and names on read.
- [x] Semantic validation (§5.5): identifier names, uniqueness, type extras, defaults, references, indexes; warnings array.
- [x] Schema card content (`table_ids`), sitemap card content (node ids), shape `page_id`.
- [x] `GET /api/items/{id}/er` (Mermaid source for the UI) and ER rendering in `render/` (layered by foreign key, crow's-foot ends, stubs for tables outside the view), so `render.svg`/`render.png` work on schema cards.
- [x] `table.*` SSE events with `schema_version`.
- [x] MCP: `get_schema`, `add_table`, `update_table`, `delete_table`; tool descriptions list the logical types.
- [x] `update_board` with `stack`.
- [x] Tests: id kept on rename-by-name, FK resolution across cards, refusing to delete a referenced table, `force` removing references, cardinality in the ER output, every validation rule.

### Frontend

- [x] Schema card: tables in view, column grid editor (logical type dropdown, per-type extras, FK picker across the board), Add table / Show existing table, Mermaid ER preview below.
- [x] Schema panel listing every table and which cards show it.
- [x] Sitemap editor: nested list, drag to reorder, slugs, purposes.
- [x] Shape "link to page" picker; slug shown under linked shapes.
- [x] Board settings: title, description, stack.

**Status:** done (2026-11-06). Notes:

- The rules (§5.4, §5.5) live in `crates/board/src/schema/` as pure functions with unit tests for each one. `service/tables.rs` stores the result. `board-model` holds the published types (`ColumnType`, `SchemaColumn`, `SchemaIndex`, `TableDefinition`, `SitemapNode`) with `JsonSchema`.
- Names are unique ignoring case, at most 64 characters (MySQL's limit). A reference without a column means the table's primary key. Removing a column another table references is refused (422) until the reference goes.
- Undeleting a table is refused (409) if its name is now taken, or if a table it references is still in the trash.
- Schema cards keep the ids of trashed tables in their content (so undelete brings them back); the UI, renderer and ER endpoint skip them.
- New pages and columns made in the UI get their `pg_`/`col_` id on the client, so an entry has one id from its first keystroke, even while saves are in flight. The server accepts any well-formed id that isn't used elsewhere.
- The card's ER preview is the server's own SVG, not Mermaid in the browser: one renderer, and no Mermaid bundle. `/er` still returns Mermaid source for other tools.
- A schema card exported on its own is its ER diagram; inside a frame, the diagram is drawn scaled into the card. Sitemap cards export as their page tree.
- Flows from `add_items` (layout auto) can link shapes to pages with `page`.
- Copy and paste: sitemap copies get new page ids. Pasting onto another board drops page links, and keeps a schema card's tables only where a table of the same name exists there.
- Also: debug builds now write line tables only, and no debug info for dependencies (the build directory went from 11 GB to 1.5 GB); `scripts/e2e.sh` takes test files to run.
- Tests: 20 schema rule tests, 4 content reference tests, ER render tests, `tests/schema.rs` (8), an MCP table test in `tests/live.rs`, and `e2e/schema.mjs` (25 browser checks).

**Done when:** you can model a real client's database across two schema cards with foreign keys between them, rename a column without its id changing, export the ER diagram as a PNG, and Claude can add a table by name that references an existing one.

---

## Phase 6: Features and the board model (M6)

**Goal:** Design (and any other project) can read the plan.

- [x] Feature card content, validation (table and page references) and editor: status, summary, stories, acceptance, table and page chips that pan to the right card (§5.2, §10.3).
- [x] `model.rs`: `board_model(board, items, connectors, tables)` (§9.2): tables in name order with filled-in labels and column hashes; pages; features, flows (frames with connectors, as nodes and edges) and notes in reading order; warnings.
- [x] `GET /api/boards/{id}/model` with `ETag` and 304; `GET /api/model.schema.json` (unauthenticated).
- [x] MCP `get_board_model`.
- [x] `board pull <board-id> --into <dir>`: writes `board-model.json`, pretty-printed with stable key order.
- [x] Publish `board-model` for Design (`board-model-v0.1.0`): tag a release so Design can use it as a git dependency (if Rust), or rely on the JSON Schema.
- [x] Create a `read` token for Design (`--name design --scope read --actor api`).
- [x] Tests: ordering, label defaults, hashes change only when a column's definition does, flow extraction, multiple-sitemap warning, ETag stability.

**Status:** done (2026-11-06). Notes:

- `board-model` now holds the whole model: `BoardModel`, `ModelTable`, `ModelColumn` (the column plus `hash`), `Feature`, `Flow`, `FlowNode`, `FlowEdge`, `Note`, `FeatureStatus`.
- Column hashes are the first 8 hex characters of a SHA-256 over the column without its id and without the names filled in on read. Renaming a referenced table doesn't change the hash; changing the column does.
- The model has no coordinates, so moving cards doesn't change the ETag. The ETag is a hash of the model's JSON.
- Features drop table and page ids that no longer resolve. Flow nodes keep a shape's `page_id` only if that page exists.
- `board pull` reads the local database, or fetches from `--from <url>` with `--token` (env `BOARD_URL`/`BOARD_TOKEN`), using `ureq` with rustls. It writes only when the content changed.
- Feature cards draw their status, progress and summary in exports. Copying one to another board keeps its tables by name and drops its pages.
- Tests: 6 model unit tests, feature validation tests, `tests/model.rs` (3), and `e2e/model.mjs` (13 browser checks, including a real `board pull`).

**Done when:** `curl` with the Design token returns a model for a real board, a second request returns 304, and `board pull` writes a file that diffs cleanly after a column change.

→ **Design track can start its Board import here** (see below).

---

## Phase 7: Typeset and the client PDF (M7)

**Blocked until Typeset's real input format is known (§15 Q1).** Update §8.4 to match it first.

- [ ] Move `typeset-doc` to the Typeset repo; Board depends on it by git tag. Add `contract_version`, the `image` block, `alt`/`width` on `svg`, `Unknown` blocks, and the `sources` request shape (§8.4, §14).
- [ ] Update `export.rs` to the v0.6 mapper (§8.6, §14): items instead of cards, render function argument, frame and feature mappers, rendered ER SVG, type formatting, heading offsets, duplicate removal, `item_ids` with `serde(default)`.
- [ ] Export selection rules: frames and cards, reading order, frames followed by their cards (§6 Export selection).
- [ ] Signed Typeset URLs (§3.3): `POST /api/boards/{id}/typeset-url`, `GET /api/boards/{id}/typeset`, MCP `get_typeset_url`, `get_typeset_document`.
- [ ] Push mode: `POST /api/boards/{id}/export/typeset` (`?dry=1` for JSON), `typeset_client.rs`, UI Export to PDF with a card/frame picker and `meta` fields.
- [ ] Diagram card SVG cache (`PUT /svg`, version check, light theme), as fallback for Mermaid-only diagrams (§4.4).
- [ ] Tests: the original 8 updated plus the new ones listed in §14.

**Done when:** Typeset produces a planning PDF from a board via a signed URL, with flowcharts and ER diagrams as sharp vector images, and Claude can ask for it.

---

## Phase 8: Polish (M8)

- [ ] Diagram cards: Mermaid editor with live preview, lazy-loaded, `securityLevel: "strict"`, Convert to shapes.
- [ ] Search UI (Cmd+K over items and tables, jump to result).
- [ ] Folder export zip (§12): `board.json`, `board-model.json`, card files, Mermaid and images per flow.
- [ ] Mobile: read-only pan and zoom.
- [ ] Rate limiting if needed (§11).

---

## Outside this repo

### Typeset track (needed for phase 7)

In order (§8.5):

1. `source` and `sources` input with an allow-list (Board and Design domains), timeouts and size caps.
2. `markdown` block.
3. `svg` block with sanitising and `width`.
4. `image` block (PNG/JPEG, allow-listed fetch, size caps).
5. `table` block.
6. `tree` and `checklist` blocks.
7. `planning-doc` template; later a `proposal` template.
8. `contract_version` check and `Unknown` block handling.
9. Raised body limit.

### Design track (after phase 6)

What Design needs to do to use Board (§9):

1. Read `/api/boards/{id}/model` server-side with the `read` token; poll with `If-None-Match`, or subscribe to SSE.
2. Generate page frames from `pages` and components from `tables`; store `{ board_id, table_id, column_id, hash }` on each generated field and show changed / renamed / removed fields (§9.3).
3. Embed Board drawings with signed image URLs.
4. Expose its own Typeset document (screens as `image` blocks with signed URLs) so Typeset can join it with Board's into the client proposal PDF (§8.3).

---

## The end state

One prompt to Claude, "make the proposal PDF for Acme", works because:

1. Board's MCP returns a signed Typeset URL for the board (planning: notes, features, sitemap, flows, data model).
2. Design's MCP returns a signed Typeset URL for the project (the screens).
3. Claude sends both to Typeset as `sources`, and Typeset returns one PDF.
