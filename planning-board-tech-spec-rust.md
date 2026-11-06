# Planning Board: Technical Spec (Rust)

Working name: **Board** (rename freely).
Status: draft v0.6. Section 16 lists what changed in each version. The build order lives in `plan.md`.

---

## 1. Overview

Board is a planning canvas for client website projects: the place a project gets thought through before anything is designed or built. A board is an infinite canvas where you can:

- **draw**: flowcharts, user journeys, process maps and brainstorms, with shapes, connectors, sticky notes, text and frames;
- **specify**: features, database schemas, sitemaps, checklists and notes, as typed cards that other tools can rely on;
- and mix the two freely: a "Checkout flow" frame can hold a flowchart whose "Save order" step points at the `orders` schema card, next to the "Online checkout" feature card.

It's Miro without being Miro: everything you draw is stored as structured data (nodes, edges, typed content), not pixels, so Claude can draw on it, Typeset can print it and Design can read it.

Board's most important outputs for other tools are the project's **database schema**, **sitemap**, **features** and **flows**. Design (design.kritano.com, a separate app) reads them to generate page frames, forms, lists and detail views (section 9). Board renders its own drawings to SVG and PNG, and Typeset turns a board (together with Design's screens) into a PDF for clients (section 8).

Everything lives in one SQLite file, behind one Rust binary. The system is **API-first**: a REST API is the contract, and the web UI, Claude (via MCP or plain HTTP), Typeset and Design are all clients of it.

### Goals

- A canvas that's quick to think on: draw a flowchart with the keyboard and mouse as fast as in Miro.
- Very fast, very small: one static binary plus `board.db`, frontend embedded.
- Everything movable: items drag freely, group into frames, and can move between boards.
- Claude can draw as well as write: a flowchart from a description, laid out automatically, ready to tidy by hand.
- One source of truth per project for the data model, page structure and flows, published as a stable, versioned JSON contract that Design and other tools read.
- A schema that isn't tied to a database: the stack is chosen per job, so Board uses generic column types.
- API-first, so scripts, Claude Code, cron jobs, Typeset and Design can all use it without special tooling.
- Nothing Claude does is irreversible: every content change and every delete can be undone from the UI.

### Non-goals (v1)

- Multi-user, real-time collaboration, comments, per-user permissions.
- Client-facing share links.
- Freehand pen drawing, image uploads, embeds.
- Specialist notations (UML, BPMN) beyond the flowchart shape set.
- Colour themes beyond a fixed palette; presentation mode.
- Visual design of any kind: mockups and wireframes are Design's job (section 9.5).
- Offline/local-first sync.

---

## 2. Architecture

```
                       ┌──────────────── Board (single Rust binary) ────────────────┐
Browser UI ─cookie─────┤  REST + SSE                                                │
Claude (MCP) ──/mcp────┤  service layer  ──  SQLite (WAL)  board.db                 │
Claude Code / scripts ─┤       ▲                                                    │
   (bearer token) REST ┤       ├── flow::{mermaid, layout}  (pure functions)        │
Design ──GET /model────┤       ├── model::board_model       (pure functions)        │
Typeset ─GET signed URL┘       └── export::to_typeset_document (pure functions)     │
        (pull)                                                                       │
Board ──POST doc──> Typeset /render (push)  ◄── same renderer, two entry points
```

### Decision: REST is the contract, MCP is an adapter

An MCP-only design would be the wrong route:

- Typeset, Design, cron jobs, scripts and Claude Code need plain HTTP. They shouldn't have to speak MCP.
- REST is trivially testable (`curl`), documentable (OpenAPI) and stable across MCP client quirks.
- MCP clients vary in quality and in how they handle large results; a REST fallback avoids being stuck.
- It keeps one code path: REST handlers and MCP tools both call the same service functions.

So: build the service layer and REST API first, then add MCP as a small wrapper (one tool per service operation, with schemas and descriptions tuned for Claude).

### Decision: one item model for everything on the canvas

Cards, shapes, sticky notes, text and frames are all **items** in one table with a `kind` column, and **connectors** join any two items. One model means one set of endpoints, one position/selection/undo path in the UI, one revision mechanism, and connectors that work between a flowchart step and a schema card without special cases.

### Stack

| Piece | Choice | Why |
|---|---|---|
| Language | Rust | Matches Typeset, one binary, shared types |
| HTTP | `axum` (0.8) + `tokio` | Same as Typeset; SSE is built in |
| DB | SQLite (WAL) via `sqlx` (runtime-checked queries, built-in migrations) | One file, transactions, safe with UI and MCP writing at once. Write transactions use `BEGIN IMMEDIATE` (section 4.2) |
| MCP | `rmcp` (official Rust SDK), Streamable HTTP server transport nested into the axum router at `/mcp` | Same process, shares the service layer. The crate moves fast, so check its current docs when you start |
| Validation | `serde` typed enums per item kind and card type, `deny_unknown_fields`, errors via `serde_path_to_error` | Replaces a schema library; bad shapes fail at deserialisation with a path-qualified error we can return verbatim |
| IDs | `ulid`, with type prefixes inside the schema (`tbl_`, `col_`, `pg_`) | Sortable, URL-safe, readable in logs |
| Flow layout | Own layered layout (section 5.3), no dependency | Small, deterministic, testable |
| Live updates | `tokio::sync::broadcast` feeding SSE | MCP/API writes appear on the board instantly |
| Search | SQLite FTS5 | Built in, no extra service |
| Contract docs | `schemars` for the board model's JSON Schema; `utoipa` for OpenAPI, optional | Non-Rust clients (Design, if it isn't Rust) can generate types |
| Frontend | Preact with `@preact/signals`, TypeScript, plain CSS, built with esbuild, embedded with `rust-embed` | About 8.5 KB gzipped together (measured); esbuild compiles the JSX with no plugin; signals re-render only what changed. Drags move elements directly and commit to state at the end |
| Rendering | Own SVG renderer, `ttf-parser` for text measurement, `usvg`/`resvg` for text-to-paths and PNG, Inter bundled | Board draws its own images without a browser (section 8.1) |
| Diagrams | Mermaid in the browser, lazy-loaded, for diagram cards only | Heavy, so keep it off the critical path |
| Deploy | One DigitalOcean droplet, systemd, Caddy/nginx in front | Same as your other services |

### Workspace layout

```
board/
  Cargo.toml                 # workspace
  crates/
    board-model/             # Board's published contract: schema, sitemap, features, flows (section 9)
      src/lib.rs             # types only, serde + schemars; Design depends on this if it's Rust
    board/                   # the app
      src/
        main.rs              # config, router, embedded assets, CLI subcommands
        db.rs                # pool setup (WAL, foreign_keys, busy_timeout), write-tx helper
        auth.rs              # session cookie, bearer tokens, scopes, signed export URLs
        service/             # boards, items, connectors, frames, schema tables, batch, revisions, trash, placement
        routes/              # REST handlers
        mcp.rs               # rmcp tool definitions (thin wrappers)
        events.rs            # broadcast channel + SSE
        flow/
          mermaid.rs         # Mermaid flowchart subset -> nodes + edges (pure)
          layout.rs          # layered auto-layout (pure)
        render/
          svg.rs             # frames, selections, ER diagrams -> SVG (pure)
          text.rs            # font metrics and line wrapping (pure)
          routing.rs         # connector routing; shares fixtures with the UI
          png.rs             # resvg rasterising, LRU cache
        fonts/               # Inter regular + semibold (OFL)
        model.rs             # builds the board model (pure)
        export/
          mod.rs             # = export.rs (content types + Typeset mapper)
          typeset_client.rs  # push to Typeset
      migrations/
  web/                       # frontend source; dist/ is embedded
```

Contract ownership follows the data: **`board-model` is owned by Board** (it's Board's output), and **`typeset-doc` is owned by Typeset** (it's Typeset's input). `typeset-doc` lives in the Typeset repo (or its own repo) as a small crate, and Board depends on it via a git dependency pinned to a tag. `typeset_doc.rs` delivered with this spec is the starting point for that crate.

The two `.rs` files delivered with this spec (`typeset_doc.rs`, `export.rs`) are pure functions with no I/O and ship with 8 passing unit tests. Dependencies are only `serde` and `serde_json`. They predate v0.3; section 14 lists the changes they need.

---

## 3. Auth

Three kinds of caller, three mechanisms:

| Caller | Mechanism |
|---|---|
| Browser UI | Session cookie |
| Claude (MCP), Claude Code, scripts, Design | Bearer token with a scope |
| Typeset (pull mode) | Signed, expiring URL for one board |

### 3.1 UI session

The browser can't usefully hold a bearer token: `EventSource` cannot send an `Authorization` header, and an admin token in `localStorage` is readable by any script on the page.

- `board login` (CLI) prints a one-time login link valid for 10 minutes, stored hashed in `login_codes`. Opening it sets the session cookie and returns a small page that moves on to the UI. A page, not a redirect: a redirect chain that starts on another site (a link clicked in chat or email) wouldn't carry the `SameSite=Strict` cookie. No password, no login form.
- Cookie: `HttpOnly`, `SameSite=Strict`, 30-day expiry, and `Secure` whenever `public_origin` is https (left off for plain-http local development). Stored server-side as a hashed session id in `sessions`.
- `SameSite=Strict` covers CSRF for this single-origin app. State-changing routes also reject requests whose `Origin` header isn't the Board origin.
- A session has `admin` rights and `actor = 'user'`.

```sql
CREATE TABLE sessions (
  id_hash      TEXT PRIMARY KEY,               -- sha256 of the cookie value
  created_at   TEXT NOT NULL,
  expires_at   TEXT NOT NULL,
  last_used_at TEXT
);

CREATE TABLE login_codes (
  code_hash  TEXT PRIMARY KEY,                 -- sha256 of the one-time code
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  used_at    TEXT
);
```

### 3.2 Bearer tokens

