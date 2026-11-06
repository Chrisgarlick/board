#!/usr/bin/env bash
# Browser tests: build Board, start it on a throwaway database, and drive the
# UI with Playwright. Screenshots land in e2e/screenshots/.
#
# Usage: scripts/e2e.sh [e2e/some.mjs ...]   (default: every e2e/*.mjs)
# First time: (cd e2e && npm install && npx playwright install chromium)
# If the installed browser doesn't match, set CHROMIUM_PATH to a Chromium binary.
set -euo pipefail
cd "$(dirname "$0")/.."

PORT=7070
if curl -s "localhost:$PORT/healthz" >/dev/null 2>&1; then
  echo "Something is already listening on port $PORT; stop it first." >&2
  exit 1
fi

(cd web && npm run -s build)
cargo build -q

TMP=$(mktemp -d)
trap 'kill "${SERVER:-}" 2>/dev/null || true; rm -rf "$TMP"' EXIT
printf 'database = "%s/board.db"\n' "$TMP" > "$TMP/board.toml"
BIN=./target/debug/board
CONFIG=(--config "$TMP/board.toml")

TOKEN=$("$BIN" "${CONFIG[@]}" token create --name e2e --scope admin --actor api 2>/dev/null)
# Start the binary itself in the background (not via a function or subshell),
# so $! is the server's own pid and the cleanup trap really stops it.
"$BIN" "${CONFIG[@]}" serve >"$TMP/serve.log" 2>&1 &
SERVER=$!
for _ in $(seq 1 50); do
  curl -s "localhost:$PORT/healthz" >/dev/null && break
  sleep 0.2
done
mkdir -p e2e/screenshots
status=0
tests=("$@")
[ ${#tests[@]} -eq 0 ] && tests=(e2e/*.mjs)
for test in "${tests[@]}"; do
  echo "== $test"
  # Each test logs in with its own one-time link.
  LINK=$("$BIN" "${CONFIG[@]}" login 2>/dev/null)
  node "$test" "$LINK" "$TOKEN" || status=1
done
exit $status
