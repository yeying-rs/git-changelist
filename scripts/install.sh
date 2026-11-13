#!/usr/bin/env bash
# Install git-changes into ~/.cargo/bin; pass --skill to also install skills/git-changes/SKILL.md.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo install --path . --locked --force

if ! command -v git-changes >/dev/null 2>&1; then
  echo "warning: git-changes installed but not on PATH; add ~/.cargo/bin to PATH" >&2
  exit 1
fi
echo "Installed: $(command -v git-changes)"
git-changes --version

if [[ "${1:-}" == "--skill" ]]; then
  git-changes skill
fi
