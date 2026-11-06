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
./scripts/e2e.sh
```

## Running it

```sh
cargo run -- serve                 # in one terminal
cargo run -- login                 # prints a one-time login link: open it
cargo run -- token create --name laptop --scope admin --actor api   # for scripts
```

`board --help` lists every command. `pull` arrives in phase 6.

Deploying to a server: see [`deploy/README.md`](deploy/README.md).
