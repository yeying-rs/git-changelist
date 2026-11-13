---
name: taglens
description: Extract verifiable Git/GitHub version facts (per-tag changelog, upgrade diffs across release and hotfix branches, earliest tag & cherry-pick backport lookup, commit-to-version mapping, and lockfile dependency diffs) via the `taglens` CLI. Use whenever investigating software version changes, upgrade risks, regressions between tags, whether a fix/PR shipped in a release or hotfix branch, or dependency changes between versions.
---

# taglens Skill

`taglens` is a deterministic, fact-only CLI (no internal LLM) that extracts verifiable version history and commit metadata from local Git repositories or GitHub (`owner/repo` or URL).

## 1. Question → Command Mapping

Always pass `-f json` when you need structured fields (`body`, `trailers`, `reverts`, `files`, `warnings`), or `-f brief --oldest-first` for a compact human-readable upgrade summary.

| Investigation Question | Command |
| --- | --- |
| **What changed between version A and B** (segmented by every intermediate tag, including hotfix branches, with cherry-picks deduplicated)? | `taglens <REPO> --range <A> <B> --oldest-first --max-commits 1000 -f json` |
| **What changed in a specific directory/module between A and B?** | `taglens <REPO> --range <A> <B> --path <DIR_OR_FILE> --files -f json` |
| **Which commits between A and B mention a keyword, error, or PR?** | `taglens <REPO> --range <A> <B> --grep "<KEYWORD>" --files -f json` |
| **Which release first included a commit / PR / fix, and was it backported to a hotfix branch?** | `taglens find "<SHA_OR_#PR_OR_KEYWORD>" <REPO> -f json` |
| **Which version is a deployed commit SHA on (exact tag, nearest ancestor tag + distance, earliest containing tag)?** | `taglens which <REV> <REPO> -f json` |
| **Which dependencies (`Cargo.lock`, `uv.lock`, `package-lock.json`, `go.sum`) changed between A and B?** | `taglens deps <A> <B> <REPO> -f json` |
| **Inspect a remote GitHub repository without cloning** | `taglens <owner/repo> --range <A> <B> -f json` |

## 2. Interpreting the JSON Fact Contract (`schema_version: 1`)

### Top-Level `Report`
- `warnings: string[]`: **Check this first.** Lists any data completeness limits (e.g. `--max-commits` truncation, GitHub Compare 250-commit cap, or lack of local cherry-pick deduplication in remote mode).
- `versions[].truncated: bool`: `true` when `commits.len() < total_commits`. Re-run with a larger `--max-commits` (e.g. `--max-commits 2000`) if you need full coverage.

### Per-Commit Evidence (`versions[].commits[]`)
- `category`: `"feature" | "fix" | "perf" | "refactor" | "docs" | "chore" | "other"`
- `classified_by`:
  - `"conventional"` — parsed from `type(scope)!: summary` (high confidence).
  - `"keyword"` — matched by subject first-word / keyword heuristics (medium confidence; verify with `summary`, `body`, or `files`).
  - `"none"` — fell back to `"other"`; always read `summary` / `body` / `files` rather than ignoring `"other"` commits.
- `breaking: bool`: `true` if `!` in prefix or `BREAKING CHANGE` in `trailers`.
- `body: string?`: full commit body (omitted when empty).
- `trailers: { key, value }[]`: extracted footers and issue links (`BREAKING CHANGE`, `Fixes`, `Closes`, `Refs`, `Co-authored-by`, `cherry-picked-from`).
- `reverts: string?` / `reverted_by: string?`: SHA cross-links when both a commit and its revert appear in the same version range. **Do not report a commit with `reverted_by` as an active change** without noting it was reverted.
- `files: { path, additions, deletions }[]`: populated when `--files` is passed (local repos only).

### `find` Output (`FindReport`)
- `matches[].first_tag`: earliest version in the primary tag series containing the commit (`null` if unreleased).
- `matches[].series[]`: `{ prefix, first_tag, tags[] }` for every tag prefix series (`v*`, `pkg@*`, etc.), sorted oldest-first.
- `matches[].backports[]`: cherry-picked copies on other branches/hotfix lines (matched via `cherry-picked-from` trailer or identical subject/category), each with its own `first_tag` and `series`.

### `which` Output (`WhichReport`)
- `exact_tags`: tags pointing directly at `rev`.
- `base_tag` + `ahead`: nearest ancestor tag and number of commits since that tag (`git describe --tags --long`).
- `first_tag` + `contained_in`: earliest tag that includes `rev` (`null` if `rev` is unreleased).

### `deps` Output (`DepsReport`)
- `lockfiles[]`: `{ file, added[], removed[], updated[] }` where each entry has `{ name, from[], to[] }`.

## 3. Recommended Investigation Workflow

1. **Locate version boundaries**: If the user gives a commit SHA, run `taglens which <SHA> <REPO> -f json`. If they ask when a fix/bug landed, run `taglens find "<query>" <REPO> -f json`.
2. **Inspect range + lockfiles**: For upgrade or regression analysis between `A` and `B`, run:
   - `taglens <REPO> --range <A> <B> --oldest-first --files --max-commits 1000 -f json`
   - `taglens deps <A> <B> <REPO> -f json`
3. **Check `warnings` and `reverted_by`**: Filter out or flag changes that were reverted within the same window, and check if any version was truncated.
4. **Drill into suspicious commits**: Once `taglens` narrows candidates to specific SHAs and file paths, run `git -C <REPO> show <SHA> -- <PATH>` to inspect the exact diff.
