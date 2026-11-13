#!/usr/bin/env bash
# Build an optimized binary and print where it is.
# Usage: scripts/build.sh [extra cargo build args, e.g. --target aarch64-apple-darwin]
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --locked "$@"

# Honour CARGO_TARGET_DIR / ~/.cargo/config.toml instead of assuming ./target.
target_dir=$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
target_triple=""
for ((i = 1; i <= $#; i++)); do
  if [[ "${!i}" == "--target" ]]; then
    next=$((i + 1))
    target_triple="${!next}"
  fi
done
binary="$target_dir/${target_triple:+$target_triple/}release/git-changelist"

if [[ ! -x "$binary" ]]; then
  echo "error: binary not found at $binary" >&2
  exit 1
fi
echo "Built: $binary ($(du -h "$binary" | cut -f1))"
"$binary" --version
