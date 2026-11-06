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

`board --help` lists the CLI commands. `login`, `token` and `pull` are stubs
until phases 1 and 6.