```sql
CREATE TABLE api_tokens (
  id           TEXT PRIMARY KEY,
  name         TEXT NOT NULL,                 -- "claude-mcp", "design", "laptop"
  token_hash   TEXT NOT NULL UNIQUE,          -- sha256 of the token
  scope        TEXT NOT NULL CHECK (scope IN ('admin','read')),
  actor        TEXT NOT NULL CHECK (actor IN ('claude','api')),
  created_at   TEXT NOT NULL,
  last_used_at TEXT
);
```

| Scope | Can do | Used by |
|---|---|---|
| `admin` | Everything | Claude via MCP or HTTP |
| `read` | All GET endpoints | Design, scripts, dashboards |

- `actor` is what gets written to `created_by` / `updated_by` / `changed_by`. It comes from the token, never from the request body. MCP and Claude-over-HTTP tokens use `actor = 'claude'`.
- Tokens are `brd_` plus 32 random bytes in base64url (the prefix lets secret scanners spot a leaked token), shown once on creation, stored as a SHA-256 hash and looked up by that hash. An attacker can't steer the hash of a guess, so lookup timing reveals nothing useful.
- CLI: `board token create --name design --scope read --actor api`, `board token list`, `board token revoke <id>`.

### 3.3 Signed export URLs (Typeset pull)

Typeset fetches with short-lived signed URLs for one board, so no long-lived secret passes through it:

```
GET /api/boards/{id}/typeset?items=a,b,c&page_breaks=1&exp=1730822400&sig=<base64url>
```

- `sig = HMAC-SHA256(server_secret, canonical string)`, where the canonical string is the path plus all query parameters except `sig`, sorted by key.
- `exp` is a Unix timestamp; default lifetime 10 minutes, max 1 hour.
- Signed URLs are minted with `POST /api/boards/{id}/typeset-url` (admin) or the MCP tool `get_typeset_url`. Claude can hand a URL to Typeset without ever seeing a secret.
- `server_secret` is 32 random bytes in the config file, generated on first run. Rotating it invalidates every outstanding URL.

### 3.4 Network

- Bind to localhost and expose only through the reverse proxy with TLS.
- Unauthenticated routes: embedded static assets, `/login/{one-time-code}`, `/healthz`, `/api/model.schema.json`. Everything else under `/api`, and `/mcp`, needs one of the mechanisms above.
- No CORS. Design calls Board from its server, not from the browser.

---

## 4. Data model

### 4.1 Schema

All timestamps are RFC 3339 UTC strings with millisecond precision (`2026-11-05T16:30:00.000Z`), generated by the service layer. Fixed width, so they compare correctly as strings. Coordinates are canvas units (1 unit = 1 CSS pixel at 100% zoom) and always absolute, including for items inside a frame.

```sql
-- migrations/0001_init.sql
CREATE TABLE boards (
  id             TEXT PRIMARY KEY,
  title          TEXT NOT NULL,
  description    TEXT,
  stack          TEXT,                           -- free text, e.g. "Laravel + MySQL"; a hint, not a constraint
  schema_version INTEGER NOT NULL DEFAULT 0,     -- bumps on any table create/update/delete/undelete
  archived       INTEGER NOT NULL DEFAULT 0,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL
);

-- Everything on the canvas (section 5).
CREATE TABLE items (
  id                 TEXT PRIMARY KEY,
  board_id           TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  kind               TEXT NOT NULL CHECK (kind IN ('card','shape','sticky','text','frame')),
  card_type          TEXT CHECK (card_type IN ('note','feature','schema','sitemap','checklist','diagram')),
  frame_id           TEXT REFERENCES items(id) ON DELETE SET NULL, -- containing frame; set by the server (4.5)
  title              TEXT,                       -- cards and frames
  content            TEXT NOT NULL,              -- JSON; shape per kind / card type
  style              TEXT,                       -- JSON; palette colour etc. (5.2)
  x                  REAL NOT NULL DEFAULT 0,
  y                  REAL NOT NULL DEFAULT 0,
  w                  REAL NOT NULL,
  h                  REAL NOT NULL,
  z                  INTEGER NOT NULL DEFAULT 0,
  locked             INTEGER NOT NULL DEFAULT 0, -- can't be moved or resized until unlocked
  version            INTEGER NOT NULL DEFAULT 1, -- optimistic locking; bumps on title/content/style change only
  rendered_svg       TEXT,                       -- cached browser-rendered SVG (diagram cards)
  svg_for_version    INTEGER,                    -- item version the SVG was rendered from
  created_by         TEXT NOT NULL CHECK (created_by IN ('user','claude','api')),
  updated_by         TEXT NOT NULL CHECK (updated_by IN ('user','claude','api')), -- last title/content/style change
  content_updated_at TEXT NOT NULL,
  deleted_at         TEXT,                       -- soft delete; NULL = live
  deleted_by         TEXT CHECK (deleted_by IN ('user','claude','api')),
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL,              -- any change, including position
  CHECK ((kind = 'card') = (card_type IS NOT NULL)),
  CHECK (kind <> 'frame' OR frame_id IS NULL)    -- frames don't nest
);
CREATE INDEX idx_items_board ON items(board_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_items_frame ON items(frame_id) WHERE frame_id IS NOT NULL;
CREATE INDEX idx_items_trash ON items(deleted_at) WHERE deleted_at IS NOT NULL;

CREATE TABLE connectors (
  id          TEXT PRIMARY KEY,
  board_id    TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  from_item   TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  to_item     TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  from_anchor TEXT NOT NULL DEFAULT 'auto' CHECK (from_anchor IN ('auto','top','right','bottom','left')),
  to_anchor   TEXT NOT NULL DEFAULT 'auto' CHECK (to_anchor IN ('auto','top','right','bottom','left')),
  label       TEXT,
  style       TEXT,                              -- JSON: line, arrowheads, routing, colour (5.2)
  created_by  TEXT NOT NULL CHECK (created_by IN ('user','claude','api')),
  deleted_at  TEXT,
  deleted_by  TEXT CHECK (deleted_by IN ('user','claude','api')),
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  CHECK (from_item <> to_item)
);
CREATE INDEX idx_connectors_board ON connectors(board_id) WHERE deleted_at IS NULL;

-- The board schema: one row per table (section 5.4). Not item content.
CREATE TABLE schema_tables (
  id                 TEXT PRIMARY KEY,           -- tbl_<ulid>
  board_id           TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name               TEXT NOT NULL,
  definition         TEXT NOT NULL,              -- JSON: comment, columns (with ids), indexes
  version            INTEGER NOT NULL DEFAULT 1,
  created_by         TEXT NOT NULL CHECK (created_by IN ('user','claude','api')),
  updated_by         TEXT NOT NULL CHECK (updated_by IN ('user','claude','api')),
  content_updated_at TEXT NOT NULL,
  deleted_at         TEXT,
  deleted_by         TEXT CHECK (deleted_by IN ('user','claude','api')),
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_tables_name ON schema_tables(board_id, name) WHERE deleted_at IS NULL;

CREATE TABLE item_revisions (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  item_id    TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  title      TEXT,
  content    TEXT NOT NULL,
  style      TEXT,
  version    INTEGER NOT NULL,
  changed_by TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL                       -- last time this revision was coalesced into
);
CREATE INDEX idx_item_revisions ON item_revisions(item_id, id);

CREATE TABLE table_revisions (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  table_id   TEXT NOT NULL REFERENCES schema_tables(id) ON DELETE CASCADE,
  name       TEXT NOT NULL,
  definition TEXT NOT NULL,
  version    INTEGER NOT NULL,
  changed_by TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX idx_table_revisions ON table_revisions(table_id, id);

-- Search: titles and text of items and tables, kept in sync by the service layer.
CREATE VIRTUAL TABLE search_fts USING fts5(entity_kind UNINDEXED, entity_id UNINDEXED, board_id UNINDEXED, title, body);

-- + sessions, login_codes, api_tokens (section 3)
```

Notes:

- **Connectors stay on one board.** Both ends must be live items on the connector's board. Connectors can cross frames. Two connectors may join the same pair of items (a "yes" and a "no" branch can both lead back to the same step).
- **Soft delete.** `DELETE` sets `deleted_at`/`deleted_by` and leaves the row, its connectors and its revisions in place. Deleting an item hides its connectors; undeleting it brings them back. Deleted items, connectors and tables are excluded from every read except the trash endpoints. Items in the trash for more than 30 days are hard-deleted by a daily task, and only then do revisions and connectors cascade away. A board `DELETE` is still a hard delete (UI confirms; not exposed over MCP).
- **Board reads leave out `rendered_svg`.** `GET /api/boards/{id}` and `get_board` select every column except `rendered_svg`; the SVG is only loaded by the exporter.

### 4.2 Pool and write transactions

Pool setup: `journal_mode=WAL`, `foreign_keys=ON`, `busy_timeout=5000`, a small pool (about 5).

In WAL mode, a transaction that starts as a reader and later writes can fail immediately with `SQLITE_BUSY` if another writer got in first; `busy_timeout` does not help in that case. Item and table updates are exactly that shape (read version → update → insert revision → prune → bump `schema_version`). So:

- Every write transaction starts with `BEGIN IMMEDIATE`, through one helper in `db.rs`. Recent sqlx versions support this with `begin_with`; if the version in use doesn't, use a second pool with one connection for writes.
- The version check is atomic: `UPDATE … SET … , version = version + 1 WHERE id = ? AND version = ? AND deleted_at IS NULL`, then check the affected row count. Zero rows → load the record and return 409.
- Any change to a table bumps `boards.schema_version` in the same transaction.
- A batch (section 6) is one transaction: all of it applies or none of it does.

### 4.3 Revisions

Items and schema tables are revised the same way. A revision is the record's title/name, content/definition and style *after* a change. Position changes are not revisions (the UI's own undo covers those, section 10.4). Revisions are coalesced so autosave doesn't flood them:

- On each change, if the newest revision has the same `changed_by` and its `updated_at` is less than 5 minutes ago, **update that revision**. Otherwise insert a new one.
- After an insert, prune to the newest 20 revisions for the record. Same transaction.
- Restoring a revision is an ordinary update by the caller (it bumps the version and creates or coalesces a revision of its own), so a restore can itself be undone.
- A change by a different actor always starts a new revision. That's what makes "undo what Claude just did" a single step.

