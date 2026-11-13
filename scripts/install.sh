#!/usr/bin/env bash
# Install taglens into ~/.cargo/bin; pass --skill to also install skills/taglens/SKILL.md.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo install --path . --locked --force

if ! command -v taglens >/dev/null 2>&1; then
  echo "warning: taglens installed but not on PATH; add ~/.cargo/bin to PATH" >&2
  exit 1
fi
echo "Installed: $(command -v taglens)"
taglens --version

if [[ "${1:-}" == "--skill" ]]; then
  taglens skill
fi
