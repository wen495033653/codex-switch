# Contributing

感谢你参与 Codex Switch。这个项目是 Tauri 2 + React + Rust 桌面应用，贡献时请保持改动聚焦，并避免提交任何账号 token、API Key 或本机配置文件。

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

提交 PR 前至少运行：

```powershell
npm run check
npm run check:tauri
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

`npm run check` 会执行 JavaScript 语法检查、开源敏感信息基础扫描，并构建 React renderer。`npm run check:tauri` 会对 Rust/Tauri 侧执行 `cargo check`，并把 Cargo target 放到系统临时目录，避免在仓库内生成大型构建产物。`cargo fmt`、`cargo test` 与 `cargo clippy` 需要在 `src-tauri/` 目录运行。

如果修改安装包、updater、Tauri config 或平台集成逻辑，还应在目标平台运行：

```powershell
npm run dist
```

当前仓库没有单独的 unit test script。不要把单独的 `npm run build:renderer` 当作完整验证；它只覆盖 renderer 构建。

## 分支与 PR

- 一个 PR 只解决一个清晰问题。
- PR 描述应包含改动摘要、验证命令和影响范围。
- UI 改动请附截图或录屏。
- 修改账号、OAuth、API 模式、updater 或本机文件写入逻辑时，请说明隐私与回滚影响。
- 不要把无关格式化、重命名或大范围重排混入功能修复。

## Issue

提交 bug 时请包含：

- 操作系统和版本。
- Codex Switch 版本。
- 复现步骤。
- 期望结果和实际结果。
- 相关日志或截图。

请先删除 token、refresh token、API Key、邮箱、账号 ID、本机用户名、本机路径等敏感信息。

## Release

Release 由 `.github/workflows/release.yml` 处理，推送 `vX.Y.Z` tag 或手动运行 Workflow 会构建 Windows / macOS 安装包并发布 GitHub Release。版本号保持三位格式，例如 `4.9.1`。

发布 updater artifact 前，仓库需要配置：

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

私钥和密码不要提交到仓库。fork 后发布自己的安装包时，请使用自己的 updater key，并同步修改 updater endpoint 与 public key。