### 4.4 SVG cache and freshness

Only Mermaid diagram cards use the SVG cache. Everything else (frames and other drawings, ER diagrams) is drawn by Board's own renderer on demand (section 8.1).

- An item's cached SVG is only valid if `svg_for_version == version`. The mapper (`Item::fresh_svg`) enforces this, so a stale diagram is never exported silently.
- `PUT /api/items/{id}/svg` with a `for_version` that isn't the item's current version returns 409 and stores nothing.
- The UI renders export SVGs with a fixed light theme, independent of the UI's own colour scheme, so a dark-mode session never caches a dark diagram for print.

### 4.5 Frames and membership

- The server decides membership: whenever an item is created, moved or resized, and whenever a frame is moved or resized, an item's `frame_id` becomes the topmost live frame whose rectangle contains the item's centre, or `NULL`. Every client (UI, MCP, scripts) gets the same answer.
- Moving a frame moves its members by the same delta, in the same transaction (one request, not one per member). Locked members move with their frame.
- Deleting a frame deletes only the frame unless `with_contents` is set; members are released (`frame_id = NULL`) and stay where they are.
- Frames don't nest in v1.

---

## 5. Items

### 5.1 Item kinds

Content is stored as JSON text and parsed into a Rust enum via `ItemContent::from_parts(kind, card_type, json)`. That function is also the write-time validation, so REST and MCP get identical errors.

| Kind | Content | Default size | Notes |
|---|---|---|---|
| `shape` | `{ "shape": "rect" \| "rounded" \| "diamond" \| "ellipse" \| "parallelogram" \| "cylinder" \| "document", "text": "…", "page_id?": "pg_…" }` | rect/rounded/parallelogram/document 160×72, diamond 150×110, ellipse 150×90, cylinder 130×100 | Flowchart steps, decisions, inputs, data stores. Text is plain (line breaks allowed), drawn centred and wrapped inside the shape |
| `sticky` | `{ "text": "…" }` | 180×180 | Brainstorming. Default colour yellow |
| `text` | `{ "text": "…", "size": "s" \| "m" \| "l" \| "xl" }` | 240×48 | Free labels and headings on the canvas. No box |
| `frame` | `{ "description?": "…" }` | 800×600 | Titled area that groups items (4.5). The unit of export for drawings (section 8) |
| `card` | per `card_type` (5.2) | 320×240 | Structured content |

Shapes are the flowchart set: `rect` (process), `rounded` (start/end), `diamond` (decision), `ellipse` (event/state), `parallelogram` (input/output), `cylinder` (data store), `document` (document/report).

**`page_id` on shapes** links a step in a user journey to a page in the sitemap. That's what lets Design (section 9) work out which screens lead to which, and lets the UI show the page's slug on the shape.

### 5.2 Cards

| Card type | JSON shape |
|---|---|
| `note` | `{ "markdown": "…" }` |
| `feature` | `{ "status": "idea" \| "planned" \| "in_progress" \| "done", "summary": "markdown", "stories?": ["As a … I want … so that …"], "acceptance?": [{ "text", "done?" }], "table_ids?": [], "page_ids?": [] }` |
| `schema` | `{ "table_ids": ["tbl_…", …] }`: a view onto the board schema (5.4) |
| `sitemap` | `{ "root": { "id", "title", "slug?", "purpose?", "children?": [ …nodes ] } }` |
| `checklist` | `{ "items": [{ "text", "done?", "group?" }] }` |
| `diagram` | `{ "syntax": "mermaid", "source": "…" }` |

- **Feature cards** are the unit of feature planning: what it is, who it's for, how you'll know it's done, and which tables and pages it touches.
- **Schema cards** don't hold tables; they choose which of the board's tables to show. A table can appear on several schema cards. Deleting a schema card never deletes a table.
- **Sitemap nodes** have stable ids (`pg_<ulid>`). Design makes one frame per page and keys it by this id. A board is expected to have one sitemap card; a second is allowed, but the board model warns (section 9.2).
- **Diagram cards** are for diagram types the canvas doesn't draw natively (sequence diagrams, Gantt charts, state machines). Flowcharts should be drawn with shapes; a Mermaid flowchart in a diagram card can be converted to shapes in one click (5.3).

### Style

`style` is the same small object for every kind, and only the fields that make sense for the kind are allowed:

| Field | Values | Applies to |
|---|---|---|
| `color` | `grey`, `blue`, `green`, `yellow`, `orange`, `red`, `purple`, `pink` | shapes (fill tint), stickies (background), text (colour), frames (header), cards (header accent) |
| `align` | `left`, `center`, `right` | shapes, text |

Connector style:

| Field | Values | Default |
|---|---|---|
| `line` | `solid`, `dashed`, `dotted` | `solid` |
| `start` / `end` | `none`, `arrow`, `open_arrow`, `dot` | `none` / `arrow` |
| `routing` | `straight`, `elbow`, `curved` | `elbow` |
| `color` | palette names as above | `grey` |

Colours are palette names, not hex values, so the UI (light and dark), Typeset (print) and Design can each map them to their own concrete colours.

Adding a kind or card type means: one enum variant, one mapper function, one editor or renderer, and a migration for the CHECK constraint.

### 5.3 Flows from Claude: auto-layout and Mermaid

Claude (or any API client) can create a whole flowchart in one call without choosing coordinates.

**Input** is either structured nodes and edges:

```json
{
  "direction": "TB",
  "frame_title": "Checkout flow",
  "nodes": [
    { "ref": "start", "shape": "rounded", "text": "Basket" },
    { "ref": "auth", "shape": "diamond", "text": "Logged in?" },
    { "ref": "login", "shape": "rect", "text": "Log in", "page": "/login" },
    { "ref": "pay", "shape": "rect", "text": "Payment", "page": "/checkout/payment" }
  ],
  "edges": [
    { "from": "start", "to": "auth" },
    { "from": "auth", "to": "pay", "label": "yes" },
    { "from": "auth", "to": "login", "label": "no" },
    { "from": "login", "to": "pay" }
  ]
}
```

or a **Mermaid flowchart**, parsed in Rust (`flow::mermaid`) into the same structure. The supported subset:

- `flowchart TD|TB|LR|RL|BT` (and `graph …`)
- Nodes: `A[text]` rect, `A(text)` rounded, `A{text}` diamond, `A((text))` ellipse, `A[/text/]` parallelogram, `A[(text)]` cylinder
- Edges: `-->`, `---`, `-.->` (dashed), `==>` (solid), labels as `-->|text|` or `-- text -->`, chains (`A --> B --> C`)
- `subgraph title … end` → a frame (one level)
- Anything else is an error that names the line, so Claude can fix it.

The same parser powers "Convert to shapes" on a diagram card, and `GET /api/boards/{id}/flows/{frame_id}/mermaid` turns a frame back into Mermaid source, so Claude can read a hand-drawn flow compactly.

**Layout** (`flow::layout`, pure and deterministic):

1. Break cycles by reversing back edges found in a depth-first search from the sources (in input order).
2. Assign layers by longest path from the sources.
3. Order nodes within each layer by the barycentre of their neighbours in the previous layer, two down-and-up sweeps.
4. Place with fixed spacing: 60 units between nodes in a layer, 90 between layers, nodes at their default size. `TB` grows downwards, `LR` to the right.
5. Wrap in a frame (if `frame_title` is given) with 40 units of padding, and place the whole group in free space with the auto-placement rule (section 7), or next to a given item (`near`).

The result is ordinary items and connectors: nothing about the flow is special afterwards, and the user can drag it into shape.

### 5.4 The board schema

Each board has **one schema**: its set of tables. A board is one job, so each job gets its own schema. Tables are first-class rows (`schema_tables`), not item content, so foreign keys are checked across the whole board and a table can't be defined twice.

A table's `definition`:

```json
{
  "comment": "Customer orders",
  "columns": [
    { "id": "col_01J…", "name": "id", "type": "bigint", "primary": true, "generated": "increment" },
    { "id": "col_01J…", "name": "user_id", "type": "bigint",
      "references": { "table_id": "tbl_01J…", "column_id": "col_01J…" } },
    { "id": "col_01J…", "name": "status", "type": "enum", "values": ["pending", "paid", "shipped"], "default": "pending" },
    { "id": "col_01J…", "name": "total", "type": "decimal", "precision": 10, "scale": 2 },
    { "id": "col_01J…", "name": "notes", "type": "text", "nullable": true, "label": "Internal notes" },
    { "id": "col_01J…", "name": "created_at", "type": "datetime", "generated": "now" }
  ],
  "indexes": [ { "column_ids": ["col_01J…", "col_01J…"], "unique": false } ]
}
```

Column fields:

| Field | Meaning |
|---|---|
| `id` | Stable column id. Survives renames and type changes |
| `name` | Database identifier |
| `type` | Logical type (below) |
| `length?` | `string` only |
| `precision?`, `scale?` | `decimal` only |
| `values?` | `enum` only; non-empty, unique |
| `nullable?` | Default `false` |
| `default?` | Literal default, as a string; must be valid for the type where that can be checked (enum member, `true`/`false`, number) |
| `primary?`, `unique?` | Keys. A composite primary key is several columns with `primary: true` |
| `generated?` | `increment`, `uuid`, `now`, `now_on_update`. Set by the database, so Design leaves it out of create forms |
| `references?` | `{ table_id, column_id }` |
| `label?` | Human label for forms and docs; defaults to the name, humanised |
| `comment?` | Notes |
| `native_type?` | Stack-specific type when it matters (`jsonb`, `varchar(191)`). Free text, recorded but not validated |

#### Logical types

The stack is chosen per job, so the schema doesn't target a database. Columns use a small fixed set of logical types, which every stack can express and which Design can map to inputs reliably. Free text like `varchar(20)` can't be mapped.

