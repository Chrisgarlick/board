-- Board-wide settings, one row per key. Values are JSON.
-- review_mode: when true (the default), the browser is a review layer and
-- cookie sessions can't change boards; tokens and MCP are unaffected.
CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

INSERT INTO settings (key, value) VALUES ('review_mode', 'true');
