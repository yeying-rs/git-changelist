#!/usr/bin/env bash
# Install taglens from this checkout into ~/.cargo/bin (or $CARGO_INSTALL_ROOT/bin).
set -euo pipefail
cd "$(dirname "$0")/.."

cargo install --path . --locked --force

if ! command -v taglens >/dev/null 2>&1; then
  echo "warning: taglens installed but not on PATH; add ~/.cargo/bin to PATH" >&2
  exit 1
fi
echo "Installed: $(command -v taglens)"
taglens --version
