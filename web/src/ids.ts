// Ids for things the client creates inside content: sitemap pages and table
// columns. The server accepts ids sent with new entries as long as they have
// the right shape (a prefix and a ULID) and aren't used elsewhere; minting
// them here means an entry keeps one id from its first keystroke, even while
// saves are still in flight.

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/** A ULID: 48 bits of milliseconds, 80 random bits, Crockford base32. */
export function ulid(now = Date.now()): string {
  let time = "";
  let t = now;
  for (let i = 0; i < 10; i++) {
    time = CROCKFORD[t % 32] + time;
    t = Math.floor(t / 32);
  }
  const random = crypto.getRandomValues(new Uint8Array(16));
  let rest = "";
  for (let i = 0; i < 16; i++) rest += CROCKFORD[random[i]! % 32];
  return time + rest;
}

export const pageId = (): string => `pg_${ulid()}`;
export const columnId = (): string => `col_${ulid()}`;
