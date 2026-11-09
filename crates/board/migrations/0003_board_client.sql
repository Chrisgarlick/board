-- The client a board is for: a client's slug in Clients (clients.kritano.com).
-- PDFs use that client's brand; Design can read it from the board model.
ALTER TABLE boards ADD COLUMN client TEXT;
