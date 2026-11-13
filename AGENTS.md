# AGENTS.md

Guidance for AI coding agents (Claude Code, Codex, Cursor, Gemini, Antigravity …) working in this repository.
Human-facing docs live in [README.md](README.md); design rationale in [docs/design.md](docs/design.md).

## What this is

`taglens` is a Rust CLI. Given a local repo path or a GitHub URL/`owner/repo`, it lists, per tag/release,
the commits since the previous version and classifies them (breaking / feature / fix / perf / refactor /
docs / chore / other). Output: coloured text, `brief` upgrade list, Markdown, JSON.

## Layout

| File | Responsibility |
| --- | --- |
| `src/main.rs` | clap CLI, target dispatch (dir → local, else GitHub slug), `--range` parsing, output/file writing |
| `src/model.rs` | Plain data: `RawCommit`, `Commit`, `Category`, `Version`, `Release`, `Report`, `Selection` |
| `src/classify.rs` | `RawCommit` → `Commit`: Conventional Commits, then zh/en keyword heuristics |
| `src/local.rs` | Local collector via the system `git` binary; tag listing, previous-tag, `--range` splitting |
| `src/github.rs` | GitHub collector: `git ls-remote` for tags, REST for releases/compare; slug parsing; version ordering |
| `src/render.rs` | `text` / `brief` / `markdown` renderers and zh/en labels |
| `scripts/` | `check.sh` (quality gate), `build.sh`, `install.sh`, `release.sh` |

Modules talk only through `model.rs` data. There is deliberately **no trait** over the two collectors —
they fetch data too differently; don't add one until a third source exists.

## Commands

```bash
scripts/check.sh            # REQUIRED before every commit: fmt --check, clippy -D warnings, tests
cargo test <name>           # run a focused test while iterating
scripts/build.sh            # release build, prints binary path
scripts/install.sh          # cargo install --path . --locked --force
scripts/release.sh X.Y.Z    # on clean main: bump, check, commit, annotated tag vX.Y.Z (no push)
```

Sandboxed agents that cannot write to the global cargo target dir (`~/.cargo/config.toml` sets
`target-dir`) should prefix commands with `CARGO_TARGET_DIR=target` (gitignored).

## Code rules

- **2-space indentation** everywhere (`rustfmt.toml` sets `tab_spaces = 2`); run `cargo fmt`.
- **No `unwrap()` / `expect()` outside `#[cfg(test)]`.** Propagate with `?` + `anyhow::Context`;
  use `unreachable!("why")` only for states the types/clap already rule out.
- User-facing messages (errors, help) are **Chinese**; code, comments and identifiers are English.
- Comments explain *why*, not *what*.
- Keep functions returning values rather than mutating caller state; keep items private unless needed.
- Prefer the smallest change that answers the real problem; no speculative config flags, builders or layers.

## Testing rules

- Write the failing test first, confirm it fails for the right reason, then implement.
- Local-mode tests build real throwaway repos with `TempRepo` in `src/local.rs` (no mocks).
  Use `commit_file` (real diff) when patch identity matters — `--allow-empty` commits share an empty patch,
  and a same-second `cherry-pick` without `-x` can reproduce the original SHA.
- GitHub-mode logic is tested through pure functions (`parse_slug`, `group_tags`, `split_range`,
  `compare_versions`). Network calls are verified manually, e.g. `taglens BurntSushi/ripgrep -n 2`.
- If you add an HTTP test against a local server, build the client with `reqwest::Client::builder().no_proxy()`
  — dev machines often have a system proxy that black-holes loopback requests.

## Behaviour that is easy to break

- **Previous tag (local)**: `git describe --tags --abbrev=0 --match '<prefix>[0-9]*' <tag>^`, so monorepo
  packages (`react@…`, `sdk/go/v…`) never interleave. `tag_prefix` = text before the first digit.
- **`--range A B`**: split at every same-prefix tag whose *version* lies between A and B (hotfix-branch tags
  included); non-tag endpoints fall back to ancestry (`git tag --merged/--no-merged`). Each segment uses
  `A...B --cherry-pick --right-only`, which drops back-ported commits and equals `A..B` on linear history.
- **Version order**: numeric segments compare numerically; a suffix (`-rc.1`) sorts *before* the bare release.
- **Breaking commits** are shown once, in the leading breaking group, not again in their category.
- **GitHub API budget**: anonymous 60 req/h. Tags come from `git ls-remote` (free); one releases call plus one
  compare per version. Don't add per-commit API calls.
- Classification categories are declared in display order in `Category`; labels live in `render.rs`
  (`ZH`/`EN`, same order). Adding a category means touching both plus `Category::ALL`.

## Git workflow

- Branch from `main`: `feat/<topic>` or `fix/<topic>`; never commit features directly on `main`.
- One logical change per commit. Message format — English Conventional Commit header, blank line,
  Chinese bullet list:

  ```text
  feat(range): split ranges by version order

  - 按版本号选取区间内同前缀的 Tag
  - 补充 hotfix 分支场景测试
  ```

- Never force-push, amend published commits, or skip hooks without explicit permission.
- Update `README.md` / `docs/design.md` in the same commit when behaviour or flags change.
