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
  for dir in "${SKILL_DIR:-}" "$HOME/.gemini/skills" "$HOME/.claude/skills" "$HOME/.agents/skills"; do
    [[ -n "$dir" ]] || continue
    if [[ -d "$dir" || "$dir" == "${SKILL_DIR:-}" ]]; then
      mkdir -p "$dir/taglens"
      cp skills/taglens/SKILL.md "$dir/taglens/SKILL.md"
      echo "Installed skill: $dir/taglens/SKILL.md"
    fi
  done
fi
