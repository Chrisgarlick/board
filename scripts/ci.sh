#!/usr/bin/env bash
# The checks CI runs. Run locally before pushing: ./scripts/ci.sh
# GitHub Actions runs this same script, so local and remote can't drift.
set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

step "Frontend: install"
(cd web && npm ci --no-audit --no-fund --silent)

step "Frontend: typecheck"
(cd web && npm run -s typecheck)

step "Frontend: test"
(cd web && npm test --silent)

step "Frontend: build"
(cd web && npm run -s build)

step "Rust: format"
cargo fmt --all --check

step "Rust: clippy"
cargo clippy --workspace --all-targets -- -D warnings

step "Rust: test"
cargo test --workspace

printf '\n\033[1;32mAll checks passed.\033[0m\n'
