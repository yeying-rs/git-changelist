# TODOs

| # | 任务 | 状态 |
| --- | --- | --- |
| 1 | 设计文档、TODO 清单、Cargo 工程初始化 | DONE（docs/design.md、依赖锁定） |
| 2 | 提交分类器 `classify.rs`（Conventional Commits + 中英文启发式）及单元测试 | DONE（7 个单元测试） |
| 3 | 本地模式 `local.rs` + 文本渲染 + CLI 主流程，临时 git 仓库集成测试 | DONE（6 个临时仓库集成测试；中英文标签提前在此实现） |
| 4 | GitHub 模式 `github.rs`（Tags/Releases/Compare、版本排序、限流提示）+ 本地 `--github` 合并 | DONE（Tag 列表改用 git ls-remote；ripgrep/tokio 实测；修复本地 monorepo 前序 Tag） |
| 5 | Markdown / JSON 输出、`--lang en`、`-o` 写文件 | TODO |
| 6 | README、release 构建与 `cargo install` 安装验证 | TODO |
