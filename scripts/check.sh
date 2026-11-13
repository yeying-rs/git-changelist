#!/usr/bin/env bash
# Quality gate: formatting, lints and tests. Run before every commit.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> cargo fmt --check"
cargo fmt --all -- --check

echo "==> cargo clippy (warnings are errors)"
cargo clippy --all-targets --locked -- -D warnings

echo "==> cargo test"
cargo test --locked

echo "All checks passed."