| Type | Extras | Typical form input (Design's choice) |
|---|---|---|
| `string` | `length?` | Text input |
| `text` | | Textarea |
| `integer`, `bigint` | | Number input (a `references` column becomes a select instead) |
| `decimal` | `precision`, `scale` | Number input with step |
| `boolean` | | Toggle |
| `date` | | Date picker |
| `time` | | Time picker |
| `datetime` | | Date and time picker |
| `uuid` | | Hidden or read-only |
| `json` | | Code editor |
| `enum` | `values` | Select or radio group |
| `binary` | | File upload |

`boards.stack` (free text, for example "Laravel + MySQL") records what a job actually uses. It tells Claude and future exporters which native types to prefer. Neither it nor `native_type` changes validation.

#### Ids and input by name

- Ids: tables `tbl_<ulid>`, columns `col_<ulid>`, pages `pg_<ulid>`. The server assigns ids to new items; clients send ids back for existing ones.
- REST and MCP accept references by name on input, and the server resolves them to ids on write: `"references": { "table": "users", "column": "id" }`, feature `"tables": ["orders"]`, feature `"pages": ["/booking"]` and shape `"page": "/login"` (by slug), schema card `"tables": ["users", "orders"]`. Responses always return ids and names together. Claude never has to handle ULIDs by hand.
- When a table definition is updated, a column without an `id` whose `name` matches an existing column keeps that column's id. A rename must carry the id. Sitemap nodes work the same way, matched by slug. Field provenance in Design depends on ids surviving edits, so this rule matters.

#### Deleting tables

- Deleting a table that other tables reference is refused with a list of the referencing columns, unless `force` is given, in which case those `references` are removed in the same transaction (one schema change, undoable via revisions and the trash).
- Schema cards and feature cards keep the ids of deleted tables but hide them on read; undeleting the table brings them back.

### 5.5 Validation

Two stages, both returning errors with a JSON path:

1. **Shape (serde).** Every content, style and definition struct has `#[serde(deny_unknown_fields)]`, so a typo like `"nulable": true` is an error instead of a silently dropped field. Deserialisation runs through `serde_path_to_error`, so errors read `columns[2].type: missing field \`type\`` rather than just ``missing field `type` ``.
2. **Semantics.**
   - Style fields only on kinds they apply to (5.2).
   - Table and column names match `^[A-Za-z_][A-Za-z0-9_]*$`.
   - Table names are unique on the board; column names are unique in a table.
   - Type extras only on their type (`length` on `string`, and so on); `scale <= precision`; enum `values` non-empty and unique; `default` valid for the type where checkable.
   - `references` resolve to a live table and column on the same board. A type mismatch between the two columns is a warning, not an error.
   - Index columns exist in their table.
   - Schema card and feature `table_ids` resolve to tables on the board; feature `page_ids` and shape `page_id` resolve to sitemap nodes on the board.
   - Sitemap slugs and node ids are unique on the board.
   - Connector ends are live items on the same board and not the same item.

Warnings come back in a `warnings` array on the response and never block the write.

Error response shape, shared by REST (422) and MCP (tool error):

```json
{ "error": "invalid_content", "path": "columns[2].type", "message": "missing field `type`" }
```

Cap content at 512 KB per item and per table definition. Cap a board at 5,000 live items (a sanity limit, not a design target).

---

## 6. REST API

`{param}` path syntax is used (axum 0.8). Auth per section 3.

### Boards, items and connectors

| Method | Route | Purpose |
|---|---|---|
| GET | `/api/boards` | List boards |
| POST | `/api/boards` | Create board |
| GET | `/api/boards/{id}` | Board with live items (no `rendered_svg`), connectors, and table summaries |
| PATCH | `/api/boards/{id}` | Rename, describe, set `stack`, archive |
| DELETE | `/api/boards/{id}` | Delete board (hard) |
| POST | `/api/boards/{id}/items` | Create item (`kind`, `card_type?`, `title?`, `content`, `style?`, optional geometry; auto-placed otherwise) |
| GET | `/api/items/{id}` | Full item |
| PATCH | `/api/items/{id}` | Update title/content/style. Requires `version`; 409 with current item on mismatch |
| PATCH | `/api/items/{id}/geometry` | x, y, w, h, z, locked (no version check, last write wins). On a frame, moves its members too |
| POST | `/api/items/move` | Move items to another board: `{ item_ids, board_id, dx?, dy? }` (rules below) |
| DELETE | `/api/items/{id}` | Soft delete (to trash). Frames: `?with_contents=true` deletes members too. Returns `{ deleted, affected }`: what went to the trash, and items whose frame membership changed |
| POST | `/api/items/{id}/undelete` | Restore from trash |
| PUT | `/api/items/{id}/svg` | Store rendered SVG for a diagram card: `{ svg, for_version }`; 409 if `for_version` isn't current |
| GET | `/api/items/{id}/revisions` | List revisions |
| POST | `/api/items/{id}/restore/{rev}` | Restore a revision (an ordinary versioned update; requires current `version`) |
| POST | `/api/boards/{id}/connectors` | Create connector |
| PATCH | `/api/connectors/{id}` | Label, anchors, style (last write wins) |
| DELETE | `/api/connectors/{id}` | Soft delete |
| POST | `/api/boards/{id}/batch` | Several operations in one transaction (below) |
| POST | `/api/boards/{id}/flows` | Create a laid-out flow from nodes and edges or Mermaid (5.3) |
| POST | `/api/items/{id}/convert-to-shapes` | Diagram card with a Mermaid flowchart → shapes and connectors next to it |
| GET | `/api/boards/{id}/flows/{frame_id}/mermaid` | A frame's shapes and connectors as Mermaid source |
| GET | `/api/boards/{id}/trash` | Deleted items, connectors and tables for a board |
| GET | `/api/search?q=` | FTS over items and tables, live only |
| GET | `/api/boards/{id}/events` | SSE stream of changes |

### Batch

`POST /api/boards/{id}/batch` takes `{ "ops": [ … ] }` and applies them in one transaction. Ops: `create_item`, `update_item` (with `version`), `set_geometry`, `delete_item`, `undelete_item`, `create_connector`, `update_connector`, `delete_connector`. A `create_item` can carry a `ref`, and later ops in the same batch can use that `ref` anywhere an item id is expected, so "create three shapes and connect them" is one request.

The response has one result per op, the `refs` map (ref → new id), and `affected`: items the ops changed as a side effect (a frame's members moving with it, frame membership changing), in their final state. Clients apply `affected` rather than re-implementing the server's rules. Connector ops (`create_connector`, `update_connector`, `delete_connector`, `undelete_connector`) take refs too, so a whole flow can be created in one batch.

The UI uses batches for multi-select moves, paste, duplicate, align and group undo. MCP's `add_items` is a batch.

### Moving items between boards

- Shapes, stickies, text, frames (with their members), and note, checklist and diagram cards can move.
- Schema, feature and sitemap cards can't: they refer to tables and pages that belong to their board. A move that includes one is refused; copy the content instead.
- Connectors between two moved items move with them. Connectors from a moved item to one that stays behind are deleted (and emit `connector.deleted`).

### Schema

| Method | Route | Purpose |
|---|---|---|
| GET | `/api/boards/{id}/schema` | All live tables with full definitions, plus `schema_version` |
| POST | `/api/boards/{id}/tables` | Create table: `{ name, definition, item_id? }`. `item_id` also adds it to that schema card |
| GET | `/api/tables/{id}` | One table |
| PATCH | `/api/tables/{id}` | Update name/definition. Requires `version`; 409 with current table on mismatch |
| DELETE | `/api/tables/{id}` | Soft delete. 409 listing referencing columns unless `?force=1` |
| POST | `/api/tables/{id}/undelete` | Restore from trash |
| GET | `/api/tables/{id}/revisions` | List revisions |
| POST | `/api/tables/{id}/restore/{rev}` | Restore a revision |
| GET | `/api/items/{id}/er` | Mermaid `erDiagram` source for a schema card's tables, for the UI to render |

### Design and export

| Method | Route | Purpose |
|---|---|---|
| GET | `/api/boards/{id}/model` | **Board model** for Design (section 9). `ETag`; `If-None-Match` returns 304 |
| GET | `/api/model.schema.json` | JSON Schema for the board model |
| POST | `/api/boards/{id}/typeset-url` | Mint a signed pull URL (section 3.3). Body: same options as the query below |
| GET | `/api/boards/{id}/typeset` | **Ready-made Typeset document** (section 8). Signed URL or admin/read auth. Query: `items=a,b,c`, `page_breaks=1`, `title`, `subtitle`, `client`, `author`, `version`, `template` |
| POST | `/api/boards/{id}/export/typeset` | Push mode: body is `ExportOptions`; renders via Typeset and returns the PDF (`?dry=1` returns the JSON) |
| GET | `/api/boards/{id}/export/folder` | Zip of markdown/JSON files and rendered images (escape hatch, no lock-in) |
| GET | `/api/items/{id}/render.svg`, `/render.png` | Rendered images; see section 8.2 for these and the other image routes |

### Export selection

The units of export are **frames** and **cards**. Loose shapes, stickies and text outside any frame aren't exported; put a drawing in a frame to include it.

When `items` / `item_ids` is omitted or empty, the export includes every top-level frame and every card that isn't in a frame, in **reading order**: grouped into 200px bands by `y`, then sorted by `x` within a band. An exported frame includes its drawing and then the cards inside it (section 8.6). Duplicate ids in an explicit list are dropped after their first appearance. `template` must be one of the templates Typeset reports (initially just `planning-doc`).

### Body limits

axum's default 2 MB body limit is replaced with per-route limits: 1 MB default, 2 MB for `PUT /svg` and batches, 16 MB for push export (it can carry `svg_overrides`).

### Conflicts

Content updates carry `version`; a stale write returns 409 with the current record so the caller can merge. Geometry and connector updates are last-write-wins.

### Live updates (SSE)

Events: `item.created`, `item.updated`, `item.geometry`, `item.deleted`, `item.undeleted`, `item.moved`, `connector.created`, `connector.updated`, `connector.deleted`, `table.created`, `table.updated`, `table.deleted`, `table.undeleted`, `board.updated`, `batch`, `resync`.

- Each event's data is `{ "origin": …, "changes": [{ "type": …, "item" | "connector" | "id" | "board": … }] }`. The SSE event name is the change type when there's one change, or `batch` for several. A `ready` event opens every stream.
- `origin` is the `X-Client-Id` header of the request that caused it (the UI sends a random id per tab; MCP and scripts send none). A tab ignores events with its own `origin`.
- While an item is being edited in a tab (or has unsaved changes there), incoming changes to it update its position but leave its text and version alone, so the tab's next save conflicts and shows "changed elsewhere" instead of overwriting the other change.
- A batch, a frame move and a flow creation are sent as one `batch` event containing the individual events, so other tabs apply them in one go rather than animating 40 separate changes.
- `item.geometry` events carry the item's new `frame_id`, since membership can change on a move.
- `table.*` events also carry the new `schema_version`.
- `item.moved` is cross-board only and is sent to both boards' streams with `from_board` and `to_board`.
- There's no replay. If a subscriber falls behind and the broadcast channel drops messages, the server sends `resync` and the client reloads the board. The client also reloads the board on every (re)connect.

---

## 7. MCP server

Mounted at `/mcp` (Streamable HTTP), bearer auth with an `admin` token whose `actor` is `claude`. Each tool is a thin wrapper over a service function. The server accepts requests addressed to `localhost`, `127.0.0.1` and the host in `public_origin` only (DNS-rebinding protection), so `public_origin` must be the real domain in production.

| Tool | Args | Notes |
|---|---|---|
| `list_boards` | none | id, title, stack, item count, table count |
| `create_board` | `title`, `description?`, `stack?` | |
| `update_board` | `board_id`, `title?`, `description?`, `stack?` | |
| `get_board` | `board_id`, `frame_id?`, `kinds?` | Items with `id`, `kind`, `card_type`, `title`/text, `version`, geometry, `frame_id`; content truncated to ~2 KB per item (flagged); connectors; table names and ids. Filters keep large boards readable |
| `get_item` | `item_id` | Full content and `version` |
| `search` | `query`, `board_id?` | FTS over items and tables |
| `add_items` | `board_id`, `items` (each with optional `ref`, geometry), `connectors?` (by `ref` or id), `layout?` (`auto` \| `none`), `direction?`, `frame_title?`, `near?` | One batch. With `layout: "auto"` and no geometry, lays the items out as a flow (5.3). Returns ids keyed by `ref`, plus any `warnings` |
| `draw_flow` | `board_id`, `mermaid`, `frame_title?`, `near?` | Mermaid flowchart subset → laid-out shapes and connectors in a frame |
| `get_flow` | `frame_id` | A frame's drawing as Mermaid source: the compact way for Claude to read a hand-drawn flow |
| `update_item` | `item_id`, `version`, `title?`, `content?`, `style?` | Returns the new version; conflict error carries the current item |
| `set_geometry` | `updates`: `[{ item_id, x?, y?, w?, h? }]` | One batch. Moving a frame moves its members |
| `move_items_to_board` | `item_ids`, `board_id`, `dx?`, `dy?` | Rules as REST |
| `delete_items` | `item_ids`, `force?`, `with_contents?` | Soft delete. Refuses any item with `updated_by = 'user'` and `content_updated_at` under 5 minutes old, unless `force` |
| `connect` | `from`, `to`, `label?`, `from_anchor?`, `to_anchor?`, `style?` | |
| `update_connector` | `connector_id`, `label?`, anchors, `style?` | |
| `delete_connector` | `connector_id` | |
| `get_schema` | `board_id` | All tables with full definitions and `schema_version` |
| `add_table` | `board_id`, `name`, `definition`, `item_id?` | References by name are fine. `item_id` also puts it on that schema card |
| `update_table` | `table_id`, `version`, `name?`, `definition?` | Same keep-the-id rules as REST (5.4) |
| `delete_table` | `table_id`, `force?` | Refuses if referenced unless `force`; same 5-minute rule as `delete_items` |
| `list_revisions` | `item_id` or `table_id` | |
| `restore_revision` | `item_id` or `table_id`, `revision_id`, `version` | |
| `get_board_model` | `board_id` | The same board model Design reads (section 9). Lets a Claude Code session in another project read the plan |
| `get_typeset_document` | `board_id`, `item_ids?` | Returns the export payload for review |
| `get_typeset_url` | `board_id`, `item_ids?`, `page_breaks?`, `meta?` | Signed pull URL to hand to Typeset, alone or as one of several `sources` (8.3) |
| `get_item_image` | `item_id` (frame, schema card or diagram card), `format?` (`png` \| `svg`), `scale?` | Returns the PNG as image content, so Claude can see what it drew, plus a signed image URL |

Rules:

- Tool descriptions state the exact content schema per kind and card type, the style fields, the Mermaid subset, the table definition shape and the logical type list. Validation errors are returned verbatim with their path, so Claude self-corrects.
- Tool descriptions steer Claude: flows with `draw_flow` or `add_items` (not diagram cards), data with `add_table`, pages with a sitemap card, user-journey steps linked to pages with `page`. Drawings that belong in a planning PDF go in a frame.
- Every Claude write is recorded with `actor = 'claude'`. Content changes start a new revision and deletes go to the trash, so either can be undone from the UI in one step.
- `force` on deletes is advisory, not a security boundary. It's there to make Claude pause; the trash is the real safety net.
- Also expose `board://{id}` as a resource for cheap context loading.

### Auto-placement

When no position is given for an item or a group: take the board's live items; starting at the left edge of the board's bounding box (or 0,0 for an empty board), scan a 4-column grid with 360 × 280 cells row by row, and use the first position where the item's (or group's) rectangle plus a 40-unit margin overlaps nothing. With `near`, scan outwards from the right of that item first, then below it.

### Client support

Claude Code connects with a static bearer token (`claude mcp add --transport http … --header "Authorization: Bearer …"`). Remote connectors in claude.ai and Claude Desktop generally expect OAuth rather than a static header; see open question 5.

Plain-HTTP alternative: Claude Code (or any script) can skip MCP and call the REST API directly. A short skill file or the OpenAPI document is enough for that.

---

## 8. Rendering and Typeset

### 8.1 Board's renderer

Board draws its own images. One renderer (`render/`, pure Rust) turns frames and schema diagrams into SVG, and `resvg` turns SVG into PNG. No browser is involved, so it works in pull mode, from MCP, and for items that have never been opened in the UI.

The same output is used everywhere:

| Need | What it gets |
|---|---|
| PDF via Typeset | SVG, inline in an `svg` block of Board's Typeset document |
| Email, slides, chat | PNG or SVG download from a frame's menu |
| Design | Signed image URLs it can embed in its own frames |
| Claude | The PNG itself (MCP image content), so it can check what it drew, plus a URL |

What it renders:

| Source | Output |
|---|---|
| Frame | Every member (shapes, stickies, text, cards drawn as titled boxes) and the connectors between members, cropped to the frame, with the frame title optional |
| Selection | The same for an arbitrary set of items (UI export of a selection), cropped to their bounding box plus padding |
| Schema card | ER diagram of the card's tables: boxes laid out with `flow::layout` (parents above children, by foreign key), crow's-foot connectors (cardinality rules in 8.4) |
| Diagram card | Its cached browser SVG, if fresh (4.4). PNG via `resvg`. No server-side Mermaid |

**Text.** SVG can't wrap text by itself, so the renderer does it: a bundled font (Inter, OFL licence, regular and semibold, embedded with `include_bytes!`) supplies glyph advances via `ttf-parser`, and lines are broken greedily at spaces (hard-breaking words that don't fit). If the text still doesn't fit the shape, the font steps down to a minimum size, then truncates with an ellipsis. This handles Latin text well; complex scripts are out of scope for v1.

