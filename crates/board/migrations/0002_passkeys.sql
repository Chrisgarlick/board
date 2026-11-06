-- Passkeys (WebAuthn) for signing in to the UI. Single user: every passkey
-- is the owner's. Only ES256 (P-256) keys, which every passkey provider
-- supports.
CREATE TABLE passkeys (
  id           TEXT PRIMARY KEY,            -- the credential id, base64url
  name         TEXT NOT NULL,               -- e.g. "iPhone", for the list
  public_key   BLOB NOT NULL,               -- SEC1 uncompressed P-256 point (65 bytes)
  sign_count   INTEGER NOT NULL DEFAULT 0,  -- synced passkeys always report 0
  created_at   TEXT NOT NULL,
  last_used_at TEXT
);

-- Single-use challenges for registering and signing in, valid 5 minutes.
CREATE TABLE passkey_challenges (
  challenge  TEXT PRIMARY KEY,              -- base64url of 32 random bytes
  kind       TEXT NOT NULL CHECK (kind IN ('register', 'login')),
  expires_at TEXT NOT NULL
);
