# AGENTS.md

- 开始修改本项目代码、文档、Git branch、PR、release 或验证流程前，先读取并遵守 `CONTRIBUTING.md`。
- `CONTRIBUTING.md` 中的项目级规则优先于全局 `git-flow` skill。
- 本文件记录维护者本地开发、合并外部 PR、以及把外部 PR 和自己的 feature 分支做整合的流程；对外贡献规则写在 `CONTRIBUTING.md`。

## Codex Switch 维护者流程

- 自己开发默认不走 PR：本地开发 -> 自己验证 -> rebase 最新 `origin/main` -> squash merge 到 `main` -> push。
- 自己开发仍使用本地分支承载改动；常用分支命名：新功能 `feature/<slug>`，修复 `fix/<slug>`，文档/流程/构建 `chore/<slug>`；验证分支可用 `validate/<slug>`。
- 每个 `feature/fix/chore` 分支只承载一个功能、修复或流程改动；不同功能必须拆到不同分支，不要混在同一个 feature 里。
- 一个分支做完后提交并 push 到 `origin/<branch>`，然后切回 `main`。
- 开始新功能、修复或流程改动时，先确认当前在干净的 `main`，再从 `main` 新开对应的 `feature/fix/chore` 分支。
- 一个变更单元只解决一个清晰问题。
- 自己的本地分支最终默认用 squash 方式合入 `main`，让 `main` 上一个功能对应一个干净 commit。
- 自己的 `feature/fix/chore` 分支 squash merge 到 `main` 并 push 后，及时删除已合并的本地分支；如果分支曾 push 到远端，也删除对应远端分支。删除前先确认 `main` 已包含 squash 后的改动。
- `main` push 只触发 CI 检查，不发布 Release。
- 外部贡献者 PR 优先单独合并到 `main`，不要先合进我的总 feature PR 再 squash；需要贡献者显示时，commit message 保留或补充 `Co-authored-by`。
- 外部 PR 是否 squash 按用户明确要求执行：用户要求 squash 就用 `Squash and merge`；用户要求不要 squash 就用普通 merge commit。
- 外部 PR 合并后，我自己的 feature 分支先 `fetch`，再基于最新 `origin/main` rebase；冲突必须在 feature 分支内解决，再继续合并或开 PR。
- 不再使用“把所有外部 PR 和我的 feature 混进一个总 PR 后整体 squash”的流程；这会让贡献归属和冲突处理变差。
- 验证完整功能时，从最新 `main` 新开验证分支，把待验 feature 合进去，push 后让用户确认；确认前不要发版。
- 正式发布只能通过 `vX.Y.Z` / `vX.Y.Z-rc.N` tag 或手动 Release Workflow。

## 发布说明文案

- 面向用户的 release notes、updater notes 和下载页更新说明不要直接写 GPT Pool、公益站点、API Key 自动配置、广告入口、推广远程开关等内嵌推广细节。
- 这类改动属于运营/推广入口调整，对用户只用泛化文案描述为 `API 模式配置体验优化`、`提示入口展示改进`、`界面体验优化` 等。
- 可以在 commit message、内部上下文或实现说明中保留准确技术名词，但不要把这些推广细节放进用户会看到的版本更新说明。

## Tag 发布流程

- 用户要求打 tag、发版、发布 Release 或准备新版本时，必须先给出发布说明文件 demo 版，等待用户确认；确认前禁止创建正式 release notes 文件、禁止 commit 发布说明、禁止打 tag。
- 发布说明 demo 必须使用目标 tag 号作为标题，并按仓库约定的 release notes 文件内容格式展示；本仓库使用 `.github/release-notes/<tag>.md`，demo 内容必须对应这个文件。
- 用户确认 demo 后，才允许把 demo 内容写入正式发布说明文件；写入后必须检查文件路径、内容和编码，再按本仓库 Git 流程 commit / push。
- 只有正式发布说明文件已经存在、内容已经由用户确认、且对应 commit 已在目标发布分支上，才允许创建并 push tag。
- push tag 前必须再次检查目标 tag 是否已存在；如果 tag 已存在或曾触发失败的 Release Workflow，禁止擅自移动、删除、重建或覆盖 tag，必须先向用户确认处理方式。
- 打 tag 前必须用本地检查确认 `.github/release-notes/<tag>.md` 已存在；不要用口头发布说明代替仓库里的正式文件。
- 修改已发布版本的用户可见更新说明时，必须同时更新 GitHub Release body 和 `latest.json` asset 的 `notes` 字段，并回读两处内容确认一致。
