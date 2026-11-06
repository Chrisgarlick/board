#!/usr/bin/env bash
# Nightly backup of board.db (spec section 12).
#
# Takes a consistent copy with SQLite's online backup (safe while Board is
# running), compresses it, keeps 14 days locally, and copies it off the
# droplet if BACKUP_REMOTE is set (any rsync destination, e.g.
# "backup@nas.example.com:board/").
#
# Cron (as the board user):  15 3 * * *  /usr/local/bin/board-backup.sh
set -euo pipefail

DB=${BOARD_DB:-/var/lib/board/board.db}
DIR=${BACKUP_DIR:-/var/lib/board/backups}
KEEP_DAYS=${KEEP_DAYS:-14}
REMOTE=${BACKUP_REMOTE:-}

mkdir -p "$DIR"
stamp=$(date -u +%Y%m%dT%H%M%SZ)
out="$DIR/board-$stamp.db"

sqlite3 "$DB" ".backup '$out'"
# The copy inherits WAL mode; switch it to a plain single file (so no -wal or
# -shm files are left behind), then refuse to keep it if SQLite says it's damaged.
sqlite3 "$out" 'PRAGMA journal_mode=DELETE;' >/dev/null
rm -f "$out-wal" "$out-shm" # now unused: the copy is a single file
if [ "$(sqlite3 "$out" 'PRAGMA integrity_check;')" != "ok" ]; then
  echo "backup $out failed its integrity check" >&2
  rm -f "$out" "$out-wal" "$out-shm"
  exit 1
fi
gzip -9 "$out"

find "$DIR" -name 'board-*.db.gz' -mtime +"$KEEP_DAYS" -delete

if [ -n "$REMOTE" ]; then
  rsync -a --delete "$DIR/" "$REMOTE"
fi
echo "backed up to $out.gz"
