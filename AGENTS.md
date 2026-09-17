# AGENTS.md

- 动手前先读并遵守 `CONTRIBUTING.md`；其中的项目级规则优先于全局 `git-flow` skill。
- 本文件是维护者的流程：本地开发、合并外部 PR、发版。对外贡献规则在 `CONTRIBUTING.md`，文档索引在 `docs/README.md`。
- 新增或移动代码前，先读 `docs/development/module-structure.md`：模块分层、依赖方向与导入约定。

## 维护者开发流程

- 自己的改动默认不走 PR：从干净的最新 `main` 新开分支 → 本地开发并验证 → rebase 最新 `origin/main` → squash merge 到 `main` → push。
- 分支命名：新功能 `feature/<slug>`，修复 `fix/<slug>`，文档、流程、构建 `chore/<slug>`，验证用 `validate/<slug>`。
- 一个分支只解决一个清晰的问题，不同功能必须拆到不同分支；`main` 上一个功能对应一个干净的 commit。
- 分支做完后提交并 push 到 `origin/<branch>`，再切回 `main`。squash 合入并 push 之后，先确认 `main` 已包含改动，再删除本地分支和对应的远端分支。
- 验证完整功能时，从最新 `main` 新开验证分支，把待验的 feature 合进去，push 后让用户确认；确认前不要发版。
- `main` 的 push 只触发 CI，不发布 Release。正式发布只能通过 `vX.Y.Z` / `vX.Y.Z-rc.N` tag 或手动 Release Workflow。

## 合并外部 PR

- 外部贡献者的 PR 单独合并到 `main`。不要先合进我的 feature 分支或一个总 PR 再整体 squash，那样会破坏贡献归属，冲突也更难处理。需要显示贡献者时，commit message 保留或补充 `Co-authored-by`。
- 是否 squash 按用户的明确要求：要 squash 就用 `Squash and merge`，不要 squash 就用普通 merge commit。
- 外部 PR 合并后，我自己的 feature 分支先 `fetch`，再 rebase 最新 `origin/main`；冲突必须在 feature 分支内解决，再继续合并或开 PR。

## 发布说明文案

- 面向用户的 release notes、updater notes 和下载页更新说明，不要直接写 GPT Pool、公益站点、API Key 自动配置、广告入口、推广远程开关等内嵌推广细节。
- 这类改动属于运营和推广入口的调整，对用户只用泛化文案，例如 `API 模式配置体验优化`、`提示入口展示改进`、`界面体验优化`。
- commit message、内部上下文和实现说明里可以保留准确的技术名词。

## Tag 发布流程

- 用户要求打 tag、发版、发布 Release 或准备新版本时，必须先给出发布说明文件的 demo 版并等待用户确认。确认前禁止创建正式的 release notes 文件、禁止 commit 发布说明、禁止打 tag。
- demo 用目标 tag 号作标题，内容就是将要写入 `.github/release-notes/<tag>.md` 的内容。
- 用户确认后，才把 demo 内容写入正式文件；写入后检查文件路径、内容和编码，再按本仓库 Git 流程 commit / push。
- 只有同时满足以下条件才允许创建并 push tag：`.github/release-notes/<tag>.md` 已存在（打 tag 前用本地检查确认，不要用口头说明代替）、内容已由用户确认、对应 commit 已在目标发布分支上。
- push tag 前再次检查目标 tag 是否已存在。如果 tag 已存在，或曾触发失败的 Release Workflow，禁止擅自移动、删除、重建或覆盖 tag，必须先向用户确认处理方式。
- `.github/release-notes/` 只保留最新一个版本的说明文件：写入新版本的文件时，删除上一版的文件。历史说明保存在各自的 GitHub Release 和 git 历史里，发布流程只读取当前 tag 对应的文件。
- 修改已发布版本的用户可见更新说明时，必须同时更新 GitHub Release body 和 `latest.json` asset 的 `notes` 字段，并回读两处确认一致。
