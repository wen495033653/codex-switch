# Contributing

感谢你参与 Codex Switch。这个项目是 Tauri 2 + React + Rust 桌面应用。贡献时建议尽量保持改动聚焦；提交前也请留意不要带入账号 token、API Key 或本机配置文件。

## 环境要求

- Node.js 22 或更高版本。
- npm，使用仓库内 `package-lock.json`。
- Rust stable toolchain。
- Windows：WebView2 Runtime，以及带 MSVC linker 的 Visual Studio Build Tools 或 Visual Studio。
- macOS：Xcode Command Line Tools。

## 初始化

```powershell
npm ci
```

开发模式：

```powershell
npm run dev
```

只调试 renderer：

```powershell
npm run dev:renderer
```

`dev:renderer` 不会连接 Tauri command；页面会保留基础 UI，桌面操作会提示 Tauri API 未加载。

## 验证

提交 PR 前，可以按改动范围选择合适的基础检查：

```powershell
npm run check
npm run check:tauri
```

如果修改 Rust / Tauri 代码，建议在 `src-tauri/` 目录运行：

```powershell
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

`npm run check` 会执行 JavaScript 语法检查、开源敏感信息基础扫描，并构建 React renderer。`npm run check:tauri` 会对 Rust/Tauri 侧执行 `cargo check`，并把 Cargo target 放到系统临时目录，避免在仓库内生成大型构建产物。`cargo fmt`、`cargo test` 与 `cargo clippy` 需要在 `src-tauri/` 目录运行。

如果修改安装包、updater、Tauri config 或平台集成逻辑，可以视本地环境运行：

```powershell
npm run dist
```

如果本地环境不具备完整验证条件，可以在 PR 描述中说明已运行的命令和未运行的原因。当前仓库没有单独的 unit test script；单独的 `npm run build:renderer` 只覆盖 renderer 构建，不建议把它当作完整验证。

## 本机文件与会话数据

- 修改 Codex 会话文件、`session_index.jsonl`、`state_5.sqlite` 或 `.codex-global-state.json` 的代码必须先拿到 `codex_sessions::lock_codex_session_io(...)`，避免和后台会话同步互相覆盖。
- 删除、导入、恢复、归档、移回进行中、修改工作目录这类会改变会话状态或路径的操作，都属于需要加锁的范围。
- 后台会话同步写回 rollout 文件时，应只写已存在的文件；不要用会重新创建目标路径的写法复活刚被删除或移动的会话。

## 分支与 PR

- 推荐流程：fork 本仓库 -> 在 fork 内从最新 `main` 创建贡献分支 -> 提交 PR。
- 分支命名可以参考：新功能 `feature/<slug>`，修复 `fix/<slug>`，维护、文档、构建或流程调整 `chore/<slug>`。
- PR 尽量聚焦在一个清晰问题上；小修小改不必过度拆分，方便 review 即可。
- PR 描述建议包含改动摘要、已运行的验证命令和影响范围。
- UI 改动如方便请附截图或录屏。
- 修改账号、OAuth、API 模式、updater 或本机文件写入逻辑时，建议说明隐私与回滚影响。
- 尽量避免把无关格式化、重命名或大范围重排混入功能修复。

## Issue

提交 bug 时，建议包含：

- 操作系统和版本。
- Codex Switch 版本。
- 复现步骤。
- 期望结果和实际结果。
- 相关日志或截图。

提交 PR 或 Issue 前，请留意截图、日志和说明中是否包含 token、refresh token、API Key、邮箱、账号 ID、本机用户名、本机路径等敏感信息，尽量先删除或打码。