**Text as paths.** Exported SVGs convert text to paths (via `usvg`), so they look identical wherever they're opened and Typeset needs no fonts. SVGs for the UI's own use keep real text.

**Theme.** Exports use a fixed light print theme that maps the palette names (5.2) to concrete colours. `?theme=dark` exists for screenshots and slides.

**Connector routing must match the UI.** The UI draws connectors live in TypeScript and the renderer draws them in Rust. To stop them drifting, routing is specified exactly (anchor choice for `auto`, elbow midpoint rule, curve control points, label position) and both implementations run the same JSON fixture file (`fixtures/routing.json`: inputs and expected path points) in their tests.

**Caching.** Rendering a frame takes milliseconds, so there's no stored render cache. An in-memory LRU (about 64 MB) is keyed by a hash of the exact inputs (members' versions and geometry, connectors, theme, scale, format), so repeated requests are free and stale images are impossible.

### 8.2 Image endpoints

| Method | Route | Purpose |
|---|---|---|
| GET | `/api/items/{id}/render.svg` | Frame, schema card or diagram card as SVG. Query: `theme`, `title=0`, `text=paths\|text` |
| GET | `/api/items/{id}/render.png` | Same as PNG. Query adds `scale` (1–4, default 2) |
| POST | `/api/boards/{id}/render` | Render a selection: `{ item_ids, format, scale?, theme? }` |
| POST | `/api/items/{id}/image-url` | Mint a signed image URL (`format`, `scale`, `theme`; default lifetime 7 days, max 90) |

Signed image URLs use the same HMAC scheme as Typeset URLs (3.3) and work without a token, so Design, Typeset or an email can fetch them. They look like `{public_origin}/i/{item_id}.png?scale=2&exp=…&sig=…`. A signed URL renders the item as it is when fetched, not when signed; it's a link to the drawing, not a snapshot. A tampered or unsigned link gets 403, an expired one 410, and one whose item has been deleted 404.

### 8.3 Typeset: entry points

Typeset turns a board into a PDF. Board's document contains its own drawings as inline SVG, so Typeset never has to fetch images from Board or draw anything itself.

**Pull (recommended default).** Typeset fetches the document from Board using a signed URL (section 3.3):

```
POST typeset/render
{ "source": { "url": "https://board.example.com/api/boards/{id}/typeset?items=a,b,c&page_breaks=1&exp=…&sig=…" } }
```

