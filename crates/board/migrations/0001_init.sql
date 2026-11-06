-- Board schema (spec section 4.1). Created in full now, including tables
-- the app starts using in later phases, so there's one initial migration.
-- All timestamps are RFC 3339 UTC with millisecond precision
-- (2026-11-06T09:30:00.000Z): fixed width, so they compare as strings.

CREATE TABLE boards (
  id             TEXT PRIMARY KEY,
  title          TEXT NOT NULL,
  description    TEXT,
  stack          TEXT,
  schema_version INTEGER NOT NULL DEFAULT 0,
  archived       INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL
);

CREATE TABLE items (
  id                 TEXT PRIMARY KEY,
  board_id           TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  kind               TEXT NOT NULL CHECK (kind IN ('card','shape','sticky','text','frame')),
  card_type          TEXT CHECK (card_type IN ('note','feature','schema','sitemap','checklist','diagram')),
  frame_id           TEXT REFERENCES items(id) ON DELETE SET NULL,
  title              TEXT,
  content            TEXT NOT NULL,
  style              TEXT,
  x                  REAL NOT NULL DEFAULT 0,
  y                  REAL NOT NULL DEFAULT 0,
  w                  REAL NOT NULL,
  h                  REAL NOT NULL,
  z                  INTEGER NOT NULL DEFAULT 0,
  locked             INTEGER NOT NULL DEFAULT 0 CHECK (locked IN (0, 1)),
  version            INTEGER NOT NULL DEFAULT 1,
  rendered_svg       TEXT,
  svg_for_version    INTEGER,
  created_by         TEXT NOT NULL CHECK (created_by IN ('user','claude','api')),
  updated_by         TEXT NOT NULL CHECK (updated_by IN ('user','claude','api')),
  content_updated_at TEXT NOT NULL,
  deleted_at         TEXT,
  deleted_by         TEXT CHECK (deleted_by IN ('user','claude','api')),
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL,
  CHECK ((kind = 'card') = (card_type IS NOT NULL)),
  CHECK (kind <> 'frame' OR frame_id IS NULL)
);
CREATE INDEX idx_items_board ON items(board_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_items_frame ON items(frame_id) WHERE frame_id IS NOT NULL;
CREATE INDEX idx_items_trash ON items(board_id, deleted_at) WHERE deleted_at IS NOT NULL;

CREATE TABLE connectors (
  id          TEXT PRIMARY KEY,
  board_id    TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  from_item   TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  to_item     TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  from_anchor TEXT NOT NULL DEFAULT 'auto' CHECK (from_anchor IN ('auto','top','right','bottom','left')),
  to_anchor   TEXT NOT NULL DEFAULT 'auto' CHECK (to_anchor IN ('auto','top','right','bottom','left')),
  label       TEXT,
  style       TEXT,
  created_by  TEXT NOT NULL CHECK (created_by IN ('user','claude','api')),
  deleted_at  TEXT,
  deleted_by  TEXT CHECK (deleted_by IN ('user','claude','api')),
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  CHECK (from_item <> to_item)
);
CREATE INDEX idx_connectors_board ON connectors(board_id) WHERE deleted_at IS NULL;

CREATE TABLE schema_tables (
  id                 TEXT PRIMARY KEY,
  board_id           TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name               TEXT NOT NULL,
  definition         TEXT NOT NULL,
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
  changed_by TEXT NOT NULL CHECK (changed_by IN ('user','claude','api')),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX idx_item_revisions ON item_revisions(item_id, id);

CREATE TABLE table_revisions (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  table_id   TEXT NOT NULL REFERENCES schema_tables(id) ON DELETE CASCADE,
  name       TEXT NOT NULL,
  definition TEXT NOT NULL,
  version    INTEGER NOT NULL,
  changed_by TEXT NOT NULL CHECK (changed_by IN ('user','claude','api')),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX idx_table_revisions ON table_revisions(table_id, id);

-- Titles and text of live items and tables, kept in sync by the service layer.
CREATE VIRTUAL TABLE search_fts USING fts5(
  entity_kind UNINDEXED,
  entity_id UNINDEXED,
  board_id UNINDEXED,
  title,
  body,
  tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TABLE sessions (
  id_hash      TEXT PRIMARY KEY,
  created_at   TEXT NOT NULL,
  expires_at   TEXT NOT NULL,
  last_used_at TEXT
);

-- One-time login links printed by `board login` (spec section 3.1).
CREATE TABLE login_codes (
  code_hash  TEXT PRIMARY KEY,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  used_at    TEXT
);

CREATE TABLE api_tokens (
  id           TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  token_hash   TEXT NOT NULL UNIQUE,
  scope        TEXT NOT NULL CHECK (scope IN ('admin','read')),
  actor        TEXT NOT NULL CHECK (actor IN ('claude','api')),
  created_at   TEXT NOT NULL,
  last_used_at TEXT
);
