# 参与贡献

感谢参与 Codex Switch。这是一个 Tauri 2 + React + Rust 的桌面应用，欢迎任何大小的改进，包括修错别字。

## 快速开始

需要：Node.js 22.12 或更高版本、Rust stable。Windows 还需要 WebView2 Runtime 和带 MSVC 的 Visual Studio Build Tools；macOS 需要 Xcode Command Line Tools。

```bash
npm ci
npm run dev
```

只改界面时可以用 `npm run dev:renderer`，它只启动前端，不连接桌面后端，界面里的桌面操作会提示未加载。

## 项目结构

| 目录 | 内容 |
| --- | --- |
| `renderer/src/` | React 前端：`components/` 页面与组件，`hooks/` 状态与操作，`utils/` 纯函数，`desktopApi.js` 是前端能调用的全部后端命令 |
| `src-tauri/src/` | Rust 后端，命令在 `main.rs` 里注册 |
| `scripts/` | 检查脚本和前端测试（`test-*.mjs`） |
| `resources/`、`build/` | 随应用打包的资源、应用图标 |
| `docs/` | 开发文档，从 [docs/README.md](./docs/README.md) 看起 |

改动较大时，先看一眼 [模块结构与依赖约定](./docs/development/module-structure.md)。

## 提交前检查

```bash
npm run check
```

它会做语法和敏感信息检查、跑前端测试并构建前端。改了 Rust 代码时，再在 `src-tauri/` 下运行：

```bash
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

CI 会跑同样的检查，Rust 部分在 Windows 和 macOS 上各跑一遍。本地跑不了某一项也没关系，在 PR 里说明即可。改了安装包、自动更新或 Tauri 配置时，有条件可以用 `npm run dist` 试一次完整打包。

## 一条硬性规则：改会话文件要先加锁

读写 Codex 会话数据（会话文件、`session_index.jsonl`、`state_5.sqlite`、`.codex-global-state.json`）的代码，必须先拿到 `codex_sessions::lock_codex_session_io(...)`，包括导入、归档、取消归档。否则会和后台的会话同步互相覆盖。回写会话文件时只写已存在的文件，不要把刚被删除或移动的会话重新创建出来。

## 提 PR

1. fork 仓库，从最新的 `main` 建分支（命名随意，例如 `fix/xxx`、`feature/xxx`）。
2. 一个 PR 解决一个问题，不要混入无关的格式化或重命名。
3. 在 PR 里写清楚改了什么、怎么验证的；界面改动最好附截图。
4. 涉及账号登录、API 配置、会话数据、自动更新或写本机文件时，顺带说明影响和如何回退。

## 提 Issue

用仓库里的 Issue 模板即可。Bug 请写明系统、Codex Switch 版本、复现步骤、期望结果和实际结果。

## 注意隐私

提交代码、截图和日志前，请去掉 token、`refresh_token`、API Key、邮箱、账号 ID、本机用户名和本机路径。`npm run check` 会拦截常见的几类，但不能代替人工检查。