**Several sources: the client proposal.** Typeset also accepts a list of sources and joins them into one PDF:

```
POST typeset/render
{ "sources": [
    { "url": "https://board.example.com/api/boards/{id}/typeset?…&sig=…" },
    { "url": "https://design.kritano.com/api/projects/{id}/typeset?…&sig=…" } ],
  "page_break_between_sources": true }
```

- Each source returns an ordinary `TypesetDocument`. `template` and `meta` come from the first source; later sources contribute only their blocks.
- Board's document carries the planning (overview notes, features, sitemap, flows, data model). Design's document carries the designs as `image` blocks (screenshots Design takes of its own HTML frames). Board never calls Design and Design never calls Board for this; Typeset is the assembler.
- Claude builds the request from `get_typeset_url` (Board's MCP) and Design's equivalent, then calls Typeset: "make the proposal PDF for Acme" is one tool call after two URL lookups.

**Push.** Board builds the document and POSTs it to Typeset (`POST /api/boards/{id}/export/typeset`). Useful from the UI's Export button, and when Typeset can't reach Board.

### 8.4 Document contract (proposed)

I don't have Typeset's current input format, so this is a proposal; `typeset_doc.rs` is its definition in code. It lives in a small `typeset-doc` crate owned by Typeset (section 2), which Design can depend on too.

```json
{
  "contract_version": 1,
  "template": "planning-doc",
  "meta": { "title": "", "subtitle": "", "client": "", "author": "", "date": "2026-11-05", "version": "" },
  "blocks": [ ]
}
```

| Block | Fields | Used for |
|---|---|---|
| `heading` | `level`, `text` | Frame and card titles (h2), table names (h3) |
| `markdown` | `text`, `heading_offset?` | Notes, feature summaries, frame descriptions, table comments |
| `table` | `columns`, `rows`, `caption?`, `column_widths?` | Schema tables, indexes |
| `tree` | `nodes` (`label`, `note?`, `children?`), `caption?` | Sitemaps |
| `checklist` | `groups` (`title?`, `items` of `text`, `done`) | Checklists, acceptance criteria |
| `svg` | `svg`, `alt?`, `caption?`, `width?` | Flows and drawings (frames), ER diagrams, Mermaid diagram cards |
| `image` | `src`, `alt`, `caption?`, `width?` | Raster images: Design's screenshots. `src` is an `https` URL on an allow-listed host, or a `data:` URL |
| `code` | `lang?`, `text`, `caption?` | Diagram source fallback |
| `callout` | `variant`, `text` | Feature status |
| `page_break` | none | Between sections (optional) |

- `width` is `full` (default), `half`, or a fraction of the text width (`0.6`). Images and SVGs keep their aspect ratio; one taller than the page is scaled to fit the page.
- `alt` is required on `image` and recommended on `svg`, so the PDF can be tagged for accessibility.

**ER cardinality (used by the renderer).** For a foreign key, the child end is "many" unless the FK column is unique (one-to-one); the parent end is optional when the FK column is nullable. Referenced tables outside a schema card's view are drawn as stubs (name only).

Evolution rules:

- `contract_version` is bumped on any breaking change. Typeset rejects versions it doesn't know with a clear error.
- Adding a block type is not breaking: Typeset deserialises unknown block types into an `Unknown` variant (`#[serde(other)]`), skips them, and logs a warning, so a newer Board or Design doesn't break an older Typeset.

### 8.5 Changes needed in Typeset

In order of value:

1. **`source` and `sources` input.** Accept `{ "source": { "url" } }` or `{ "sources": [ … ] }` as an alternative to an inline document: fetch each with a short timeout and a size cap, join as in 8.3, then continue as normal. Only fetch from allow-listed hosts (your Board and Design domains) to avoid SSRF. No token handling needed; the URLs are signed.
2. **`markdown` block.** Parse with `pulldown-cmark` and map to your existing primitives; apply `heading_offset`.
3. **`svg` block.** Scale to `width`, keep aspect ratio. **Sanitise** (strip scripts, `foreignObject`, external references). Board's exported SVGs are generated and already safe, but Typeset's sanitising is the authoritative one.
4. **`image` block.** PNG and JPEG; fetch `https` sources from allow-listed hosts only, with a per-image size cap (10 MB) and a per-document cap (100 MB).
5. **`table` block.** Relative column widths, repeated header row across pages, no row splitting.
6. **`tree` and `checklist` blocks.** Nested list with indentation first, polish later.
7. **`planning-doc` template.** Cover from `meta`, running header with client and title, page numbers. A `proposal` template (same, with a more client-facing cover) when you need it.
8. **`contract_version` check and `Unknown` block handling.**
9. Raise the request body limit for large documents.

Compared with v0.5, Typeset no longer needs `canvas` or `er` blocks: Board draws those itself.

### 8.6 Mapper

See `export.rs`. The mapper takes the board's items, connectors, tables and sitemap pages, plus a render function, so it can resolve ids and embed drawings. Summary:

- frame → `heading` (frame title), optional `markdown` (description), `svg` (the frame rendered by Board, text as paths, without its title since the heading carries it), then each card inside the frame in reading order as below
- note → `heading` + `markdown` (heading offset 1)
- feature → `heading`, `callout` (status), `markdown` summary, stories as a `markdown` list, acceptance as a `checklist`, then a `markdown` line listing the tables and pages it touches
- schema → `heading`, `svg` (ER diagram rendered by Board, or an override in push mode), then per table in the view: `heading` (h3), optional comment `markdown` (heading offset 3), `table` (Column / Type / Null / Default / Key / Notes), optional indexes `table`. The Type column shows the logical type with its extras (`decimal(10,2)`, `enum(pending, paid, shipped)`, `string(120)`), or `native_type` when set
- sitemap → `heading` + `tree`
- checklist → `heading` + `checklist`
- diagram → `heading` + `svg` (cached browser SVG), or `code` fallback if no fresh SVG

The mapper stays pure: it receives rendering as a function argument (`&dyn Fn(&Item) -> Option<String>`), so tests can pass a stub. "Today" is passed in, in the Europe/London time zone. `svg_overrides` go through the same SVG checks as `PUT /svg` before they reach the mapper.

---

## 9. Design integration

### 9.1 Roles

Design (design.kritano.com) is a separate app. It reads a board and turns it into designs:

| Board | Becomes in Design |
|---|---|
| Sitemap pages | One frame per page (desktop and mobile), named and ordered the same way |
| Schema tables | Generated components: a form, a list or table view, and a detail view per table |
| Feature cards | What to design for a feature: its pages, tables and acceptance criteria |
| Flows | How pages lead to each other (from shapes linked to pages), and the steps a feature has to support |
| Rendered drawings | Flow and ER images embedded in Design's own frames, via signed image URLs |
| Notes | Context Claude reads before designing |

Board never calls Design and doesn't depend on it. Board's one obligation is a stable, documented, versioned read contract: the **board model**.

### 9.2 The board model

`GET /api/boards/{id}/model` (also the MCP tool `get_board_model`, and the file `board pull` writes):

```json
{
  "contract": "board-model",
  "contract_version": 1,
  "board": { "id": "…", "title": "Acme Rebuild", "description": "…", "stack": "Laravel + MySQL" },
  "schema_version": 42,
  "tables": [
    { "id": "tbl_…", "name": "orders", "comment": "Customer orders", "version": 7,
      "columns": [
        { "id": "col_…", "name": "status", "type": "enum", "values": ["pending", "paid", "shipped"],
          "default": "pending", "nullable": false, "label": "Status", "hash": "3f9a1c2e" }
      ],
      "indexes": [ { "column_ids": ["col_…"], "unique": false } ] }
  ],
  "pages": [
    { "id": "pg_…", "title": "Home", "slug": "/", "purpose": "…", "children": [ ] }
  ],
  "features": [
    { "id": "…", "title": "Online booking", "status": "planned", "summary": "…",
      "stories": [ ], "acceptance": [ { "text": "…", "done": false } ],
      "table_ids": [ ], "page_ids": [ ] }
  ],
  "flows": [
    { "id": "…", "title": "Checkout flow",
      "nodes": [
        { "id": "…", "kind": "shape", "shape": "rect", "text": "Log in", "page_id": "pg_…" },
        { "id": "…", "kind": "card", "card_type": "schema", "title": "orders" }
      ],
      "edges": [ { "from": "…", "to": "…", "label": "no" } ] }
  ],
  "notes": [ { "id": "…", "title": "Brand and tone", "markdown": "…" } ],
  "warnings": [ ]
}
```

- `tables` are all live tables, in name order. Column defaults like `label` are filled in, so Design never has to apply Board's rules itself.
- `pages` come from the board's sitemap card. With more than one sitemap card, their pages are concatenated in reading order and `warnings` says so.
- `flows` are frames that contain at least one connector: their members as nodes (no coordinates: layout is Board's concern, not Design's) and the connectors between members as edges. Stickies and free text are left out.
- `features`, `flows` and `notes` are in reading order.
- `hash` on each column is a short hash of the column's definition (everything except `id`), so a change to a column is detectable without diffing.
- The response has an `ETag`; Design polls with `If-None-Match` and gets 304 when nothing has changed. Design can also subscribe to the board's SSE stream and refetch on relevant events.

### 9.3 Field provenance

Each field Design generates stores where it came from: `{ board_id, table_id, column_id, hash }`. On refresh Design can tell:

- **Unchanged:** same `column_id`, same `hash`.
- **Changed:** same `column_id`, different `hash` (type, nullability, enum values, label…). Flag the field as out of date.
- **Renamed:** same `column_id`, different `name`. Update the label, keep the field.
- **Removed:** `column_id` no longer present. Flag the field as orphaned.
- **New:** a column with no generated field. Offer to add it.

Pages work the same way with `page_id`. This only works because ids survive edits (5.4), which is why Board is strict about it.

### 9.4 Getting the model into other places

- **Design (live):** calls `/model` from its server with a `read` token. No CORS.
- **Images for Design:** Design embeds Board's flows and ER diagrams with signed image URLs (8.2), so they stay current without copying.
- **Into the client PDF:** Design exposes its own Typeset document (its screens as `image` blocks). Typeset joins it with Board's document (8.3). That's a convention for Design's spec; Board doesn't need to know about it.
- **Claude Code in another project:** uses Board's MCP server and `get_board_model`, so a session working on Design or a client build can read the plan directly.
- **A repo (committed):** `board pull <board-id> --into <dir>` writes `board-model.json` (pretty-printed, stable key order) so changes show up as diffs and builds don't depend on Board being up. Same JSON as the endpoint.
- **Types:** if Design is Rust, it depends on the `board-model` crate via a git tag. Otherwise it generates types from `/api/model.schema.json`.

Evolution follows the same rules as `typeset-doc`: additive changes don't bump `contract_version`, breaking ones do, and clients ignore fields they don't know.

### 9.5 The boundary

Both apps are infinite canvases, so keep the line sharp: **Board holds boxes and arrows for thinking; Design holds frames and pixels.** A flowchart, a user journey or a brainstorm belongs in Board. A mockup or wireframe belongs in Design. Board's shapes are a fixed flowchart set with palette colours, not a drawing toolkit, and Board stores no layouts, brand data or component choices (the type-to-input table in 5.4 is guidance for Design, not something Board stores). That keeps you from building the same canvas twice and wanting to merge them later.

---

## 10. Frontend

### 10.1 Canvas

- CSS-transformed container for pan and zoom. Cards, frames, stickies and text are absolutely positioned `div`s; shapes are small inline SVGs (so diamonds, cylinders and parallelograms draw cleanly) with an HTML text layer on top; connectors are one SVG layer beneath. No canvas library at this scale.
- Items outside the viewport (plus a margin) aren't rendered once a board passes about 500 items.
- Frames draw beneath their members; their title bar is the drag handle.
- Zoom levels from 10% to 400%; below about 40%, card bodies collapse to their title so the board stays readable.

### 10.2 Tools and interactions

| Tool | Key | Behaviour |
|---|---|---|
| Select | `V` | Click to select, shift-click to add, drag on empty space for a marquee |
| Hand | `H`, or hold `Space` | Pan |
| Shape | `R` (rect), `D` (diamond), `O` (ellipse) | Click to place at default size or drag to size; the toolbar has the full shape set |
| Sticky | `S` | |
| Text | `T` | |
| Frame | `F` | Drag to draw; or select items and press `F` to frame them |
| Connector | `C` | Drag from one item to another |
| Card | `K` | Opens a picker for the card type |

- **Quick flow:** a selected shape shows four `+` handles. Dragging from one draws a connector; dropping on empty space creates a new shape of the same type there, already connected, with the text editor open. Clicking a handle does the same at the default spacing. `Tab` on a selected shape creates and connects the next one below it. This is what makes drawing a flowchart fast.
- **Editing:** double-click (or `Enter`) to edit an item's text or a connector's label; `Esc` to finish.
- **Deleting:** `Delete` sends the selection to the trash; on a frame it releases the contents, and `Shift+Delete` takes the contents too.
- **Connectors:** hovering an item shows its four anchors; dropping on an anchor pins it, dropping on the item body uses `auto`. Connectors re-route live as items move.
- **Arrange:** snap to a 10-unit grid (hold `Alt` to disable), alignment guides against nearby items, align and distribute for a multi-selection, bring forward/back (`]` / `[`), lock (`Cmd+L`).
- **Clipboard:** copy, cut, paste and duplicate (`Cmd+D`) for any selection, including the connectors between selected items. Pasting between boards works for kinds that can move (section 6).
- **Navigation:** zoom with scroll or pinch, `Shift+1` zoom to fit, `Shift+2` zoom to selection, Cmd+K to search items and tables and jump to the result.
- **Style bar:** a small floating bar over the selection: colour, shape type (switch rect ↔ diamond without losing text or connectors), connector line, arrowheads and routing.
- **Export:** a frame's menu (and the selection's) has Export as PNG, Export as SVG and Copy image link. Schema and diagram cards have the same.

