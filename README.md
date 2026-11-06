# Board

A planning canvas for client website projects: flowcharts, user journeys and
brainstorms alongside typed cards for features, database schemas, sitemaps and
checklists. One Rust binary and one SQLite file.

- Design: [`planning-board-tech-spec-rust.md`](planning-board-tech-spec-rust.md)
- Build order: [`plan.md`](plan.md)

## Layout

```
crates/board         the app (server, CLI, embedded frontend)
crates/board-model   Board's published read contract, for Design (phase 5+)
crates/typeset-doc   Typeset document contract; moves to the Typeset repo in phase 7
web/                 frontend source; web/dist is served by the app
```

## Development

Needs Rust (stable) and Node 22.

```sh
cd web && npm install && npm run build && cd ..
cargo run -- serve                 # http://localhost:7070, creates board.toml on first run
```

For frontend work, run `npm run watch` in `web/` alongside `cargo run -- serve`.
Debug builds read `web/dist` from disk, so a browser refresh picks up changes
without recompiling Rust. Release builds embed `web/dist` in the binary.

CI runs locally. The checks (GitHub Actions can run the same script, but only when triggered by hand):

```sh
./scripts/ci.sh
```

To run them automatically before every push:

```sh
git config core.hooksPath .githooks     # once per clone; skip a push with --no-verify
```

Browser tests (Playwright, kept out of the normal checks so they never download a browser):

```sh
(cd e2e && npm install && npx playwright install chromium)   # once
./scripts/e2e.sh                      # every suite
./scripts/e2e.sh e2e/schema.mjs       # just one
```

## Running it

```sh
cargo run -- serve                 # in one terminal
cargo run -- login                 # prints a one-time login link: open it
cargo run -- token create --name laptop --scope admin --actor api   # for scripts
```

`board --help` lists every command.

## Connecting Claude Code

Board is an MCP server at `/mcp`. Create a token that acts as Claude, then add it to Claude Code:

```sh
board token create --name claude-mcp --scope admin --actor claude     # prints brd_…
claude mcp add --transport http board https://board.example.com/mcp \
  --header "Authorization: Bearer brd_…"
```

(Locally, use `http://localhost:7070/mcp`.) Claude can then list and read boards,
draw flows from Mermaid, add and edit items and connectors, and restore
revisions. Everything it does is recorded as Claude, appears live in open tabs
with a brief highlight, and can be undone from an item's History or the Trash.

## Reading a plan from Design (or any project)

Every board has a **board model**: its tables (labels filled in, a hash per
column), sitemap pages, features, flows and notes, in one JSON document.

```sh
board token create --name design --scope read --actor api            # on the server; prints brd_…
curl -H "Authorization: Bearer brd_…" https://board.example.com/api/boards/<id>/model
```

The response has an `ETag`; send it back as `If-None-Match` and an unchanged
board answers `304`. The JSON Schema is at `/api/model.schema.json` (no login).

To commit the plan into a repo, so builds don't depend on Board being up:

```sh
board pull <board-id> --into . --from https://board.example.com --token brd_…   # or BOARD_URL / BOARD_TOKEN
```

That writes `board-model.json`, pretty-printed in a stable order, and leaves it
alone when nothing changed, so a column change shows up as a small diff. On the
server itself, leave out `--from` to read the local database.

Rust projects use the types directly:

```toml
board-model = { git = "ssh://git@github.com/Chrisgarlick/board.git", tag = "board-model-v0.1.0" }
```

(The repo is private, so Cargo fetches over SSH; if it can't find your key, set
`CARGO_NET_GIT_FETCH_WITH_CLI=true`.) Claude in another project can call the
MCP tool `get_board_model` instead.

Deploying to a server: see [`deploy/README.md`](deploy/README.md).
