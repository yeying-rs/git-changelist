# taglens

按 Tag / Release 查看仓库每个版本相对上一版本带来了哪些改变，并把提交自动归类为
**破坏性变更 / 新增功能 / 问题修复 / 性能优化 / 重构 / 文档 / 维护 / 其他**，
帮助你在升级前快速了解「从 A 升到 B 会发生什么」。

- 支持**本地仓库路径**和 **GitHub 地址**（`https://…`、`git@…`、`owner/repo`）
- `--range A B` 按中间每个版本分段，每条变化标注到引入它的 Tag 上
- 识别 Conventional Commits，也能处理自由格式与中文提交信息
- 输出终端彩色文本、精简升级清单（brief）、Markdown 或 JSON，可写入文件
- 适配 monorepo（`react@0.1.0`、`tokio-macros-2.x` 等多前缀 Tag）与 hotfix 分支

## 安装

需要 Rust 1.88+（edition 2024）和系统已安装的 `git`。

```bash
# 从 GitHub 安装
cargo install --git https://github.com/yuxuetr/changelog --locked

# 或者克隆后本地安装
git clone https://github.com/yuxuetr/changelog && cd changelog
scripts/install.sh
```

## 快速开始

```bash
taglens                                            # 当前仓库，最近 5 个版本
taglens /path/to/repo -n 10 --unreleased           # 最近 10 个版本 + 尚未发布的提交
taglens BurntSushi/ripgrep                         # 直接查看 GitHub 仓库
taglens . --range v0.5.1.1 v0.5.4.1                # 两个版本之间，按版本分段
```

### 看升级带来的改变（推荐）

```bash
taglens . --range v0.5.1.1 v0.5.4.1 -f brief --oldest-first --max-commits 1000
```

```text
v0.5.2（v0.5.1.1 → v0.5.2，67 个提交）
  新增功能: [plugins] redact secrets in captured transcripts before sending (#6455)
  新增功能: [privacy] support automatic privacy extraction and restore for sessions (#6278)
  ...
v0.5.2.1（v0.5.2 → v0.5.2.1，6 个提交）
  新增功能: [connector] backport DingTalk OAuth auth to v0.5.2
  问题修复: [deps] require SDK 0.1.17 for Compile APIs (#6596)
  ...
v0.5.4.1（v0.5.4 → v0.5.4.1，46 个提交）
  破坏性变更: [ui] add reusable Wiki and Agent experience dashboards (#7379)
  ...
```

每段列出的是**升级到该版本**带来的变化；起点版本（这里的 v0.5.1.1）本身不单独成段。

### 导出

```bash
taglens . --range v1.0.0 v1.3.0 -f markdown -o CHANGES.md   # 带 compare / PR / commit 链接
taglens . --range v1.0.0 v1.3.0 -f json -o changes.json     # 供脚本处理
taglens owner/repo -f brief --lang en -o UPGRADE.txt        # 英文分类名
```

## 参数

| 参数 | 说明 |
| --- | --- |
| `TARGET` | 本地仓库路径（默认 `.`）或 GitHub 地址 |
| `-n, --limit N` | 最近 N 个版本（默认 5） |
| `--tag T` | 只看某个 Tag |
| `--range A..B` / `--range A B` | 只看 A 到 B 之间的变化，按中间的版本分段 |
| `--unreleased` | 额外展示最新 Tag 之后尚未发布的提交 |
| `--github` | 本地模式下，按 `origin` 拉取 GitHub Release Notes 合并展示 |
| `--token` | GitHub Token，默认读取环境变量 `GITHUB_TOKEN` |
| `--max-commits N` | 每个版本最多展示的提交数（默认 200） |
| `--merges` | 保留合并提交（默认过滤） |
| `-f, --format` | `text`（默认）/ `brief` / `markdown` / `json` |
| `--oldest-first` | 版本从旧到新排列（默认从新到旧） |
| `-o, --output` | 写入文件而不是标准输出 |
| `--lang` | `zh`（默认）/ `en` |

## 工作原理

### 分类规则

1. **Conventional Commits**：`feat` → 新增功能，`fix` → 问题修复，`perf` → 性能优化，`refactor` → 重构，
   `docs` → 文档，`test/build/ci/chore/style/deps/revert/release` → 维护；
   `type!:` 或正文含 `BREAKING CHANGE:` 标记为破坏性变更，并置顶展示。
2. **未知前缀视为 scope**：Go 风格的 `net/http: fix ...` 会按 `fix ...` 继续判断。
3. **启发式**：按首词（`Add` / `Support` / `Fix` / `Resolve` / `新增` / `支持` / `修复` / `解决` …）
   与关键词（`bug` / `crash` / `panic` / `崩溃` …）归类；只有版本号的发布提交归为维护。
4. 判断不了的归入「其他」，不做猜测。
5. 标题末尾的 `(#123)` 会提取为 PR 编号。

### 版本与区间

- **上一个版本**：本地模式用 `git describe --tags --match '<前缀>[0-9]*'` 找同前缀、拓扑上最近的 Tag；
  GitHub 模式按版本号排序（`v1.10.0` > `v1.9.0`，预发布版排在正式版之前）。
- **`--range A B` 分段**：按版本号取 A、B 之间同前缀的所有 Tag（包括 hotfix 分支上的 Tag）切开。
  本地模式每段使用 `git log --cherry-pick --right-only A...B`，已经在上一版本中回合（cherry-pick）
  过的提交不会重复出现。
- **GitHub 模式**：Tag 列表来自 `git ls-remote`（不占 API 配额），Release 说明和区间提交来自 REST API。

## 限制

- GitHub 匿名 API 额度为 60 次/小时，每个版本消耗 1 次（另加 1 次 Releases）；
  设置 `GITHUB_TOKEN` 可提升到 5000 次/小时，并支持私有仓库。
- GitHub 单个区间最多返回 250 个提交，超出时标注「仅显示 N / 共 M」。
- GitHub 模式下最早的版本没有可比较的前序 Tag，只展示 Release 说明。
- GitHub 模式拿不到分支历史，`--cherry-pick` 去重只在本地模式生效；分析有 hotfix 分支的仓库建议用本地路径。
- monorepo 中默认列表按 Tag 数最多的前缀排在前面，可用 `--tag` / `--range` 精确指定。

## 开发

```bash
scripts/check.sh          # fmt --check + clippy -D warnings + test，提交前必跑
scripts/build.sh          # release 构建并输出二进制路径
scripts/install.sh        # 安装到 ~/.cargo/bin
scripts/release.sh 0.2.0  # 更新版本号、检查、提交，并创建附带变更摘要的 v0.2.0 Tag（不自动 push）
```

- 设计文档：[docs/design.md](docs/design.md)
- AI 编码助手须知：[AGENTS.md](AGENTS.md)
