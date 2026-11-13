# taglens

按 Tag / Release 查看仓库每个版本相对上一版本的提交，并自动归类为
**新增功能 / 问题修复 / 破坏性变更 / 性能优化 / 重构 / 文档 / 维护 / 其他**。

支持本地仓库路径和 GitHub 地址，输出终端彩色文本、Markdown 或 JSON。

## 安装

```bash
cargo install --path .
```

需要系统已安装 `git`。

## 用法

```bash
# 本地仓库（默认当前目录，最近 5 个版本）
taglens
taglens /path/to/repo -n 10 --unreleased

# 本地仓库 + 合并 GitHub Release Notes（origin 需指向 GitHub）
taglens /path/to/repo --github

# GitHub 仓库：URL、SSH 地址或 owner/repo
taglens https://github.com/BurntSushi/ripgrep
taglens tokio-rs/tokio --tag tokio-1.54.1
taglens owner/repo --range v1.0.0..v1.2.0   # 或 --range v1.0.0 v1.2.0

# 导出
taglens owner/repo -f markdown -o RELEASES.md
taglens . -f json --lang en
```

| 参数 | 说明 |
| --- | --- |
| `-n, --limit N` | 最近 N 个版本（默认 5） |
| `--tag T` / `--range A..B`（或 `--range A B`） | 只看某个 Tag / 某个区间 |
| `--unreleased` | 额外展示最新 Tag 之后尚未发布的提交 |
| `--github` | 本地模式下合并 GitHub Release Notes |
| `--token` | GitHub Token，默认读取 `GITHUB_TOKEN` |
| `--max-commits N` | 每个版本最多展示的提交数（默认 200） |
| `--merges` | 保留合并提交（默认过滤） |
| `-f, --format` | `text` / `markdown` / `json` |
| `-o, --output` | 写入文件 |
| `--lang` | `zh` / `en` |

## 分类规则

1. **Conventional Commits**：`feat` / `fix` / `perf` / `refactor` / `docs` / `chore` 等；
   `!` 或 `BREAKING CHANGE:` 标记为破坏性变更。
2. **启发式**：未遵循规范时，按首词（`Add` / `Fix` / `新增` / `修复` …）与关键词（`bug` / `crash` …）归类。
3. 无法判断的归入「其他」，不猜测。

## 限制

- GitHub 匿名 API 额度为 60 次/小时，每个版本消耗 1 次（另加 1 次 Releases）；设置 `GITHUB_TOKEN` 可提升到 5000 次/小时。
- GitHub 单个区间最多返回 250 个提交，超出时会标注「仅显示 N / 共 M」。
- GitHub 模式下，最早的版本没有可比较的前序 Tag，不展示提交明细。
- monorepo 中按 Tag 前缀分组（如 `tokio-` 与 `tokio-macros-`），Tag 数最多的包排在前面；可用 `--tag` 精确指定。

设计细节见 [docs/design.md](docs/design.md)。
