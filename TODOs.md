# TODOs

| # | 任务 | 状态 |
| --- | --- | --- |
| 1 | 设计文档、TODO 清单、Cargo 工程初始化 | DONE（docs/design.md、依赖锁定） |
| 2 | 提交分类器 `classify.rs`（Conventional Commits + 中英文启发式）及单元测试 | DONE（7 个单元测试） |
| 3 | 本地模式 `local.rs` + 文本渲染 + CLI 主流程，临时 git 仓库集成测试 | DONE（6 个临时仓库集成测试；中英文标签提前在此实现） |
| 4 | GitHub 模式 `github.rs`（Tags/Releases/Compare、版本排序、限流提示）+ 本地 `--github` 合并 | DONE（Tag 列表改用 git ls-remote；ripgrep/tokio 实测；修复本地 monorepo 前序 Tag） |
| 5 | Markdown / JSON 输出、`--lang en`、`-o` 写文件 | DONE（Markdown 含 compare/PR/commit 链接） |
| 6 | README、release 构建与 `cargo install` 安装验证 | DONE（release 7.4M，已安装到 ~/.cargo/bin） |
| 7 | 更新设计文档 `docs/design.md` 与 `TODOs.md`（v0.2 面向 AI 的版本事实契约与排查子命令） | DONE（已补充 JSON 事实契约、find/which/deps 与 Skill 规划） |
| 8 | 完善 JSON 事实契约：`schema_version`、`body`、`trailers`、`classified_by`、`reverts`/`reverted_by`、`truncated`、`warnings[]` | DONE（32 个单元测试通过，含 trailer/revert/truncation 校验） |
| 9 | 提交改动文件统计 `--files`（`--numstat`）及过滤参数 `--path`、`--grep` | DONE（单次 git log --numstat 采集文件增删统计，支持路径与关键词过滤） |
| 10 | 版本排查子命令 `taglens find <QUERY>` 与 `taglens which <REV>` | DONE（支持最早 Tag、各前缀系列与 hotfix cherry-pick 回移识别） |
| 11 | 依赖锁文件变更对比子命令 `taglens deps <FROM> <TO>`（`Cargo.lock`、`uv.lock`、`package-lock.json`、`go.sum`） | DONE（支持 4 类锁文件新增/移除/升级对比，无新增外部依赖） |
| 12 | 编写 `skills/taglens/SKILL.md`，更新 `scripts/install.sh`、`README.md` 与 `AGENTS.md` | DONE（新增 SKILL.md 与 --skill 安装支持，同步更新文档） |
