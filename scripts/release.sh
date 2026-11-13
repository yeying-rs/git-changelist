#!/usr/bin/env bash
# Cut a release: bump the version, run the quality gate, commit and create an annotated tag
# whose message is the change summary produced by git-changes itself.
# Usage: scripts/release.sh 0.2.0
# Nothing is pushed; the script prints the push command at the end.
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:-}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.]+)?$ ]]; then
  echo "usage: scripts/release.sh <version>   e.g. 0.2.0" >&2
  exit 1
fi
tag="v$version"

branch=$(git branch --show-current)
if [[ "$branch" != "main" ]]; then
  echo "error: releases are cut from main (current: $branch)" >&2
  exit 1
fi
if [[ -n "$(git status --porcelain)" ]]; then
  echo "error: working tree is not clean" >&2
  exit 1
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "error: tag $tag already exists" >&2
  exit 1
fi

# Only the [package] version, which is the first `version =` line in Cargo.toml.
sed -i.bak "1,/^version = /s/^version = \".*\"/version = \"$version\"/" Cargo.toml
rm -f Cargo.toml.bak
cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace >/dev/null

scripts/check.sh

previous=$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)
if [[ -n "$previous" ]]; then
  notes=$(cargo run --quiet --release --locked -- . --range "$previous" HEAD -f brief --oldest-first --max-commits 1000)
else
  notes=$(cargo run --quiet --release --locked -- . --unreleased -f brief --max-commits 1000)
fi
# The commits are summarised before the tag exists, so the heading says HEAD.
notes=$(printf '%s\n' "$notes" | sed "1s/HEAD/$tag/g")

git add Cargo.toml Cargo.lock
if ! git diff --cached --quiet; then
  git commit -q -m "chore(release): $tag

- 版本号更新为 $version"
fi
git tag -a "$tag" -m "$tag

$notes"

echo "Released $tag locally."
echo "Push with: git push origin main $tag"
