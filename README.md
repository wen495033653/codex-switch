# Codex Switch

[English](./README.en.md) | [Releases](https://github.com/wen495033653/codex-switch/releases)

Codex Switch 是一个本地桌面工具，用来切换 Codex 订阅账号、应用 OpenAI-compatible API 配置，并通过指定代理启动 Codex。

## 功能

- 管理多个 Codex 订阅账号。
- 通过 OAuth、`refresh_token` 或 JSON 文件导入账号。
- 在订阅账号和 API 模式之间切换。
- 保存 API Base URL 和 API Key，并自动规范化常见 Base URL 输入。
- 同步订阅/API 模式的本地会话列表。
- 通过 Codex Switch 启动 Codex 时注入 `HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY`、`WS_PROXY`、`WSS_PROXY`。
- 创建 Codex 桌面图标；代理地址为空时创建普通 Codex 图标，代理地址不为空时创建代理启动图标。
- 从 GitHub Releases 检查、下载并安装更新。

## 安装

从 [GitHub Releases](https://github.com/wen495033653/codex-switch/releases) 下载对应平台安装包：

- Windows：x64 `.exe`
- macOS Apple Silicon：`aarch64` `.dmg`
- macOS Intel：`x64` `.dmg`

普通用户不需要手动下载 `.sig` 文件，它只给应用内 updater 校验安装包使用。

## 本地文件

Codex Switch 会读写 Codex 本身使用的本地文件：

- `~/.codex/auth.json`
- `~/.codex/config.toml`
- `~/.codex/sessions/**/rollout-*.jsonl`
- `~/.codex/state_5.sqlite`

账号数据和应用设置保存在应用数据目录：

- Windows：`%APPDATA%/codex-switch/`
- macOS：`~/Library/Application Support/codex-switch/`

导出的账号 JSON 包含 `refresh_token`，请按 credential 处理。

## 开发

```bash
npm ci
npm run dev
```

常用检查：

```bash
npm run check
npm run check:tauri

cd src-tauri
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

构建安装包：

```bash
npm run dist
```

构建产物位于 `src-tauri/target/release/bundle/`。

## 发布

推送 `vX.Y.Z` 或 prerelease tag 后，GitHub Actions 会构建 Windows 和 macOS 安装包。

updater 签名需要配置 GitHub repository secrets：

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

私钥不能提交到仓库。

## License

MIT. See [LICENSE](./LICENSE).
