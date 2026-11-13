#!/usr/bin/env bash
# Install git-changelist into ~/.cargo/bin; pass --skill to also install skills/git-changelist/SKILL.md.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo install --path . --locked --force

if ! command -v git-changelist >/dev/null 2>&1; then
  echo "warning: git-changelist installed but not on PATH; add ~/.cargo/bin to PATH" >&2
  exit 1
fi
echo "Installed: $(command -v git-changelist)"
git-changelist --version

if [[ "${1:-}" == "--skill" ]]; then
  git-changelist skill
fi