### 10.3 Card editors

- note: textarea with markdown preview toggle.
- feature: status picker, summary (markdown), stories list, acceptance checklist, and pickers for tables and pages. The card face shows status plus chips for its tables and pages; clicking a chip pans to the schema or sitemap card that shows it.
- schema: the card shows its tables, each with a column grid editor (logical type dropdown, extras shown per type, FK picker across the whole board schema). "Add table" creates a new table; "Show existing table" adds one already on the board. ER diagram rendered below, with tables outside the view drawn as stubs.
- sitemap: nested list with drag to reorder.
- checklist: list with checkboxes and group labels.
- diagram: source textarea with live Mermaid preview, and "Convert to shapes" for flowcharts.

Shapes have a small "link to page" action that opens a sitemap page picker; linked shapes show the page's slug underneath their text.

**Schema panel:** a side panel listing every table on the board and which schema cards show it, including tables shown on none. Editing a table anywhere updates every card that shows it (via `table.updated`).

**Board settings:** title, description, `stack`.

### 10.4 Saving, undo and live updates

- **Content saves:** debounced PATCH carrying `version`. Saves for an item or table are sent **one at a time**: the next save waits for the previous response and uses the `version` it returned, so a tab never conflicts with itself. On a real 409 show "changed elsewhere" with a reload action.
- **Geometry saves:** sent when a drag or resize ends (not during), as one batch for a multi-selection.
- **Undo/redo (`Cmd+Z` / `Cmd+Shift+Z`):** a local per-tab stack of the user's own operations (create, edit, move, resize, restyle, connect, delete), each with its inverse. Undoing sends the inverse as a batch. The stack holds the last 100 operations and is cleared on reload. Server revisions are separate: they're for history and for undoing Claude.
- **SVG cache:** after a diagram card renders, re-render off-screen with the fixed light export theme and `PUT /svg` in the background. Ignore a 409 (the item has moved on; the next render will catch up).
- **Live updates:** every request sends `X-Client-Id` (random per tab). SSE merges incoming records, skips events with this tab's `origin`, applies `batch` events in one frame, and reloads the board on `resync` or reconnect. An item being edited or dragged locally is never overwritten, only flagged. Items Claude has just created get a brief highlight so you can see what changed.
- **Trash:** a per-board trash panel lists deleted items, connectors and tables with who deleted them and an Undelete button.
- **Mobile:** read-only pan and zoom for v1.

---

## 11. Security

- UI: session cookie (`HttpOnly`, `Secure`, `SameSite=Strict`), one-time login links from the CLI, `Origin` check on writes.
- Machine clients: scoped, hashed bearer tokens (section 3.2); constant-time compare; actor taken from the token. Design gets a `read` token only.
- Typeset pull: HMAC-signed, expiring, single-board URLs; no long-lived secret leaves Board.
- Localhost bind plus TLS at the proxy. No CORS.
- Content size caps, per-route body limits and an item cap per board; reject unknown kinds, unknown fields and malformed shapes at the service layer.
- Shape, sticky and text content is plain text, rendered as text nodes, never as HTML. Markdown rendered without raw HTML; Mermaid with `securityLevel: "strict"`.
- SVG upload and `svg_overrides`: size cap, reject `<script`, `foreignObject` and external hrefs; Typeset sanitises again.
- The Mermaid parser and layout run in Rust on untrusted input: cap input at 64 KB and 500 nodes, and keep the layout's work bounded (no unbounded crossing-minimisation loops).
- Rendering is bounded: frames up to 20,000 × 20,000 units, PNGs up to 40 megapixels, a 5-second render timeout. `resvg` only ever sees SVG that Board generated or that passed the upload checks, with external resources and image loading disabled.
- Typeset `source`/`sources` and `image` fetching restricted to allow-listed hosts (Board and Design).
- `template` checked against an allow-list.
- Rate limiting is optional for v1 (single user behind localhost). If added, per token via `tower_governor` or similar.

---

## 12. Backup and portability

- Nightly `sqlite3 board.db ".backup …"` via cron, copied off-droplet, 14 days kept.
- Daily trash purge (items deleted more than 30 days ago) and expired session cleanup, run by the binary itself.
- `GET /api/boards/{id}/export/folder` produces a zip: `board.json` (every item with geometry and style, and every connector), `board-model.json` (section 9.2), one `.mmd` file (Mermaid source) and one `.png` and `.svg` per flow frame, an ER `.png`/`.svg` per schema card, plus one file per card (`.md` for notes, `.json` otherwise), so your data stays readable without the app.

---

## 13. Milestones

1. **M1: Core.** Migrations (items, connectors, soft delete, revisions, FTS), service layer with `BEGIN IMMEDIATE` writes, revision coalescing and batches, session cookie and bearer tokens, REST API for boards, items and connectors, canvas with pan/zoom, select/drag/resize, note and checklist cards, trash. Deployed to the droplet with nightly backups, so it can be used for real from here on.
2. **M2: Drawing.** Shapes, stickies, text, frames (with membership), connectors with anchors and routing, toolbar and shortcuts, quick flow, multi-select, clipboard, align/snap, style bar, local undo/redo. After this milestone Board is usable as a planning canvas on its own.
3. **M3: Claude.** `rmcp` tools over the service layer, `add_items` with auto-layout, the Mermaid flowchart parser (`draw_flow`, `get_flow`, Convert to shapes), auto-placement, SSE with `batch` events and highlights, OpenAPI.
4. **M4: Rendering.** Board's renderer for frames and selections (text wrapping, shared routing fixtures, print theme, text as paths), PNG via `resvg`, image endpoints and signed image URLs, Export menu in the UI, MCP `get_item_image`. After this milestone drawings can leave Board as files.
5. **M5: Board schema and sitemap.** `schema_tables` with logical types, stable ids, input by name, semantic validation, schema cards as views, schema panel, ER source endpoint and ER rendering, sitemap editor with node ids, shapes linked to pages, revisions UI. MCP schema tools.
6. **M6: Features and the board model.** Feature cards, `/model` endpoint with ETag (tables, pages, features, flows, notes), `board-model` crate and JSON Schema, `board pull`, `get_board_model`. **Design's Board import (its v2) depends on this milestone.**
7. **M7: Typeset.** *Blocked on open question 1.* Mapper wired in (`/typeset` endpoint, signed URLs, push) with rendered SVGs, feature mapping, SVG cache for diagram cards, Typeset changes from 8.5 including `sources` and `image`. The combined client proposal PDF needs Design's Typeset document as well.
8. **M8: Polish.** Diagram cards, search UI, folder export, mobile view.

Later ideas: clients, with boards attached to them (open question 3); schema exporters per stack (Laravel migrations and models, SQL DDL per dialect, Prisma), driven by logical types, `native_type` and `boards.stack`; import a schema from an existing database; board templates (brochure, e-commerce, booking) with starter flows and frames; connector waypoints; a minimap.

---

## 14. Changes needed in the delivered code

`export.rs` and `typeset_doc.rs` were written against v0.2. To match v0.5:

**New `board-model` crate**

- Move the schema and sitemap types out of `export.rs` into it: `SchemaTable` (with `id`, `version`, `definition`), `SchemaColumn` (with `id`, logical `ColumnType` enum, `length`, `precision`, `scale`, `values`, `generated`, `label`, `native_type`, and `references` as `{ table_id, column_id }`), `SchemaIndex` (`column_ids`), `SitemapNode` (with `id`), `FeatureContent`, `Flow` (`nodes`, `edges`), and the `BoardModel` response type. Derive `schemars::JsonSchema`.

**`export.rs`**

- Replace `CardContent` with `ItemContent` (kinds from 5.1, with `Card(CardContent)` as one variant), plus `ItemStyle` and `ConnectorStyle`. `Card` becomes `Item` with geometry and `frame_id`; add `Connector`.
- `CardContent`: `Schema` becomes `{ table_ids }`; add `Feature`; sitemap nodes carry `id`.
- Add `#[serde(deny_unknown_fields)]` to every content and style struct.
- Make `from_parts` go through `serde_path_to_error`, and add a `validate` step with the semantic checks from 5.5 that takes the board's tables and pages, returning a `ContentError { path, message }` plus warnings.
- `to_typeset_document` takes items, connectors, tables and pages. `ExportOptions.card_ids` becomes `item_ids` with `#[serde(default)]`; the empty-means-all-in-reading-order rule lives in the service layer, which passes the resolved list in. Drop duplicate ids in the mapper.
- The mapper takes a render function (8.6). Add the frame mapper (`svg` from the renderer, then member cards) and the feature mapper.
- Schema mapper: emit the rendered ER `svg` instead of the cached browser SVG; use an override SVG only if one is supplied. Format the Type column from the logical type and extras, or `native_type`.
- Table comment markdown: `heading_offset: 3`.
- `schema_to_mermaid`: works from the board's tables; use the logical type name as the Mermaid type; emit `PK, FK` when a column is both; use `||--o|` for unique FKs and `|o--o{` for nullable FKs; draw referenced tables outside the view as empty entities.
- New tests: unknown field rejected, error path present, style field on the wrong kind rejected, duplicate ids, id kept on rename-by-name, frame export uses the render function and includes member cards in reading order, feature mapping, type formatting.

**`typeset_doc.rs`**

- Add `contract_version: u32` to `TypesetDocument`.
- Add the `Image` block (`src`, `alt`, `caption`, `width`) and `alt` / `width` on `Svg`. Remove the reserved v0.5 ideas of `Canvas` and `Er` blocks; Board renders those.
- Add the `sources` request shape (`RenderRequest` with `source` or `sources`) so Board, Design and Typeset share it.
- Add `#[serde(other)] Unknown` to `Block`.
- Move into the `typeset-doc` crate in the Typeset repo.

**New modules**

- `flow/mermaid.rs`: the subset parser from 5.3, with errors naming the line, and the reverse (frame → Mermaid). Tests for every node shape, edge style, labels, chains, subgraphs and each unsupported construct; and a round-trip test.
- `flow/layout.rs`: the layered layout from 5.3. Tests for determinism, cycles, `TB` vs `LR`, and no overlaps.
- `render/`: SVG for frames, selections and ER diagrams; text wrapping; routing. Snapshot tests (`insta`) on the SVG output, the shared `fixtures/routing.json` cases, wrapping edge cases (long words, overflow to ellipsis), and a PNG smoke test.
- `model.rs`: pure `board_model(board, items, connectors, tables) -> BoardModel`, with tests for ordering, label defaults, column hashes, flow extraction, multiple-sitemap warning and the ETag input.

---

## 15. Open questions

1. What does Typeset accept today? A real payload would let me replace the proposed contract with the actual one. **Blocks M7.**
2. Pull mode requires Typeset to reach Board over HTTP. Are they on the same droplet or network, or does one sit behind auth that complicates this?
3. ~~Per-client folders or a flat list?~~ **Decided:** a flat list of boards with search for v1. Later, boards can be attached to a client: a `clients` table and a nullable `boards.client_id`, added by an additive migration, with a client filter on the board list.
4. ~~Frontend framework?~~ **Decided:** Preact with signals (see the stack table in section 2).
5. ~~Which MCP clients?~~ **Decided:** Claude Code only for now, with a static bearer token. Using Board from claude.ai or Claude Desktop as a remote connector would mean adding OAuth to `/mcp`; revisit when needed.
6. ~~Fonts?~~ **Decided:** Inter for every export in v1. A brand font per board can be added later; every export path would need to honour it.
7. One schema per board assumes one board per job. If you want several independent schemas on one board (for example, comparing two designs of the same data), tables would gain a `schema_id` and schema cards would belong to one schema. Easy now, harder after M5, so decide before then.
8. What is Design built in? If Rust, it uses the `board-model` crate; otherwise it generates types from the JSON Schema.
9. ~~Shape set and nested frames?~~ **Decided:** the shape set as specced, and no nested frames in v1. Adding nesting later means one migration (rebuild `items` without the `frame_id` CHECK, since SQLite can't drop a constraint in place) and changing membership to the innermost containing frame.

---

## 16. Changes

### v0.6

- **Decisions:** frontend is Preact with signals; boards are a flat list for v1, with clients to come later.
- **Rendering:** Board draws its own SVG and PNG images of frames, selections and ER diagrams (bundled font, text wrapping, text as paths, `resvg`), with image endpoints, signed image URLs, an Export menu and MCP `get_item_image`.
- **Typeset:** `canvas` and `er` blocks dropped (Board renders those and sends SVG); new `image` block; `sources` input joins Board's and Design's documents into one client proposal PDF.
- **Design:** embeds Board's drawings via signed image URLs; contributes its screens to the PDF through its own Typeset document.
- **Milestones:** new M4 Rendering; schema, model and Typeset move to M5–M7.

### v0.5

- **Purpose:** Board is a full planning canvas, not just a schema tool: flowcharts, user journeys and brainstorms alongside structured cards.
- **Item model:** cards, shapes, stickies, text and frames are all `items` with a `kind`; `links` became `connectors` that join any two items, with anchors, labels and styles. Fixed colour palette by name.
- **Frames:** server-computed membership, frames move their contents, optional delete-with-contents, no nesting in v1. Frames are the unit of export for drawings.
- **Claude drawing:** `add_items` with `ref`s and auto-layout, `draw_flow` from a Mermaid flowchart subset parsed in Rust, `get_flow` to read a frame back as Mermaid, "Convert to shapes" on diagram cards; deterministic layered layout.
- **API:** `/api/items` replaces `/api/cards`; batch endpoint (one transaction, `ref`s); flow endpoints; cross-board moves keep connectors between moved items; `batch` SSE events.
- **Typeset:** new `canvas` block drawn by Typeset from Board's geometry; only Mermaid diagram cards still need a browser SVG.
- **Design:** board model gains `flows`; shapes can link to sitemap pages so Design can see how pages connect.
- **Frontend:** toolbar and shortcuts, quick-flow handles, multi-select, clipboard, snap and guides, style bar, local undo/redo, viewport culling.
- **Milestones:** drawing is M2 (before MCP) so the canvas is usable early; schema and Design work shifts to M4 and M5; Typeset to M6.

### v0.4

- **Purpose:** Board framed as the planning source of truth for a project, with Design (design.kritano.com) as a read-only consumer.
- **Board schema:** tables moved out of card content into `schema_tables`, one schema per board; schema cards are views; foreign keys validated across the board; table revisions and trash; `schema_version` on boards.
- **Logical types:** a fixed, stack-neutral column type set with type-specific extras, plus `generated`, `label` and `native_type`; `boards.stack` as a free-text hint.
- **Stable ids:** `tbl_`, `col_`, `pg_` ids that survive renames; input by name or slug, resolved server-side.
- **Feature cards:** status, summary, stories, acceptance criteria, linked tables and pages.
- **Board model contract:** `/api/boards/{id}/model` with ETag, column hashes for field provenance, JSON Schema, `board-model` crate owned by Board, `board pull`, MCP `get_board_model`.

### v0.3

- **Auth:** UI moved to a session cookie (`EventSource` can't send bearer headers); tokens gained `actor`; the `export` scope was replaced by signed, expiring, single-board URLs.
- **Data model:** soft delete with a trash and 30-day purge; `updated_by`, `content_updated_at`; FTS5 search; RFC 3339 UTC timestamps; `rendered_svg` excluded from board reads.
- **Writes:** `BEGIN IMMEDIATE` for all write transactions; atomic version check.
- **Revisions:** coalesced per actor within 5 minutes; restore is a normal versioned update.
- **Validation:** `deny_unknown_fields`, path-qualified errors, semantic checks.
- **REST, SSE, MCP:** trash/undelete, search, signed-URL minting, default export order, per-route body limits, `origin` for self-echo suppression, reload on lag or reconnect, tools aligned with the service layer, defined auto-placement.
- **Typeset:** `er` block replaces browser-rendered ER SVGs; `contract_version` and `Unknown` blocks; `typeset-doc` owned by Typeset; export SVGs in a fixed light theme.
